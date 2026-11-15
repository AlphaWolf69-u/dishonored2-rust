//! Selected-ammo entity requests through the existing equipment Sources and
//! material worker. Old draw/source leases retire through PlayerArms' fence.
use crate::{
    assets::equipment::{
        Descriptor as Weapon,
        attachment::{Animation, Descriptor, Upgrade},
    },
    gameplay::{
        entity_lifetime::{EntityId, LiveEntities, Stage},
        player::equipment_attachment::{Binding, Selection},
        reference_frame::ReferenceFrame,
    },
};
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Pending {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<Arc<Descriptor>, String>>>,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    weapon: EntityId,
    slot: usize,
}
impl Key {
    fn ammo(weapon: EntityId) -> Self {
        Self { weapon, slot: 0 }
    }
}
struct UpgradePending {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<Vec<Arc<Upgrade>>, String>>>,
}
struct Upgrades {
    paths: Vec<String>,
    pending: Option<UpgradePending>,
    definitions: Option<Vec<Arc<Upgrade>>>,
    failed: Option<String>,
    installed: bool,
}
struct Entry {
    weapon: Arc<Weapon>,
    ammunition: String,
    path: String,
    selection: Selection,
    binding: Option<Binding>,
    explicit_identity: bool,
    animation: Animation,
    visible: bool,
    child: Option<EntityId>,
    descriptor: Option<Arc<Descriptor>>,
    pending: Option<Pending>,
    failed: Option<String>,
    material_requested: bool,
}
#[derive(Resource, Default)]
struct Attachments {
    live: HashMap<Key, Entry>,
    upgrades: HashMap<EntityId, Upgrades>,
    upgrade_weapons: HashMap<EntityId, Arc<Weapon>>,
    magazine_final: HashMap<EntityId, (Arc<crate::gameplay::player::weapons::Ranged>, i32)>,
    cancelled_upgrades: Vec<UpgradePending>,
    cancelled: Vec<Pending>,
    closing: bool,
    // Shared handles to the one source owner, captured while the setup batch
    // is fully published. Equipment callbacks temporarily borrow PlayerActions
    // out of World; they must not create a second cache to acquire a child.
    source: Option<(
        Arc<crate::archive::Catalog>,
        Arc<Mutex<crate::assets::equipment::Sources>>,
    )>,
}

/// Root setup invokes before LevelLifetime records its resource IDs.
pub(super) fn initialize(world: &mut World) {
    if world.contains_resource::<Attachments>() {
        return;
    }
    let source = world
        .get_resource::<super::player_actions::PlayerActions>()
        .map(|actions| actions.equipment_attachment_source());
    world.insert_resource(Attachments {
        source,
        ..Default::default()
    });
}

/// Called by the native selected-ammunition change, never for every inventory
/// ammo variant. The caller supplies the real declaration and bind tag.
pub(super) fn select(
    world: &mut World,
    weapon_id: EntityId,
    weapon: Arc<Weapon>,
    ammunition: &str,
    attachment: Option<&str>,
    tag: Option<&str>,
    visible: bool,
) -> Result<(), String> {
    let mut owner = world
        .remove_resource::<Attachments>()
        .ok_or("ammunition owner was not initialized during level publication")?;
    let result = (|| {
        if owner.closing {
            return Err("ammunition acquisition is closed".into());
        }
        let path = attachment.filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL"));
        if let Some(entry) = owner.live.get_mut(&Key::ammo(weapon_id)) {
            if entry.ammunition == ammunition && path == Some(entry.path.as_str()) {
                entry.visible = visible;
                if let Some(child) = entry.child {
                    if let Some(mut arms) =
                        world.get_resource_mut::<super::player_arms::PlayerArms>()
                    {
                        if arms.attachments.has(child) {
                            arms.attachments.set_visible(child, visible)?;
                        }
                    }
                }
                return Ok(());
            }
        }
        if let Some(mut old) = owner.live.remove(&Key::ammo(weapon_id)) {
            if let Err(error) = withdraw(world, &mut old, &mut owner.cancelled) {
                owner.live.insert(Key::ammo(weapon_id), old);
                return Err(error);
            }
        }
        let Some(path) = path else {
            return Ok(());
        };
        let tag = tag.ok_or("selected ammunition has no authored weapon bind tag")?;
        // Validate while this immutable rig is borrowed, before any worker can
        // reserve/publish child draw entities. There is no fallback socket.
        let selection = Selection::read(&weapon.rig, tag)?;
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("ammunition has no managed owner")?;
        if live.stage(weapon_id) != Some(Stage::Spawned) {
            return Err("ammunition weapon generation is stale".into());
        }
        let (catalog, sources) = owner
            .source
            .clone()
            .ok_or("ammunition has no canonical equipment sources")?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        let requested = path.to_owned();
        let task = IoTaskPool::get().spawn(async move {
            let stopped = || cancellation.load(Ordering::Acquire);
            let mut sources = sources
                .lock()
                .map_err(|_| "equipment source owner is poisoned")?;
            sources.acquire_attachment(&catalog, &requested, Some(&stopped))
        });
        owner.live.insert(
            Key::ammo(weapon_id),
            Entry {
                weapon,
                ammunition: ammunition.to_owned(),
                path: path.to_owned(),
                selection,
                binding: None,
                explicit_identity: false,
                animation: Animation::Static,
                visible,
                child: None,
                descriptor: None,
                pending: Some(Pending {
                    cancelled,
                    task: Mutex::new(task),
                }),
                failed: None,
                material_requested: false,
            },
        );
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

fn completed(pending: &mut Pending) -> Result<Option<Result<Arc<Descriptor>, String>>, String> {
    let mut task = pending
        .task
        .lock()
        .map_err(|_| "ammunition acquisition task is poisoned")?;
    Ok(block_on(poll_once(&mut *task)))
}

pub(super) fn advance(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<Attachments>() else {
        return Ok(());
    };
    let result = (|| {
        let mut index = 0;
        while index < owner.cancelled.len() {
            if completed(&mut owner.cancelled[index])?.is_some() {
                owner.cancelled.swap_remove(index);
            } else {
                index += 1;
            }
        }
        advance_upgrades(world, &mut owner)?;
        for (key, entry) in &mut owner.live {
            let weapon = key.weapon;
            if let Some(error) = &entry.failed {
                return Err(error.clone());
            }
            if let Some(pending) = entry.pending.as_mut() {
                if let Some(result) = completed(pending)? {
                    entry.pending = None;
                    match result {
                        Ok(descriptor) => entry.descriptor = Some(descriptor),
                        Err(error) => {
                            entry.failed = Some(error.clone());
                            return Err(error);
                        }
                    }
                }
            }
            let Some(descriptor) = entry.descriptor.as_ref() else {
                continue;
            };
            if entry.binding.is_none() {
                let Some((parent, _)) = super::equipment_identity::frame(world, weapon) else {
                    continue;
                };
                let joint = entry
                    .selection
                    .joint()
                    .and_then(|joint| {
                        world
                            .get_resource::<super::player::PlayerState>()
                            .and_then(|state| state.player.equipment_joint_world(weapon, joint))
                    })
                    .map(frame);
                entry.binding =
                    entry
                        .selection
                        .bind(entry.explicit_identity, parent, joint, parent);
                if entry.binding.is_none() {
                    continue;
                }
            }
            let child = match entry.child {
                Some(child) => child,
                None => {
                    let child = super::equipment_identity::admit_attachment(
                        world,
                        &format!("{}:{}", entry.ammunition, key.slot),
                        descriptor,
                        weapon,
                    )?;
                    entry.child = Some(child);
                    child
                }
            };
            if let Some(error) =
                super::runtime_materials::equipment_admission::attachment_failure(world, child)
            {
                return Err(error);
            }
            if !entry.material_requested {
                super::runtime_materials::equipment_admission::request_attachment(
                    world,
                    child,
                    descriptor.clone(),
                )?;
                entry.material_requested = true;
            }
        }
        advance_magazine_final(world, &mut owner)?;
        super::equipment_identity::poll_retirements(world)?;
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

/// Shared material worker calls before applying its assets/command batch.
/// Full managed generations and immutable source identity reject stale work.
pub(super) fn validate(
    world: &World,
    child: EntityId,
    prepared: &super::player_arms::attachment::PreparedModel,
) -> Result<(), String> {
    let owner = world
        .get_resource::<Attachments>()
        .ok_or("ammunition owner was retired")?;
    let (key, entry) = owner
        .live
        .iter()
        .find(|(_, entry)| entry.child == Some(child))
        .ok_or("prepared ammunition request was cancelled")?;
    if owner.closing {
        return Err("prepared ammunition belongs to a closing level".into());
    }
    let descriptor = entry
        .descriptor
        .as_ref()
        .ok_or("prepared ammunition source is absent")?;
    if !Arc::ptr_eq(descriptor, prepared.descriptor()) {
        return Err("prepared ammunition source generation changed".into());
    }
    if entry.binding.is_none() {
        return Err("prepared child has no actual parent binding".into());
    }
    let live = world
        .get_resource::<LiveEntities>()
        .ok_or("prepared ammunition managed owner is absent")?;
    if live.stage(child) != Some(Stage::Spawned) || live.parent(child)? != Some(key.weapon) {
        return Err("prepared ammunition identity/binding is stale".into());
    }
    if world
        .get_resource::<super::player_arms::PlayerArms>()
        .is_none_or(|arms| arms.attachments.has(child))
    {
        return Err("prepared ammunition renderer is absent or already published".into());
    }
    Ok(())
}

/// Called in the same exclusive publication after validate and the real image
/// gate. No extra acknowledgement or hidden replacement draw is generated.
pub(super) fn publish(
    world: &mut World,
    child: EntityId,
    prepared: super::player_arms::attachment::PreparedModel,
) -> Result<(), String> {
    validate(world, child, &prepared)?;
    let owner = world.resource::<Attachments>();
    let (key, entry) = owner
        .live
        .iter()
        .find(|(_, entry)| entry.child == Some(child))
        .ok_or("ammunition request disappeared")?;
    let weapon = entry.weapon.clone();
    let binding = entry.binding.ok_or("prepared child binding disappeared")?;
    let parent = key.weapon;
    let visible = entry.visible;
    world
        .resource_mut::<super::player_arms::PlayerArms>()
        .attachments
        .publish(child, parent, weapon, binding, prepared, visible)?;
    super::equipment_identity::mark_rendered(world, child)
}

pub(super) fn set_visible(
    world: &mut World,
    weapon: EntityId,
    visible: bool,
) -> Result<(), String> {
    let Some(mut owner) = world.get_resource_mut::<Attachments>() else {
        return Ok(());
    };
    let Some(entry) = owner.live.get_mut(&Key::ammo(weapon)) else {
        return Ok(());
    };
    entry.visible = visible;
    let child = entry.child;
    drop(owner);
    if let Some(child) = child {
        if let Some(mut arms) = world.get_resource_mut::<super::player_arms::PlayerArms>() {
            if arms.attachments.has(child) {
                arms.attachments.set_visible(child, visible)?;
            }
        }
    }
    Ok(())
}

pub(super) fn visible(world: &World, weapon: EntityId) -> bool {
    let Some(child) = world
        .get_resource::<Attachments>()
        .and_then(|owner| owner.live.get(&Key::ammo(weapon)))
        .and_then(|entry| entry.child)
    else {
        return false;
    };
    world
        .get_resource::<super::player_arms::PlayerArms>()
        .is_some_and(|arms| arms.attachments.visible(child))
}

/// Startup observes the complete selected-child path, including source IO
/// before it reaches the shared material worker. No animation completion is
/// required to enter gameplay.
pub(super) fn startup_pending(world: &World) -> bool {
    let Some(owner) = world.get_resource::<Attachments>() else {
        return false;
    };
    !owner.cancelled_upgrades.is_empty()
        || owner
            .upgrades
            .values()
            .any(|upgrade| upgrade.pending.is_some() || !upgrade.installed)
        || !owner.cancelled.is_empty()
        || owner.live.values().any(|entry| {
            entry.pending.is_some()
                || entry.child.is_none()
                || entry.child.is_some_and(|id| {
                    world
                        .get_resource::<super::player_arms::PlayerArms>()
                        .is_none_or(|arms| !arms.attachments.has(id))
                })
        })
}

pub(super) fn startup_error(world: &World) -> Option<String> {
    let owner = world.get_resource::<Attachments>()?;
    owner
        .upgrades
        .values()
        .find_map(|upgrade| upgrade.failed.clone())
        .or_else(|| {
            owner.live.values().find_map(|entry| {
                entry.failed.clone().or_else(|| {
                    entry.child.and_then(|id| {
                        super::runtime_materials::equipment_admission::attachment_failure(world, id)
                    })
                })
            })
        })
}

/// The caller passes the current player animation owner's actual tag frame.
/// This is also used before the Fire callback; a previous render frame or a
/// camera/weapon-origin fallback is never used as the loaded bolt's position.
pub(super) fn current_frame(
    world: &World,
    weapon: EntityId,
    tag_frame: Option<(Vec3, Mat3)>,
) -> Option<ReferenceFrame> {
    if !visible(world, weapon) {
        return None;
    }
    if let Some(state) = world.get_resource::<super::player::PlayerState>() {
        return current_frame_for_player(world, weapon, &state.player);
    }
    let entry = world
        .get_resource::<Attachments>()?
        .live
        .get(&Key::ammo(weapon))?;
    match entry.selection {
        // The caller already resolved this authored model tag this frame.
        Selection::Tag { .. } => tag_frame.map(frame),
        Selection::Entity => {
            let (parent, _) = super::equipment_identity::frame(world, weapon)?;
            entry.binding?.world(parent, None)
        }
        // A scoped caller must supply its actual Player to the explicit API
        // for bone-only bindings. Previous-frame render poses are not used.
        Selection::Bone { .. } => None,
    }
}

fn withdraw(
    world: &mut World,
    entry: &mut Entry,
    cancelled: &mut Vec<Pending>,
) -> Result<(), String> {
    if let Some(pending) = entry.pending.take() {
        pending.cancelled.store(true, Ordering::Release);
        cancelled.push(pending);
    }
    if let Some(child) = entry.child {
        super::runtime_materials::equipment_admission::withdraw_attachment(world, child);
        let mut arms = world
            .remove_resource::<super::player_arms::PlayerArms>()
            .ok_or("ammunition retirement lost its renderer")?;
        let mut commands = super::setup_commands::SetupCommands::prepared(world);
        let receipt = arms.attachments.remove(child, &mut commands);
        if let Some(receipt) = &receipt {
            arms.retiring_equipment.push(receipt.clone());
        }
        commands.publish(world);
        world.insert_resource(arms);
        match receipt {
            Some(receipt) => {
                super::equipment_identity::await_render_retirement(world, child, receipt)?
            }
            None => {
                // This child never published any render model. Its cancelled
                // prepared batches remain with the shared material worker.
                super::equipment_identity::await_unrendered_retirement(world, child)?;
            }
        }
        entry.child = None;
    }
    entry.descriptor = None;
    Ok(())
}

pub(super) fn remove(world: &mut World, weapon: EntityId) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<Attachments>() else {
        return Ok(());
    };
    let result = (|| {
        owner.magazine_final.remove(&weapon);
        owner.upgrade_weapons.remove(&weapon);
        if let Some(mut upgrades) = owner.upgrades.remove(&weapon) {
            if let Some(job) = upgrades.pending.take() {
                job.cancelled.store(true, Ordering::Release);
                owner.cancelled_upgrades.push(job);
            }
        }
        let keys = owner
            .live
            .keys()
            .filter(|key| key.weapon == weapon)
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            let mut entry = owner
                .live
                .remove(&key)
                .ok_or("child retirement entry disappeared")?;
            if let Err(error) = withdraw(world, &mut entry, &mut owner.cancelled) {
                owner.live.insert(key, entry);
                return Err(error);
            }
        }
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    let Some(mut owner) = world.remove_resource::<Attachments>() else {
        return Ok(true);
    };
    owner.closing = true;
    let result = (|| {
        for (_, mut upgrade) in owner.upgrades.drain() {
            if let Some(job) = upgrade.pending.take() {
                job.cancelled.store(true, Ordering::Release);
                owner.cancelled_upgrades.push(job);
            }
        }
        let weapons = owner.live.keys().copied().collect::<Vec<_>>();
        for weapon in weapons {
            let entry = owner
                .live
                .get_mut(&weapon)
                .ok_or("ammunition shutdown entry disappeared")?;
            withdraw(world, entry, &mut owner.cancelled)?;
        }
        owner.live.clear();
        owner.upgrade_weapons.clear();
        owner.magazine_final.clear();
        let mut index = 0;
        while index < owner.cancelled.len() {
            if completed(&mut owner.cancelled[index])?.is_some() {
                owner.cancelled.swap_remove(index);
            } else {
                index += 1;
            }
        }
        poll_cancelled_upgrades(&mut owner.cancelled_upgrades)?;
        Ok(owner.cancelled.is_empty()
            && owner.cancelled_upgrades.is_empty()
            && super::equipment_identity::poll_retirements(world)?)
    })();
    if !matches!(result, Ok(true)) {
        world.insert_resource(owner);
    }
    result
}

fn frame((position, rotation): (Vec3, Mat3)) -> ReferenceFrame {
    ReferenceFrame {
        position: position.to_array(),
        basis: rotation.to_cols_array_2d(),
    }
}
fn poll_cancelled_upgrades(jobs: &mut Vec<UpgradePending>) -> Result<(), String> {
    let mut index = 0;
    while index < jobs.len() {
        let done = {
            let mut task = jobs[index]
                .task
                .lock()
                .map_err(|_| "upgrade source task is poisoned")?;
            block_on(poll_once(&mut *task)).is_some()
        };
        if done {
            jobs.swap_remove(index);
        } else {
            index += 1;
        }
    }
    Ok(())
}

/// 61a2b0 builds the authored default-upgrade vector once during the real
/// owned actor Start. Both child channels survive holster and retire on depletion.
pub(super) fn default_upgrades(
    world: &mut World,
    weapon: EntityId,
    descriptor: Arc<Weapon>,
    paths: &[String],
) -> Result<(), String> {
    let mut owner = world
        .remove_resource::<Attachments>()
        .ok_or("equipment child owner absent")?;
    let result = (|| {
        if owner.closing {
            return Err("equipment child admission closed".into());
        }
        if let Some(previous) = owner.upgrades.get(&weapon) {
            if previous.paths == paths {
                return Ok(());
            }
            return Err("default upgrade list changed without native upgrade replacement".into());
        }
        if paths.is_empty() {
            owner.upgrades.insert(
                weapon,
                Upgrades {
                    paths: Vec::new(),
                    pending: None,
                    definitions: Some(Vec::new()),
                    failed: None,
                    installed: true,
                },
            );
            return Ok(());
        }
        let (catalog, sources) = owner
            .source
            .clone()
            .ok_or("upgrade canonical source owner absent")?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        let requested = paths.to_vec();
        let paths = requested.clone();
        let task = IoTaskPool::get().spawn(async move {
            let stopped = || cancellation.load(Ordering::Acquire);
            let mut sources = sources
                .lock()
                .map_err(|_| "upgrade source owner poisoned")?;
            requested
                .iter()
                .map(|path| sources.acquire_upgrade(&catalog, path, Some(&stopped)))
                .collect()
        });
        owner.upgrades.insert(
            weapon,
            Upgrades {
                paths,
                pending: Some(UpgradePending {
                    cancelled,
                    task: Mutex::new(task),
                }),
                definitions: None,
                failed: None,
                installed: false,
            },
        );
        // Keep the immutable actual weapon descriptor while the source job is
        // pending; the child entries take the same lease at publication.
        owner.upgrade_weapons.insert(weapon, descriptor);
        Ok(())
    })();
    world.insert_resource(owner);
    result
}
fn advance_upgrades(world: &mut World, owner: &mut Attachments) -> Result<(), String> {
    poll_cancelled_upgrades(&mut owner.cancelled_upgrades)?;
    for (&weapon, upgrades) in &mut owner.upgrades {
        if let Some(error) = &upgrades.failed {
            return Err(error.clone());
        }
        if let Some(pending) = upgrades.pending.as_mut() {
            let result = {
                let mut task = pending.task.lock().map_err(|_| "upgrade task poisoned")?;
                block_on(poll_once(&mut *task))
            };
            if let Some(result) = result {
                upgrades.pending = None;
                match result {
                    Ok(value) => upgrades.definitions = Some(value),
                    Err(error) => {
                        upgrades.failed = Some(error.clone());
                        return Err(error);
                    }
                }
            }
        }
        if upgrades.installed {
            continue;
        }
        let Some(definitions) = upgrades.definitions.as_ref() else {
            continue;
        };
        let descriptor = owner
            .upgrade_weapons
            .get(&weapon)
            .ok_or("upgrade lost actual parent descriptor")?;
        // Validate every binding before publishing any child, preserving native
        // vector and static/entity channel ordering.
        let selections = definitions
            .iter()
            .map(|upgrade| Selection::read(&descriptor.rig, &upgrade.tag))
            .collect::<Result<Vec<_>, _>>()?;
        for (index, upgrade) in definitions.iter().enumerate() {
            for (channel, child) in upgrade.attachments.iter().enumerate() {
                let Some(child) = child else { continue };
                owner.live.insert(
                    Key {
                        weapon,
                        slot: 1 + index * 2 + channel,
                    },
                    Entry {
                        weapon: descriptor.clone(),
                        ammunition: upgrade.declaration.clone(),
                        path: child.entity.clone(),
                        selection: selections[index],
                        binding: None,
                        explicit_identity: true,
                        animation: upgrade.animation,
                        visible: true,
                        child: None,
                        descriptor: Some(child.clone()),
                        pending: None,
                        failed: None,
                        material_requested: false,
                    },
                );
            }
        }
        upgrades.installed = true;
    }
    Ok(())
}
/// 61a650/61b4c0: only actual entitydef children with an animation component
/// receive the requested mode. Static model children are never synthetic rigs.
pub(super) fn animate_upgrade(
    world: &mut World,
    weapon: EntityId,
    mode: u8,
    play: bool,
) -> Result<(), String> {
    world.resource_scope(|world, mut arms: Mut<super::player_arms::PlayerArms>| {
        let Some(owner) = world.get_resource::<Attachments>() else {
            return Ok(());
        };
        for (key, entry) in &owner.live {
            if key.weapon == weapon && key.slot != 0 && entry.animation as u8 == mode {
                if let Some(id) = entry.child {
                    arms.attachments.animate(id, play, mode == 1 || mode == 2)?;
                }
            }
        }
        Ok(())
    })
}
/// 639410(param,5) chooses the first authored upgrade, not the first ready
/// child. Returning false retains44f240's pending magazine command.
pub(super) fn animate_magazine(
    world: &mut World,
    weapon: EntityId,
    alias: &str,
    mode: crate::gameplay::player::animation::PlayMode,
) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<Attachments>() else {
        return Ok(false);
    };
    let Some(upgrades) = owner.upgrades.get(&weapon) else {
        return Ok(false);
    };
    let Some(definitions) = upgrades.definitions.as_ref() else {
        return Ok(false);
    };
    let Some(index) = definitions
        .iter()
        .position(|upgrade| upgrade.animation == Animation::Magazine)
    else {
        return Ok(true);
    };
    let child = owner
        .live
        .get(&Key {
            weapon,
            slot: 1 + index * 2 + 1,
        })
        .and_then(|entry| entry.child);
    let Some(child) = child else { return Ok(false) };
    let Some(mut arms) = world.get_resource_mut::<super::player_arms::PlayerArms>() else {
        return Ok(false);
    };
    arms.attachments.animate_alias(child, alias, mode)
}

/// Same-frame native fire consumer while PlayerState may be borrowed out of
/// World. Uses the actual managed equipment joint cache, never render history.
pub(super) fn current_frame_for_player(
    world: &World,
    weapon: EntityId,
    player: &crate::gameplay::player::Player,
) -> Option<ReferenceFrame> {
    if !visible(world, weapon) {
        return None;
    }
    let entry = world
        .get_resource::<Attachments>()?
        .live
        .get(&Key::ammo(weapon))?;
    let binding = entry.binding?;
    let (parent, _) = super::equipment_identity::frame(world, weapon)?;
    let joint = binding
        .joint
        .and_then(|joint| player.equipment_joint_world(weapon, joint))
        .map(frame);
    binding.world(parent, joint)
}

/// Actual44e720 capacity recomputation leaves44f240 pending until the first
/// type5 child's animation component is started. Do not call for every round.
pub(super) fn queue_magazine_final(
    world: &mut World,
    weapon: EntityId,
    ranged: Arc<crate::gameplay::player::weapons::Ranged>,
    magazine: i32,
) -> Result<(), String> {
    let mut owner = world
        .get_resource_mut::<Attachments>()
        .ok_or("equipment child owner absent")?;
    owner.magazine_final.insert(weapon, (ranged, magazine));
    Ok(())
}
fn magazine_child(owner: &Attachments, weapon: EntityId) -> Option<Option<EntityId>> {
    let upgrades = owner.upgrades.get(&weapon)?;
    let definitions = upgrades.definitions.as_ref()?;
    let Some(index) = definitions
        .iter()
        .position(|upgrade| upgrade.animation == Animation::Magazine)
    else {
        return Some(None);
    };
    let child = owner
        .live
        .get(&Key {
            weapon,
            slot: 1 + index * 2 + 1,
        })?
        .child?;
    Some(Some(child))
}
fn advance_magazine_final(world: &mut World, owner: &mut Attachments) -> Result<(), String> {
    let Some(mut arms) = world.get_resource_mut::<super::player_arms::PlayerArms>() else {
        return Ok(());
    };
    // No temporary Vec in active Think. Deferred commands are authored events,
    // not repeated requests allocated each frame.
    let live = &owner.live;
    let upgrades = &owner.upgrades;
    owner.magazine_final.retain(|weapon, (ranged, magazine)| {
        let Some(upgrades) = upgrades.get(weapon) else {
            return true;
        };
        let Some(definitions) = upgrades.definitions.as_ref() else {
            return true;
        };
        let Some(index) = definitions
            .iter()
            .position(|upgrade| upgrade.animation == Animation::Magazine)
        else {
            return false;
        };
        let Some(child) = live
            .get(&Key {
                weapon: *weapon,
                slot: 1 + index * 2 + 1,
            })
            .and_then(|entry| entry.child)
        else {
            return true;
        };
        if !arms.attachments.animation_ready(child) {
            return true;
        }
        if let Some((alias, mode)) = ranged.magazine_final_animation(*magazine) {
            if arms.attachments.has_alias(child, alias) {
                // Native44f240 ignores44e920's lookup result and clears the
                // pending flag once the real animation component is ready.
                let _ = arms.attachments.animate_alias(child, alias, mode);
            }
        }
        false
    });
    Ok(())
}
/// 44f3d0: the Fire animation callback changes only the real magazine child's
/// pose. Round consumption remains the weapon Runtime's independent callback.
pub(super) fn magazine_fire(
    world: &mut World,
    weapon: EntityId,
    ranged: &crate::gameplay::player::weapons::Ranged,
    magazine: i32,
) -> Result<(), String> {
    let Some((alias, mode)) = ranged.magazine_fire_animation(magazine) else {
        return Ok(());
    };
    match animate_magazine(world, weapon, alias, mode)? {
        true => Ok(()),
        false => Err("gun magazine Fire animation component is not started".into()),
    }
}
/// 44e7b0/44f470: try the authored reload alias, then its permitted fire-alias
/// fallback. Missing lookup is an original error, never a rest-pose substitute.
pub(super) fn magazine_reload(
    world: &mut World,
    weapon: EntityId,
    ranged: &crate::gameplay::player::weapons::Ranged,
    capacity: i32,
    total: u32,
) -> Result<(), String> {
    let Some(owner) = world.get_resource::<Attachments>() else {
        return Ok(());
    };
    let Some(child) = magazine_child(owner, weapon) else {
        return Err("gun magazine Reload animation component is not started".into());
    };
    let Some(child) = child else { return Ok(()) };
    let mut arms = world.resource_mut::<super::player_arms::PlayerArms>();
    if !arms.attachments.animation_ready(child) {
        return Err("gun magazine Reload animation component is not started".into());
    }
    for (alias, mode) in ranged
        .magazine_reload_animations(capacity, total)
        .into_iter()
        .flatten()
    {
        if arms.attachments.has_alias(child, alias) {
            arms.attachments.animate_alias(child, alias, mode)?;
            return Ok(());
        }
    }
    Err("gun magazine Reload has no authored animation alias".into())
}
