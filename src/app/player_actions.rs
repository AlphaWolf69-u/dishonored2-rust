//! bevy_backend: demanded player equipment preparation on the existing IO
//! pool. The live clip banks keep animating; workers acquire only missing
//! compressed resources and publish stable indices after source completion.
use super::interaction_focus::{DiscoveredDocuments, ShortcutSelection, ShortcutSettings};
use crate::{
    assets::{
        animation_rig::{Clips, Rig},
        archive::Catalog,
        equipment::{Descriptor, Sources},
        inventory_item::PickableItem,
        wwise::SoundEventDeclaration,
    },
    gameplay::player::{
        self, Player,
        arms::{EquipmentDefinition, PreparedEquipment},
        equipment::EquipmentTree,
        midnight::{Midnight, PreparedScene},
        states::Scenes,
    },
};
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Snapshot {
    rig: Arc<Rig>,
    model: String,
    aliases: HashMap<String, usize>,
    base: usize,
}

struct Bank {
    model: String,
    base: usize,
    animations: Vec<crate::edge_animation::EdgeAnimation>,
    aliases: HashMap<String, usize>,
}

struct Pending {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<ReadyEquipment, String>>>,
}

struct PendingPower {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<ReadyPower, String>>>,
}

struct PendingMetadata {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<Arc<crate::assets::equipment::Metadata>, String>>>,
    complete: Option<Result<Arc<crate::assets::equipment::Metadata>, String>>,
    admit: bool,
}

struct PendingOwned {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<Arc<Descriptor>, String>>>,
}

pub(super) struct ReadyPower {
    pub item: Arc<PickableItem>,
    pub equipment: player::weapons::Equipment,
    pub id: i32,
    side: usize,
    arm: Option<PreparedEquipment>,
    banks: Vec<Option<Bank>>,
    settings: crate::gameplay::powers::Settings,
    blink: Option<super::blink::Prepared>,
    far_reach: Option<super::far_reach::Prepared>,
    scenes: Scenes,
    family: Option<PreparedFamily>,
    effect_paths: Vec<String>,
    reservation_key: Arc<str>,
    reservation: Option<Arc<str>>,
}

enum PreparedFamily {
    BendTime(super::power_world::PreparedSources),
    Windblast(super::windblast::PreparedSources),
}

struct PowerActivation {
    id: i32,
    blink: Option<super::blink::Prepared>,
    far_reach: Option<super::far_reach::Prepared>,
    equipment: Arc<player::weapons::Equipment>,
}

struct PowerDependencyRetirement {
    id: i32,
    reservation: Arc<str>,
    sources_released: bool,
}

impl ReadyPower {
    fn publish_dependencies(&mut self, world: &mut World) -> Result<bool, String> {
        if let Some(family) = self.family.take() {
            match family {
                PreparedFamily::BendTime(sources) => world.get_resource_mut::<super::power_world::PowerWorld>()
                    .ok_or("owned Bend Time has no canonical world owner")?.absorb(sources)?,
                PreparedFamily::Windblast(sources) => world.get_resource_mut::<super::windblast::Windblasts>()
                    .ok_or("owned Windblast has no canonical world owner")?.absorb(sources)?,
            }
        }
        if self.effect_paths.is_empty() && self.reservation.is_none() { return Ok(true); }
        if self.reservation.is_none() {
            // Host producer identity only. Canonical FX acquisition, material
            // preparation and actual playing instances remain in that owner.
            let key = self.reservation_key.clone();
            world.get_resource_mut::<super::usable_effects::EffectRequests>()
                .ok_or("owned power has no canonical FX request owner")?
                .reserve(key.clone(), std::mem::take(&mut self.effect_paths));
            self.reservation = Some(key);
        }
        super::usable_effects::reservation_ready(world, self.reservation.as_deref().expect("power reservation identity"))
    }
    pub fn side(&self) -> usize {
        self.side
    }
    pub fn can_apply(&self, player: &Player) -> bool {
        // Resource staging does not activate the target. The native switch
        // state owns admission, cancellation and the actual tree replacement.
        player.body.arms.get(self.side).is_some()
    }
    pub fn is_current(&self, player: &Player) -> bool {
        self.banks.iter().enumerate().all(|(target, bank)| {
            let Some(bank) = bank else { return true };
            match target {
                0 => player.body.animations.len() == bank.base,
                1 | 2 => player.body.arms.get(target - 1).is_some_and(|arm| {
                    arm.model == bank.model && arm.animations.len() == bank.base
                }),
                3 | 4 => player.body.equipment[target - 3]
                    .as_ref()
                    .is_some_and(|tree| {
                        tree.model == bank.model && tree.animations.len() == bank.base
                    }),
                _ => false,
            }
        })
    }
    fn stage(
        mut self,
        player: &mut Player,
    ) -> Result<(player::equipment_states::Target, PowerActivation), String> {
        if !self.can_apply(player) {
            return Err("power arm tree is absent".into());
        }
        if !self.is_current(player) {
            return Err("player clip banks changed during power preparation".into());
        }
        for (target, bank) in self.banks.drain(..).enumerate() {
            let Some(bank) = bank else { continue };
            let aliases = bank
                .aliases
                .into_iter()
                .map(|(alias, index)| (alias, bank.base + index));
            match target {
                0 => {
                    player.body.animations.extend(bank.animations);
                    player.body.animation_aliases.extend(aliases);
                }
                1 | 2 => {
                    let arm = &mut player.body.arms[target - 1];
                    arm.animations.extend(bank.animations);
                    arm.animation_aliases.extend(aliases);
                }
                3 | 4 => {
                    let tree = player.body.equipment[target - 3]
                        .as_mut()
                        .expect("validated equipment tree");
                    tree.animations.extend(bank.animations);
                    tree.animation_aliases.extend(aliases);
                }
                _ => unreachable!(),
            }
        }
        let arm = &mut player.body.arms[self.side];
        if let Some(prepared) = self.arm {
            arm.install_equipment(prepared);
        }
        player.install_animation_scenes(self.scenes.clone());
        if self.id == 6 { player.register_grapple_scenes(self.scenes.clone()); }
        if !player.has_registered_power(self.id)
            && !player.register_power(
                crate::gameplay::powers::Cast::new(self.settings),
                self.scenes,
            )
        {
            return Err("owned power component registration was rejected".into());
        }
        let entity = self
            .item
            .equipment
            .clone()
            .ok_or("selected power has no equipment entity")?;
        let equipment = Arc::new(self.equipment);
        Ok((
            player::equipment_states::Target {
                identity: self.item.path.to_string(),
                entity,
                equipment: equipment.clone(),
                tree: None,
            },
            PowerActivation {
                id: self.id,
                blink: self.blink,
                far_reach: self.far_reach,
                equipment,
            },
        ))
    }
}

/// CPU preparation is complete, but the material/image and render owners
/// must admit the actual selected model before apply changes the live hand.
pub(super) struct ReadyEquipment {
    pub item: Arc<PickableItem>,
    pub descriptor: Arc<Descriptor>,
    pub tree: EquipmentTree,
    side: usize,
    arm: Option<PreparedEquipment>,
    banks: Vec<Option<Bank>>,
    scenes: Scenes,
}

impl ReadyEquipment {
    pub fn side(&self) -> usize {
        self.side
    }

    pub fn can_apply(&self, player: &Player) -> bool {
        player.body.arms.get(self.side).is_some()
    }

    pub fn is_current(&self, player: &Player) -> bool {
        self.banks.iter().enumerate().all(|(target, bank)| {
            let Some(bank) = bank else { return true };
            match target {
                0 => player.body.animations.len() == bank.base,
                1 | 2 => player.body.arms.get(target - 1).is_some_and(|arm| {
                    arm.model == bank.model && arm.animations.len() == bank.base
                }),
                3 | 4 if target == self.side + 3 => self.tree.animations.len() == bank.base,
                3 | 4 => player.body.equipment[target - 3]
                    .as_ref()
                    .is_some_and(|tree| {
                        tree.model == bank.model && tree.animations.len() == bank.base
                    }),
                _ => false,
            }
        })
    }

    /// Validate the entire transaction before touching a live bank. A stale
    /// result is reacquired from new alias-index snapshots by the caller.
    pub fn stage(
        mut self,
        player: &mut Player,
    ) -> Result<player::equipment_states::Target, String> {
        if !self.can_apply(player) {
            return Err("equipment arm tree is absent".into());
        }
        if !self.is_current(player) {
            return Err("player clip banks changed during equipment preparation".into());
        }
        for (target, bank) in self.banks.drain(..).enumerate() {
            let Some(bank) = bank else { continue };
            let aliases = bank
                .aliases
                .into_iter()
                .map(|(alias, index)| (alias, bank.base + index));
            match target {
                0 => {
                    player.body.animations.extend(bank.animations);
                    player.body.animation_aliases.extend(aliases);
                }
                1 | 2 => {
                    let arm = &mut player.body.arms[target - 1];
                    arm.animations.extend(bank.animations);
                    arm.animation_aliases.extend(aliases);
                }
                3 | 4 => {
                    let tree = if target == self.side + 3 {
                        &mut self.tree
                    } else {
                        player.body.equipment[target - 3]
                            .as_mut()
                            .expect("validated equipment tree")
                    };
                    tree.animations.extend(bank.animations);
                    tree.animation_aliases.extend(aliases);
                }
                _ => unreachable!(),
            }
        }
        let arm = &mut player.body.arms[self.side];
        if let Some(prepared) = self.arm.take() {
            arm.install_equipment(prepared);
        }
        player.install_animation_scenes(self.scenes);
        Ok(player::equipment_states::Target {
            identity: self.item.path.to_string(),
            entity: self.descriptor.entity.clone(),
            equipment: Arc::new(self.descriptor.equipment.clone()),
            tree: Some(self.tree),
        })
    }
}

#[derive(Resource)]
pub(super) struct PlayerActions {
    catalog: Arc<Catalog>,
    body: Arc<Rig>,
    arms: [Arc<Rig>; 2],
    definitions: Arc<[HashMap<String, Arc<EquipmentDefinition>>; 2]>,
    sources: Arc<Mutex<Sources>>,
    cancelled: Arc<AtomicBool>,
    loading: HashMap<String, Pending>,
    power_loading: HashMap<String, PendingPower>,
    metadata_loading: HashMap<String, PendingMetadata>,
    owned_loading: HashMap<String, PendingOwned>,
    owned_prepared: HashSet<String>,
    owned_targets: VecDeque<String>,
    failed_targets: HashSet<String>,
    pub prepared_owned_equipment: Vec<(String, Arc<Descriptor>)>,
    pub prepared_owned_metadata: Vec<(String, Arc<crate::assets::equipment::Metadata>)>,
    equipment_admissions: HashSet<String>,
    startup_error: Option<String>,
    metadata: HashMap<String, Arc<crate::assets::equipment::Metadata>>,
    metadata_failed: HashSet<String>,
    metadata_order: VecDeque<String>,
    shortcut_items: VecDeque<String>,
    ready: Vec<ReadyEquipment>,
    power_ready: Vec<ReadyPower>,
    active: [Option<Arc<Descriptor>>; 2],
    wanted: [Option<String>; 16],
    source_flags: HashMap<String, u32>,
    pub selected_ammunition: HashMap<String, Option<String>>,
    pub blink: Option<super::blink::Runtime>,
    pub active_blink: Option<super::blink::Runtime>,
    pub retiring_blinks: Vec<super::blink::Runtime>,
    blink_player: Option<super::blink::PlayerSettings>,
    far_reach_player: Option<super::far_reach::PlayerSettings>,
    pub far_reach: Option<super::far_reach::Runtime>,
    pub active_far_reach: Option<super::far_reach::Runtime>,
    slash_scene_paths: Arc<[String]>,
    power_activations: HashMap<String, PowerActivation>,
    power_reservations: HashMap<String, Arc<str>>,
    power_dependency_ids: HashMap<String, i32>,
    retiring_power_dependencies: Vec<PowerDependencyRetirement>,
    next_power_reservation: u64,
    pub power_targets: Vec<player::equipment_states::Target>,
    active_power: Option<String>,
    pub potion_sounds: [Option<Arc<SoundEventDeclaration>>; 2],
    pub potion_results: Vec<player::consumables::PotionResult>,
    pub selection_requests: Vec<ShortcutSelection>,
    pub removed_equipment: Vec<String>,
    shortcuts: Arc<ShortcutSettings>,
    shortcuts_configured: bool,
    rehydrated: bool,
}

impl Drop for PlayerActions {
    fn drop(&mut self) {
        self.close_acquisitions();
    }
}

impl PlayerActions {
    pub fn prepare(
        catalog: Arc<Catalog>,
        declarations: &crate::entities::player::PlayerDeclarations,
    ) -> Result<Self, String> {
        let body = Rig::acquire(&catalog, &declarations.body_model)?;
        let parts = player::decls::arm_parts(&declarations.body)?;
        let mut rigs = Vec::with_capacity(2);
        let mut definitions: [HashMap<String, Arc<EquipmentDefinition>>; 2] = Default::default();
        for (side, (part, _, _)) in parts.iter().enumerate() {
            let entity = part.as_deref().ok_or("player arm has no entity")?;
            let fields = crate::entities::player::resolved_decl("entitydef", entity, |path| {
                Ok(catalog
                    .read(path)
                    .map_err(|error| error.to_string())?
                    .map(crate::entities::owned_declaration_text))
            })?;
            let model = crate::entities::scene_motion::atom(
                crate::entities::scene_motion::child(&fields, "renderModelInfo"),
                "model",
            )
            .ok_or("player arm has no render model")?;
            rigs.push(Rig::acquire(&catalog, model)?);
            // Metadata only. No unselected equipment animation, model,
            // material or GPU allocation is acquired by this projection.
            for name in player::decls::arm_equipment_names(&declarations.locomotion, side) {
                definitions[side].insert(
                    name.clone(),
                    Arc::new(EquipmentDefinition::read(
                        &declarations.locomotion,
                        &declarations.body,
                        side,
                        &name,
                    )?),
                );
            }
        }
        let potion_sounds = player::consumables::PotionSettings::sounds(
            &catalog,
            &declarations.health,
            &declarations.mana,
        )?;
        let shortcuts = Arc::new(ShortcutSettings::read(&declarations.controller)?);
        let mut slash_scene_paths = player::combat::equipment_scene_paths(declarations);
        if let Some(definition) = declarations
            .states
            .iter()
            .find(|(kind, _)| kind == "playerstateslash")
            .and_then(|(_, fields)| crate::entities::scene_motion::atom(fields, "m_ruleHandler"))
            .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
            .map(|name| crate::assets::slash_rules::Definition::acquire(&catalog, name))
            .transpose()?
        {
            slash_scene_paths.extend(definition.scene_paths().map(str::to_owned));
        }
        slash_scene_paths.sort();
        slash_scene_paths.dedup();
        let slash_scene_paths = Arc::from(slash_scene_paths);
        let blink_player = declarations
            .states
            .iter()
            .filter(|_| !declarations.blink_mantle_sensor.is_empty())
            .find(|(kind, _)| kind == "playerstateblinking")
            .map(|(_, state)| {
                super::blink::Prepared::player(state, &declarations.blink_mantle_sensor)
            })
            .transpose()?;
        let far_reach_player = declarations.states.iter()
            .filter(|_| !declarations.grapple_sensor.is_empty())
            .find(|(kind, _)| kind == "playerstategrapple")
            .map(|(_, state)| super::far_reach::Prepared::player(state, &declarations.grapple_sensor)).transpose()?;
        Ok(Self {
            catalog,
            body,
            arms: rigs
                .try_into()
                .map_err(|_| "player requires two arm rigs")?,
            definitions: Arc::new(definitions),
            sources: Default::default(),
            cancelled: Default::default(),
            loading: Default::default(),
            power_loading: Default::default(),
            metadata_loading: Default::default(),
            owned_loading: Default::default(),
            owned_prepared: Default::default(),
            owned_targets: Default::default(),
            failed_targets: Default::default(),
            prepared_owned_equipment: Vec::new(),
            prepared_owned_metadata: Vec::new(),
            equipment_admissions: Default::default(),
            startup_error: None,
            metadata: Default::default(),
            metadata_failed: Default::default(),
            metadata_order: Default::default(),
            shortcut_items: Default::default(),
            ready: Vec::new(),
            power_ready: Vec::new(),
            active: Default::default(),
            wanted: Default::default(),
            source_flags: Default::default(),
            selected_ammunition: Default::default(),
            potion_sounds,
            potion_results: Vec::new(),
            blink: None,
            active_blink: None,
            retiring_blinks: Vec::new(),
            blink_player,
            far_reach_player,
            far_reach: None,
            active_far_reach: None,
            slash_scene_paths,
            power_activations: Default::default(),
            power_reservations: Default::default(),
            power_dependency_ids: Default::default(),
            retiring_power_dependencies: Vec::new(),
            next_power_reservation: 0,
            power_targets: Vec::new(),
            active_power: None,
            selection_requests: Vec::new(),
            removed_equipment: Vec::new(),
            shortcuts,
            shortcuts_configured: false,
            rehydrated: false,
        })
    }

    pub fn close_acquisitions(&self) {
        self.cancelled.store(true, Ordering::Release);
        for pending in self.loading.values() {
            pending.cancelled.store(true, Ordering::Release);
        }
        for pending in self.power_loading.values() {
            pending.cancelled.store(true, Ordering::Release);
        }
        for pending in self.metadata_loading.values() {
            pending.cancelled.store(true, Ordering::Release);
        }
        for pending in self.owned_loading.values() {
            pending.cancelled.store(true, Ordering::Release);
        }
    }

    pub fn startup_pending(&self) -> bool {
        !self.rehydrated
            || !self.loading.is_empty()
            || !self.power_loading.is_empty()
            || !self.metadata_loading.is_empty()
            || !self.ready.is_empty()
            || !self.power_ready.is_empty()
            || !self.selection_requests.is_empty()
            || !self.owned_loading.is_empty()
            || !self.prepared_owned_equipment.is_empty()
            || !self.prepared_owned_metadata.is_empty()
            || !self.owned_targets.is_empty()
    }

    /// Keep tasks alive until their synchronous archive reads have returned.
    /// Merely dropping a task does not acknowledge Catalog scope retirement.
    pub fn retire_acquisitions(&mut self) -> bool {
        self.close_acquisitions();
        self.ready.clear();
        self.power_ready.clear();
        self.prepared_owned_equipment.clear();
        self.prepared_owned_metadata.clear();
        let running = |pending: &mut Pending| {
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            block_on(poll_once(task)).is_none()
        };
        self.loading.retain(|_, pending| running(pending));
        self.power_loading.retain(|_, pending| {
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            block_on(poll_once(task)).is_none()
        });
        self.metadata_loading.retain(|_, pending| {
            if pending.complete.is_some() {
                return false;
            }
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            block_on(poll_once(task)).is_none()
        });
        self.owned_loading.retain(|_, pending| {
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            block_on(poll_once(task)).is_none()
        });
        self.loading.is_empty()
            && self.power_loading.is_empty()
            && self.metadata_loading.is_empty()
            && self.owned_loading.is_empty()
    }

    pub fn requested(&self, path: &str) -> bool {
        self.loading.contains_key(path)
            || self.power_loading.contains_key(path)
            || self
                .ready
                .iter()
                .any(|ready| ready.item.path.as_ref() == path)
            || self
                .power_ready
                .iter()
                .any(|ready| ready.item.path.as_ref() == path)
    }
    pub fn startup_error(&self) -> Option<&str> { self.startup_error.as_deref() }
    pub fn take_equipment_admission(&mut self, identity: &str) -> bool { self.equipment_admissions.remove(identity) }

    pub fn active(&self, side: usize) -> Option<&Arc<Descriptor>> {
        self.active.get(side)?.as_ref()
    }

    pub fn accepted(&mut self, side: usize, descriptor: Arc<Descriptor>) {
        self.active[side] = Some(descriptor);
    }
    pub fn remove_active(&mut self, side: usize) -> Option<Arc<Descriptor>> {
        self.active.get_mut(side)?.take()
    }

    /// Called only by the actual equipment state's Equipped callback. Scene
    /// preparation does not grant ownership or replace an in-flight cast.
    pub fn activate_power(
        &mut self,
        identity: &str,
        player: &mut Player,
    ) -> Result<Option<(i32, Arc<player::weapons::Equipment>)>, String> {
        let Some(activation) = self.power_activations.get(identity) else {
            return Ok(None);
        };
        let id = activation.id;
        let equipment = activation.equipment.clone();
        let blink = activation.blink.clone();
        let far_reach = activation.far_reach.clone();
        if !player.equip_registered_power(id) {
            return Err("native equipment activation could not install the selected power".into());
        }
        self.detach_blink();
        self.detach_far_reach(player);
        self.blink = blink.map(|prepared| super::blink::Runtime::new(prepared, &player.physics));
        self.active_power = Some(identity.to_owned());
        self.far_reach = far_reach.map(|prepared| super::far_reach::Runtime::new(prepared, &player.physics));
        Ok(Some((id, equipment)))
    }

    pub fn cancel_power_selection(&mut self, identity: &str) {
        // Cancelling a switch does not destroy the still-owned power entity.
        // Its immutable prepared action inputs survive until quantity zero.
        let _ = identity;
    }

    pub fn deactivate_power(&mut self, identity: &str, player: &mut Player) -> bool {
        if self.active_power.as_deref() != Some(identity) {
            return false;
        }
        player.unequip_power();
        self.detach_blink();
        self.detach_far_reach(player);
        self.active_power = None;
        true
    }

    fn detach_blink(&mut self) {
        if let Some(blink) = self.blink.take() {
            if blink.has_activity() {
                // Only one actual Blinking state exists. Its motion, time
                // lease and damage/FX payload outlive equipment selection.
                assert!(
                    self.active_blink.is_none(),
                    "two simultaneous Blink movement owners"
                );
                self.active_blink = Some(blink);
            } else {
                // Even an idle target can own helper FX. The canonical
                // dispatcher retires these with its real clock/FX owners.
                self.retiring_blinks.push(blink);
            }
        }
    }
    fn detach_far_reach(&mut self, player: &mut Player) {
        if let Some(mut runtime) = self.far_reach.take() {
            if runtime.has_activity() {
                assert!(self.active_far_reach.is_none(), "two simultaneous Grapple movement owners");
                self.active_far_reach = Some(runtime);
            } else { runtime.cancel(player); }
        }
    }
    pub fn wanted(&self, slot: u8, path: &str) -> bool {
        self.wanted.get(slot as usize).and_then(Option::as_deref) == Some(path)
    }
    pub fn request_flags(&self, identity: &str) -> Option<u32> {
        self.source_flags.get(identity).copied()
    }
    pub(super) fn equipment_attachment_source(&self) -> (Arc<Catalog>, Arc<Mutex<Sources>>) {
        (self.catalog.clone(), self.sources.clone())
    }

    fn request_metadata(&mut self, item: &PickableItem, admit: bool) {
        if self.cancelled.load(Ordering::Acquire)
            || self.metadata.contains_key(item.path.as_ref())
            || self.metadata_failed.contains(item.path.as_ref())
        {
            return;
        }
        if let Some(pending) = self.metadata_loading.get_mut(item.path.as_ref()) {
            pending.admit |= admit;
            return;
        }
        let Some(entity) = item.equipment.clone() else {
            return;
        };
        let sources = self.sources.clone();
        let catalog = self.catalog.clone();
        let closed = self.cancelled.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let task = IoTaskPool::get().spawn(async move {
            let cancelled =
                || closed.load(Ordering::Acquire) || worker_cancelled.load(Ordering::Acquire);
            if cancelled() {
                return Err("equipment metadata acquisition cancelled".into());
            }
            let result = sources
                .lock()
                .map_err(|_| "equipment sources are poisoned")?
                .metadata(
                    &catalog,
                    &entity,
                    crate::assets::equipment::enumeration,
                    Some(&cancelled),
                );
            result
        });
        self.metadata_order.push_back(item.path.to_string());
        self.metadata_loading.insert(
            item.path.to_string(),
            PendingMetadata {
                cancelled,
                task: Mutex::new(task),
                complete: None,
                admit,
            },
        );
    }

    fn finish_metadata(&mut self, inventory: &mut DiscoveredDocuments) {
        for pending in self.metadata_loading.values_mut() {
            if pending.complete.is_some() {
                continue;
            }
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(result) = block_on(poll_once(task)) {
                pending.complete = Some(result);
            }
        }
        // 619760 chooses the first admitted item per slot. IO completion
        // order must not change the authored loadout's equipment priority.
        while let Some(path) = self.metadata_order.front() {
            let Some(pending) = self.metadata_loading.get_mut(path) else {
                self.metadata_order.pop_front();
                continue;
            };
            let Some(result) = pending.complete.take() else {
                break;
            };
            let cancelled = pending.cancelled.load(Ordering::Acquire);
            let admit = pending.admit;
            let path = self
                .metadata_order
                .pop_front()
                .expect("metadata request order");
            self.metadata_loading.remove(&path);
            if cancelled {
                if inventory.quantity(&path) > 0 {
                    if let Some(item) = inventory.declaration(&path).cloned() {
                        self.request_metadata(&item, admit);
                    }
                }
                continue;
            }
            let metadata = match result {
                Ok(metadata) => metadata,
                Err(error) => {
                    eprintln!("Equipment metadata {path}: {error}");
                    self.startup_error.get_or_insert_with(|| format!("Equipment metadata {path}: {error}"));
                    self.metadata_failed.insert(path);
                    continue;
                }
            };
            if inventory.quantity(&path) == 0 {
                continue;
            }
            if let Some(ranged) = &metadata.ranged {
                for ammo in &ranged.ammunition {
                    inventory.bind_ammunition(&path, &ammo.item);
                }
            }
            if admit {
                self.equipment_admissions.insert(path.clone());
            }
            // Non-rendering powers are real managed entities too.
            self.prepared_owned_metadata.push((path.clone(), metadata.clone()));
            self.request_owned(&path, &metadata);
            self.owned_targets.push_back(path.clone());
            self.metadata.insert(path, metadata);
        }
        while let Some(path) = self.shortcut_items.front() {
            if self.metadata_failed.contains(path) {
                self.shortcut_items.pop_front();
                continue;
            }
            let Some(item) = inventory.declaration(path) else {
                self.shortcut_items.pop_front();
                continue;
            };
            if item.equipment.is_none() {
                inventory.admit_shortcut_item(path, None);
            } else if let Some(metadata) = self.metadata.get(path) {
                inventory.admit_shortcut_item(path, Some(&metadata.equipment));
            } else {
                break;
            }
            self.shortcut_items.pop_front();
        }
    }

    fn request_owned(&mut self, path: &str, metadata: &crate::assets::equipment::Metadata) {
        // 6170b0 creates the actual entity on 0->positive, before autoequip.
        // A model-less power is a real non-rendering entity, not a failed
        // weapon. Only authored renderModelInfo presence requests geometry.
        if metadata.model.is_none()
            || self.owned_prepared.contains(path)
            || self.owned_loading.contains_key(path)
            || self.cancelled.load(Ordering::Acquire)
        {
            return;
        }
        let catalog = self.catalog.clone();
        let sources = self.sources.clone();
        let entity = metadata.entity.clone();
        let closed = self.cancelled.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let task = IoTaskPool::get().spawn(async move {
            let cancelled =
                || closed.load(Ordering::Acquire) || worker_cancelled.load(Ordering::Acquire);
            if cancelled() {
                return Err("owned equipment acquisition cancelled".into());
            }
            let result = sources
                .lock()
                .map_err(|_| "equipment sources are poisoned")?
                .acquire(
                    &catalog,
                    &entity,
                    crate::assets::equipment::enumeration,
                    Some(&cancelled),
                );
            result
        });
        self.owned_loading.insert(
            path.to_owned(),
            PendingOwned {
                cancelled,
                task: Mutex::new(task),
            },
        );
    }

    fn finish_owned(&mut self, inventory: &DiscoveredDocuments) {
        let mut completed = Vec::new();
        self.owned_loading.retain(|path, pending| {
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            match block_on(poll_once(task)) {
                None => true,
                Some(Ok(descriptor)) => {
                    if !pending.cancelled.load(Ordering::Acquire) && inventory.quantity(path) > 0 {
                        completed.push((path.clone(), descriptor));
                    }
                    false
                }
                Some(Err(error)) => {
                    if !pending.cancelled.load(Ordering::Acquire) {
                        eprintln!("Owned equipment {path}: {error}");
                        self.startup_error.get_or_insert_with(|| format!("Owned equipment {path}: {error}"));
                    }
                    false
                }
            }
        });
        for (path, descriptor) in completed {
            self.owned_prepared.insert(path.clone());
            self.prepared_owned_equipment.push((path, descriptor));
        }
    }

    pub fn take_ready(&mut self) -> Vec<ReadyEquipment> {
        let mut completed = Vec::new();
        self.loading.retain(|path, pending| {
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            match block_on(poll_once(task)) {
                None => true,
                Some(Ok(ready)) => {
                    if !pending.cancelled.load(Ordering::Acquire) {
                        completed.push(ready);
                    }
                    false
                }
                Some(Err(error)) => {
                    if !pending.cancelled.load(Ordering::Acquire) {
                        eprintln!("Equipment {path}: {error}");
                        self.startup_error.get_or_insert_with(|| format!("Equipment {path}: {error}"));
                        self.failed_targets.insert(path.clone());
                    }
                    false
                }
            }
        });
        self.ready.extend(completed);
        std::mem::take(&mut self.ready)
    }

    pub fn requeue(&mut self, ready: ReadyEquipment) {
        self.ready.push(ready);
    }

    pub fn take_ready_powers(&mut self) -> Vec<ReadyPower> {
        let mut completed = Vec::new();
        self.power_loading.retain(|path, pending| {
            let task = pending
                .task
                .get_mut()
                .unwrap_or_else(|error| error.into_inner());
            match block_on(poll_once(task)) {
                None => true,
                Some(Ok(ready)) => {
                    if !pending.cancelled.load(Ordering::Acquire) {
                        completed.push(ready);
                    }
                    false
                }
                Some(Err(error)) => {
                    if !pending.cancelled.load(Ordering::Acquire) {
                        eprintln!("Power {path}: {error}");
                        self.startup_error.get_or_insert_with(|| format!("Power {path}: {error}"));
                        self.failed_targets.insert(path.clone());
                    }
                    false
                }
            }
        });
        self.power_ready.extend(completed);
        std::mem::take(&mut self.power_ready)
    }

    pub fn requeue_power(&mut self, ready: ReadyPower) {
        self.power_ready.push(ready);
    }

    pub fn configure_shortcuts(&mut self, inventory: &mut DiscoveredDocuments) {
        if !self.shortcuts_configured {
            inventory.configure_shortcuts(self.shortcuts.clone());
            self.shortcuts_configured = true;
        }
    }

    pub fn request_power(
        &mut self,
        item: Arc<PickableItem>,
        definitions: Arc<(Vec<crate::assets::power_journal::Power>, String)>,
        index: usize,
        equipment: player::weapons::Equipment,
        player: &Player,
    ) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) || self.requested(&item.path) {
            return Ok(());
        }
        let side = match equipment.slot {
            1 => 0,
            0 => 1,
            slot => return Err(format!("power slot {slot} has no player arm")),
        };
        let power = definitions
            .0
            .get(index)
            .ok_or("selected power definition is absent")?;
        let id = crate::assets::power_journal::power_id(&power.power_id)
            .ok_or("power has no registered ID")?;
        let mut settings = crate::gameplay::powers::Settings::read(id, &power.component)?;
        let blink = if id == 2 {
            Some(super::blink::Prepared::with_power(
                &power.component,
                self.blink_player
                    .as_ref()
                    .ok_or("Blink player movement/sensor declarations are absent")?,
            )?)
        } else {
            None
        };
        let far_reach = if id == 6 {
            Some(super::far_reach::Prepared::with_power(&power.component, self.far_reach_player.as_ref()
                .ok_or("Far Reach player movement/sensor declarations are absent")?)?)
        } else { None };
        let name = equipment
            .loco_equip_state
            .as_deref()
            .ok_or("power has no locomotion equipment state")?;
        let definition = self.definitions[side]
            .get(name)
            .cloned()
            .ok_or_else(|| format!("arm {side} has no {name} equipment state"))?;
        let mut snapshots: [Option<Snapshot>; 5] = Default::default();
        snapshots[0] = Some(Snapshot {
            rig: self.body.clone(),
            model: self.body.path.clone(),
            aliases: player.body.animation_aliases.clone(),
            base: player.body.animations.len(),
        });
        for (side, arm) in player.body.arms.iter().enumerate().take(2) {
            snapshots[side + 1] = Some(Snapshot {
                rig: self.arms[side].clone(),
                model: arm.model.clone(),
                aliases: arm.animation_aliases.clone(),
                base: arm.animations.len(),
            });
        }
        for (other, tree) in player.body.equipment.iter().enumerate() {
            if other == side {
                continue;
            }
            if let Some(tree) = tree {
                snapshots[other + 3] = Some(Snapshot {
                    rig: tree.rig.clone(),
                    model: tree.model.clone(),
                    aliases: tree.animation_aliases.clone(),
                    base: tree.animations.len(),
                });
            }
        }
        let arm = player
            .body
            .arms
            .get(side)
            .ok_or("power arm tree is absent")?;
        let already_prepared = arm.has_equipment(name);
        let additives = arm.additives.items.clone();
        let catalog = self.catalog.clone();
        let closed = self.cancelled.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let path = item.path.to_string();
        self.power_dependency_ids.insert(path.clone(), id);
        let reservation_key = match self.power_reservations.get(&path) {
            Some(key) => key.clone(),
            None => {
                self.next_power_reservation = self.next_power_reservation.wrapping_add(1);
                let key: Arc<str> = format!("inventory-power:{}:{}", self.next_power_reservation, path).into();
                self.power_reservations.insert(path.clone(), key.clone()); key
            }
        };
        let task = IoTaskPool::get().spawn(async move {
            // Retain the canonical metadata owner while acquiring its paths.
            let power = definitions.0.get(index).ok_or("owned power definition is absent")?;
            let cancelled =
                || closed.load(Ordering::Acquire) || worker_cancelled.load(Ordering::Acquire);
            if cancelled() {
                return Err("power acquisition cancelled".into());
            }
            if let Some(name) = &settings.cancel_sound {
                settings.cancel_sound_event = crate::assets::fx::SoundEventAsset {
                    delay: [0.0; 2], event: Some(name.clone()),
                }.resolve(&catalog)?;
            }
            let family = match id {
                1 => Some(PreparedFamily::BendTime(super::power_world::PreparedSources::acquire_power(&catalog, power)?)),
                10 => Some(PreparedFamily::Windblast(super::windblast::PreparedSources::acquire_power(&catalog, power)?)),
                _ => None,
            };
            let mut effect_paths = blink.as_ref().into_iter().flat_map(|prepared| prepared.effect_paths()).map(str::to_owned).collect::<Vec<_>>();
            if let Some(family) = &family {
                match family {
                    PreparedFamily::BendTime(sources) => effect_paths.extend(sources.effect_paths().map(str::to_owned)),
                    PreparedFamily::Windblast(sources) => effect_paths.extend(sources.effect_paths().map(str::to_owned)),
                }
            }
            effect_paths.sort(); effect_paths.dedup();
            if cancelled() { return Err("power dependency acquisition cancelled".into()); }
            let rigs: Vec<_> = snapshots
                .iter()
                .map(|snapshot| snapshot.as_ref().map(|snapshot| snapshot.rig.clone()))
                .collect();
            let mut clips: Vec<Option<Clips<'_>>> = rigs
                .iter()
                .map(|rig| {
                    rig.as_ref().map(|rig| Clips {
                        catalog: &catalog,
                        rig,
                        rest: &rig.skeleton.rest,
                        animations: Vec::new(),
                        loaded: HashMap::new(),
                    })
                })
                .collect();
            let snapshot = snapshots[side + 1].as_ref().expect("power arm snapshot");
            let arm = if already_prepared {
                None
            } else {
                Some(player::arms::prepare_equipment(
                    clips[side + 1].as_mut().expect("power arm clips"),
                    &snapshot.aliases,
                    snapshot.base,
                    &definition,
                    &additives,
                    Some(&cancelled),
                )?)
            };
            let mut paths: Vec<_> = [
                &settings.in_midnight,
                &settings.loop_midnight,
                &settings.out_midnight,
                &settings.in_midnight_slide,
                &settings.loop_midnight_slide,
                &settings.out_midnight_slide,
                &settings.cancel_midnight,
            ]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
            if let Some(prepared) = &far_reach { paths.extend(prepared.scene_paths().map(str::to_owned)); }
            paths.sort();
            paths.dedup();
            let scenes = prepare_scenes(&catalog, &paths, &snapshots, &mut clips, &cancelled)?;
            if cancelled() {
                return Err("power acquisition cancelled".into());
            }
            let banks = clips
                .into_iter()
                .zip(snapshots)
                .map(|(clips, snapshot)| {
                    clips.zip(snapshot).map(|(clips, snapshot)| Bank {
                        model: snapshot.model,
                        base: snapshot.base,
                        animations: clips.animations,
                        aliases: clips.loaded,
                    })
                })
                .collect();
            Ok(ReadyPower {
                item,
                equipment,
                id,
                side,
                arm,
                banks,
                settings,
                blink,
                far_reach,
                scenes,
                family,
                effect_paths,
                reservation_key,
                reservation: None,
            })
        });
        self.power_loading.insert(
            path,
            PendingPower {
                cancelled,
                task: Mutex::new(task),
            },
        );
        Ok(())
    }

    /// Request the actual owned item's authored entity. Archetype and slot
    /// are resolved there; inventory type/UI index cannot substitute for them.
    pub fn request(&mut self, item: Arc<PickableItem>, player: &Player) -> Result<(), String> {
        if self.cancelled.load(Ordering::Acquire) || self.requested(&item.path) {
            return Ok(());
        }
        let entity = item
            .equipment
            .as_deref()
            .ok_or("inventory item has no equipment entity")?
            .to_owned();
        let mut snapshots: [Option<Snapshot>; 5] = Default::default();
        snapshots[0] = Some(Snapshot {
            rig: self.body.clone(),
            model: self.body.path.clone(),
            aliases: player.body.animation_aliases.clone(),
            base: player.body.animations.len(),
        });
        let mut additive_items = Vec::with_capacity(2);
        for (side, arm) in player.body.arms.iter().enumerate().take(2) {
            snapshots[side + 1] = Some(Snapshot {
                rig: self.arms[side].clone(),
                model: arm.model.clone(),
                aliases: arm.animation_aliases.clone(),
                base: arm.animations.len(),
            });
            additive_items.push(arm.additives.items.clone());
        }
        if additive_items.len() != 2 {
            return Err("player equipment requires both arm trees".into());
        }
        for (side, tree) in player.body.equipment.iter().enumerate() {
            if let Some(tree) = tree {
                snapshots[side + 3] = Some(Snapshot {
                    rig: tree.rig.clone(),
                    model: tree.model.clone(),
                    aliases: tree.animation_aliases.clone(),
                    base: tree.animations.len(),
                });
            }
        }
        let catalog = self.catalog.clone();
        let sources = self.sources.clone();
        let definitions = self.definitions.clone();
        let prepared_states = player
            .body
            .arms
            .iter()
            .map(|arm| arm.equipment().to_owned())
            .collect::<Vec<_>>();
        let slash_scene_paths = self.slash_scene_paths.clone();
        let arm_rigs = self.arms.clone();
        let closed = self.cancelled.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let path = item.path.to_string();
        let task = IoTaskPool::get().spawn(async move {
            let cancelled =
                || closed.load(Ordering::Acquire) || worker_cancelled.load(Ordering::Acquire);
            if cancelled() {
                return Err("equipment acquisition cancelled".into());
            }
            let descriptor = sources
                .lock()
                .map_err(|_| "equipment sources are poisoned")?
                .acquire(
                    &catalog,
                    &entity,
                    crate::assets::equipment::enumeration,
                    Some(&cancelled),
                )?;
            let side = match descriptor.equipment.slot {
                1 => 0,
                0 => 1,
                slot => return Err(format!("equipment slot {slot} has no first-person tree")),
            };
            let name = descriptor
                .equipment
                .loco_equip_state
                .as_deref()
                .ok_or("equipment has no locomotion state")?;
            let definition = definitions[side]
                .get(name)
                .ok_or_else(|| format!("arm {side} has no equipment locomotion state {name}"))?;
            let body = snapshots[0].as_ref().expect("body snapshot").rig.clone();
            let tree = descriptor.instantiate(&body, &arm_rigs, true)?;
            snapshots[side + 3] = Some(Snapshot {
                rig: tree.rig.clone(),
                model: tree.model.clone(),
                aliases: tree.animation_aliases.clone(),
                base: tree.animations.len(),
            });
            let rigs: Vec<_> = snapshots
                .iter()
                .map(|snapshot| snapshot.as_ref().map(|snapshot| snapshot.rig.clone()))
                .collect();
            let mut clips: Vec<Option<Clips<'_>>> = rigs
                .iter()
                .map(|rig| {
                    rig.as_ref().map(|rig| Clips {
                        catalog: &catalog,
                        rig,
                        rest: &rig.skeleton.rest,
                        animations: Vec::new(),
                        loaded: HashMap::new(),
                    })
                })
                .collect();
            let snapshot = snapshots[side + 1].as_ref().expect("arm snapshot");
            let arm = if prepared_states
                .get(side)
                .is_some_and(|current| current == name)
            {
                None
            } else {
                Some(player::arms::prepare_equipment(
                    clips[side + 1].as_mut().expect("arm clips"),
                    &snapshot.aliases,
                    snapshot.base,
                    definition,
                    &additive_items[side],
                    Some(&cancelled),
                )?)
            };
            let mut paths: Vec<_> = descriptor.equipment.scenes.values().cloned().collect();
            if descriptor.equipment.archetype == 1 {
                paths.extend(slash_scene_paths.iter().cloned());
            }
            paths.sort();
            paths.dedup();
            let scenes = prepare_scenes(&catalog, &paths, &snapshots, &mut clips, &cancelled)?;
            if cancelled() {
                return Err("equipment acquisition cancelled".into());
            }
            let banks = clips
                .into_iter()
                .zip(snapshots)
                .map(|(clips, snapshot)| {
                    clips.zip(snapshot).map(|(clips, snapshot)| Bank {
                        model: snapshot.model,
                        base: snapshot.base,
                        animations: clips.animations,
                        aliases: clips.loaded,
                    })
                })
                .collect();
            Ok(ReadyEquipment {
                item,
                descriptor,
                tree,
                side,
                arm,
                banks,
                scenes,
            })
        });
        self.loading.insert(
            path,
            Pending {
                cancelled,
                task: Mutex::new(task),
            },
        );
        Ok(())
    }
}

fn prepare_scenes(
    catalog: &Catalog,
    paths: &[String],
    snapshots: &[Option<Snapshot>; 5],
    clips: &mut [Option<Clips<'_>>],
    cancelled: &dyn Fn() -> bool,
) -> Result<Scenes, String> {
    use crate::gameplay::sound::Emitter;
    let models: [&str; 5] = std::array::from_fn(|target| {
        snapshots[target]
            .as_ref()
            .map_or("", |snapshot| snapshot.model.as_str())
    });
    let casting = [
        ("PlayerBody", models[0], Emitter::PlayerBody),
        ("PlayerCamera", models[0], Emitter::PlayerBody),
        ("PlayerLeftArm", models[1], Emitter::PlayerLeftArm),
        ("PlayerRightArm", models[2], Emitter::PlayerRightArm),
        (
            "PlayerLeftEquipment",
            models[3],
            Emitter::PlayerLeftEquipment,
        ),
        (
            "PlayerRightEquipment",
            models[4],
            Emitter::PlayerRightEquipment,
        ),
    ];
    let mut scenes = Scenes::new();
    for path in paths {
        if cancelled() {
            return Err("equipment scene acquisition cancelled".into());
        }
        let decl = crate::assets::midnight_scene::acquire_player(catalog, path, Some(cancelled))?;
        let count = decl.scenes.len();
        let sounds = player::midnight::prepare_sounds_owned(catalog, &decl, &casting)?;
        let mut scene = PreparedScene {
            decl,
            clips: vec![Vec::new(); count],
            arm_clips: [vec![Vec::new(); count], vec![Vec::new(); count]],
            equipment_clips: [vec![Vec::new(); count], vec![Vec::new(); count]],
            sounds,
        };
        let mut next = scene.decl.first;
        let mut visited = HashSet::new();
        while let Some(index) = Midnight::resolve(&scene, next, models)? {
            if !visited.insert(index) {
                break;
            }
            for target in 0..5 {
                let Some(group) = scene.group_for_model(index, target, models[target])? else {
                    continue;
                };
                let Some(snapshot) = &snapshots[target] else {
                    continue;
                };
                let bank = clips[target].as_mut().expect("snapshot bank");
                let keys = &scene.decl.scenes[index].groups[group].anim_keys;
                let indices = keys
                    .iter()
                    .map(|key| {
                        snapshot.aliases.get(&key.anim).copied().map_or_else(
                            || {
                                bank.clip_cancellable(&key.anim, Some(cancelled))
                                    .map(|index| snapshot.base + index)
                            },
                            Ok,
                        )
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                match target {
                    0 => scene.clips[index] = indices,
                    1 | 2 => scene.arm_clips[target - 1][index] = indices,
                    3 | 4 => scene.equipment_clips[target - 3][index] = indices,
                    _ => unreachable!(),
                }
            }
            next = scene.decl.scenes[index].next;
        }
        scenes.insert(path.clone(), Arc::new(scene));
    }
    Ok(scenes)
}

/// Called before level marker retirement, while the actual player/model
/// owners still exist. Completion discards cancelled worker results.
pub(super) fn retire_acquisitions(world: &mut World) -> bool {
    world
        .get_resource_mut::<PlayerActions>()
        .is_none_or(|mut actions| actions.retire_acquisitions())
}

/// Release owned time/FX requests before their shared managers are retired.
/// Both the selected component and a detached, still-moving Blink are real
/// consumers; equipment selection alone is not their completion signal.
pub(super) fn retire_effects(world: &mut World) -> Result<(), String> {
    let Some(actions) = world.get_resource::<PlayerActions>() else { return Ok(()); };
    let has_blinks = actions.blink.is_some() || actions.active_blink.is_some() || !actions.retiring_blinks.is_empty()
        || actions.far_reach.is_some() || actions.active_far_reach.is_some();
    let has_reservations = !actions.power_reservations.is_empty() || !actions.retiring_power_dependencies.is_empty();
    if !has_blinks && !has_reservations { return Ok(()); }
    if !world.contains_resource::<super::usable_effects::EffectRequests>() {
        return Err("player effects cannot retire after their FX request owner".into());
    }
    if has_blinks {
        if world.get_resource::<super::player::PlayerState>().and_then(|state| state.player.as_ref()).is_none() {
            return Err("player effects cannot retire after their player owner".into());
        }
        if !world.contains_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>() {
            return Err("player effects cannot retire after their clock owner".into());
        }
        world.resource_scope(|world, mut actions: Mut<PlayerActions>| {
            world.resource_scope(|world, mut state: Mut<super::player::PlayerState>| {
                world.resource_scope(|world, mut clock: Mut<crate::gameplay::game_clock_runtime::ClockRuntime>| {
                    let mut effects = world.resource_mut::<super::usable_effects::EffectRequests>();
                    let player = state.player.as_mut().expect("player retirement admission");
                    if let Some(blink) = actions.blink.as_mut() { blink.retire(player, &mut clock, &mut effects)?; }
                    if let Some(blink) = actions.active_blink.as_mut() { blink.retire(player, &mut clock, &mut effects)?; }
                    for blink in &mut actions.retiring_blinks { blink.retire(player, &mut clock, &mut effects)?; }
                    if let Some(runtime) = actions.far_reach.as_mut() { runtime.retire(player); }
                    if let Some(runtime) = actions.active_far_reach.as_mut() { runtime.retire(player); }
                    actions.retiring_blinks.clear(); actions.blink = None; actions.active_blink = None;
                    actions.far_reach = None; actions.active_far_reach = None;
                    Ok::<(), String>(())
                })
            })
        })?;
    }
    world.resource_scope(|world, mut actions: Mut<PlayerActions>| {
        let mut effects = world.resource_mut::<super::usable_effects::EffectRequests>();
        for (_, key) in actions.power_reservations.drain() { effects.release_reservation(key); }
        for retired in actions.retiring_power_dependencies.drain(..) { effects.release_reservation(retired.reservation); }
        actions.power_dependency_ids.clear();
    });
    Ok(())
}

mod input;
pub(super) use input::{
    advance_selection, dispatch_potions, dispatch_shortcuts, install_ranged, register_ranged, player_frame,
};
