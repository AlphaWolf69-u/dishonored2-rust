//! Equipment switches (835e00/8363f0/836f60), owning the exact source
//! request and its two animation phases. Decoding stays in asset owners;
//! accepted state callbacks alone change the equipped tree and inventory.
use super::{equipment::EquipmentTree, states::{Frame, StateId, StateMachine}, weapons::Equipment, Player};
use crate::entities::{scene_motion::atom, Value};
use std::{collections::{HashMap, VecDeque}, sync::{Arc, atomic::{AtomicU32, Ordering}}};

static NEXT_REQUEST: AtomicU32 = AtomicU32::new(1);

pub struct Target {
    pub identity: String,
    pub entity: String,
    pub equipment: Arc<Equipment>,
    pub tree: Option<EquipmentTree>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Declaration {
    pub slot: u8,
    pub power_switch: Option<String>,
    pub power_switch_slide: Option<String>,
    pub default_equipment: Option<String>,
    pub default_ammo: Option<String>,
}
impl Declaration {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let text = |name| atom(fields, name).filter(|value| !value.is_empty() && *value != "NULL").map(str::to_owned);
        let slot = atom(fields, "m_slot").map_or(Ok(0), |value|
            crate::assets::equipment::enumeration("arkEquipmentSlot_t", value)
                .or_else(|| value.parse().ok()).ok_or_else(|| format!("unknown equipment slot {value}")))?;
        if !(0..16).contains(&slot) { return Err("equipment switch slot is outside inventory".into()); }
        Ok(Self { slot: slot as u8, power_switch: text("m_powerSwitchMidnight"),
            power_switch_slide: text("m_powerSwitchMidnightSlide"),
            default_equipment: text("m_defaultEquipment"), default_ammo: text("m_defaultAmmo") })
    }
    pub fn state(&self) -> StateId { match self.slot { 1 => StateId::EQUIP_LEFT, 4 => StateId(80), _ => StateId::EQUIP_RIGHT } }
    pub fn side(&self) -> Option<usize> { match self.slot { 1 => Some(0), 0 => Some(1), _ => None } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callback {
    Equipped { identity: String, side: usize },
    Unequipped { identity: String, side: usize },
    Cancelled { identity: String, side: usize },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase { #[default] Idle, Unequip, Detach, Attach, Equip, Equipping }

#[derive(Default)]
struct Slot {
    current: Option<Target>,
    incoming: Option<Target>,
    equipped: bool,
    requested: bool,
    restore: bool,
    reselect: bool,
    phase: Phase,
    handle: u32,
    flags: u32,
    uid: u32,
    /// 836920: one counted request per source state, except anonymous90.
    restrictions: Vec<StateId>,
    anonymous: i32,
}

#[derive(Default)]
pub struct Runtime {
    slots: [Slot; 2],
    /// Actual still-owned equipment entities keep playback state across hand
    /// selection, 619380/619640; quantity-zero teardown alone destroys them.
    owned: HashMap<String, Target>,
    callbacks: VecDeque<Callback>,
}
impl Runtime {
    /// 839a10 ->6145f0 applies the actual equipment entity's own Hide/Show
    /// callback. It does not select, stop or reconstruct its component.
    pub(super) fn set_owned_hidden(&mut self, hidden: bool) {
        for target in self.owned.values_mut().chain(self.slots.iter_mut().flat_map(|slot|
            slot.current.iter_mut().chain(slot.incoming.iter_mut()))) {
            if let Some(tree) = &mut target.tree { tree.visible = !hidden; }
        }
    }

    /// Actual retained entities whose tree is not currently borrowed by Body.
    /// Audio/frame publication borrows this registry without building a list.
    pub fn inactive_targets(&self) -> impl Iterator<Item = &Target> {
        self.owned.values().chain(self.slots.iter().flat_map(|slot|
            slot.current.iter().chain(slot.incoming.iter())))
            .filter(|target| target.tree.is_some())
    }
    pub fn transitioning(&self, side: usize) -> bool {
        self.slots.get(side).is_some_and(|slot| slot.requested || slot.phase != Phase::Idle)
    }
    pub fn has_request(&self, side: usize) -> bool { self.transitioning(side) }
    pub fn requests_other_hand(&self, side: usize) -> bool {
        self.slots.get(side).is_some_and(|slot| slot.requested
            && slot.flags & 0x104 == 0 && (slot.incoming.is_some() || slot.restore)
            && slot.anonymous + (slot.restrictions.len() as i32) < 1)
    }
    pub fn equipped(&self, side: usize) -> bool { self.slots.get(side).is_some_and(|slot| slot.equipped) }
    pub fn preferred(&self, side: usize) -> Option<&Equipment> {
        self.slots.get(side)?.current.as_ref().map(|target| &*target.equipment)
    }
    pub(super) fn preferred_identity(&self, side: usize) -> Option<&str> {
        self.slots.get(side)?.current.as_ref().map(|target| target.identity.as_str())
    }
    pub fn instant_pending(&self, side: usize) -> bool {
        self.slots.get(side).is_some_and(|slot| slot.requested && slot.flags & 0x30 == 0x30
            && slot.phase == Phase::Idle && (slot.incoming.is_some() || slot.restore)
            && slot.anonymous + (slot.restrictions.len() as i32) < 1)
    }
    pub(super) fn begin_instant(&mut self, side: usize) {
        let slot = &mut self.slots[side];
        slot.requested = false;
        slot.handle = 0;
        slot.phase = if slot.equipped { Phase::Detach } else { Phase::Attach };
    }
    pub fn request(&mut self, target: Target, flags: u32, source: StateId) -> Result<u32, String> {
        let side = match target.equipment.slot { 1 => 0, 0 => 1, slot => return Err(format!("equipment slot {slot} has no player arm")) };
        let target = self.owned.remove(&target.identity).unwrap_or(target);
        let slot = &mut self.slots[side];
        if slot.current.as_ref().is_some_and(|current| current.identity == target.identity) {
            if let Some(previous) = slot.incoming.take() {
                self.callbacks.push_back(Callback::Cancelled { identity: previous.identity.clone(), side });
                self.owned.insert(previous.identity.clone(), previous);
            }
            slot.restore = true;
        } else {
            if let Some(previous) = slot.incoming.replace(target) {
                self.callbacks.push_back(Callback::Cancelled { identity: previous.identity.clone(), side });
                self.owned.insert(previous.identity.clone(), previous);
            }
        }
        Self::request_flags(slot, flags, source);
        slot.reselect = true;
        Ok(slot.uid)
    }
    pub fn request_stored(&mut self, side: usize, flags: u32, source: StateId) {
        let slot = &mut self.slots[side];
        Self::request_flags(slot, flags, source);
        // 8363f0 uses the controller's remembered equipment for restore flags.
        slot.restore = flags & 0xc != 0;
        slot.reselect = false;
    }
    fn request_flags(slot: &mut Slot, flags: u32, source: StateId) {
        if flags & 1 != 0 {
            if source.0 == 90 { slot.anonymous += 1; }
            else if !slot.restrictions.contains(&source) { slot.restrictions.push(source); }
        }
        if flags & 2 != 0 {
            if source.0 == 90 { slot.anonymous -= 1; }
            else if let Some(index) = slot.restrictions.iter().position(|owner| *owner == source) { slot.restrictions.swap_remove(index); }
        }
        slot.flags = flags;
        slot.uid = NEXT_REQUEST.fetch_update(Ordering::Relaxed, Ordering::Relaxed,
            |value| Some((value.wrapping_add(1) & 0x7fff_ffff).max(1))).unwrap_or(1);
        slot.requested = true;
    }
    pub fn take_callbacks(&mut self) -> Vec<Callback> { self.callbacks.drain(..).collect() }
}

/// Decisions run on the already admitted state. The outer Player applies
/// Detach/Attach between frame borrows, then starts the incoming scene in the
/// same simulation frame with the new entity's actual rig and clip bank.
pub(super) fn advance(decl: &Declaration, sneaking: bool, sliding: bool,
    active: &[StateId], frame: &mut Frame<'_>) -> Result<bool, String> {
    let Some(side) = decl.side() else { return Ok(true) };
    let slot = &mut frame.equipment_changes.slots[side];
    slot.restrictions.retain(|owner| active.contains(owner));
    // 835e00/836f60 can interrupt an outgoing unequip, but never an
    // already running incoming equip. Block44 is the explicit exception.
    if slot.requested && slot.phase == Phase::Detach && slot.handle != 0
        && (slot.flags & 0x20 != 0 || active.contains(&StateId(44))) {
        frame.midnight.stop(slot.handle);
        slot.handle = 0;
    }
    if slot.phase == Phase::Idle && slot.requested {
        slot.requested = false;
        let permitted = slot.anonymous + (slot.restrictions.len() as i32) < 1;
        if !permitted && slot.incoming.is_some() {
            // Remember the desired equipment while another actual state owns
            // the hand, as the controller's 7fb000/7fb040 references do.
            return Ok(true);
        }
        // A restore request resolves the controller's suspended selection,
        // not an instruction to holster an already present unchanged hand.
        if permitted && slot.equipped && slot.incoming.is_none() && slot.restore && !slot.reselect {
            slot.restore = false;
            return Ok(true);
        }
        if slot.equipped { slot.phase = Phase::Unequip; }
        else if permitted && (slot.incoming.is_some() || slot.restore) { slot.phase = Phase::Attach; }
        else { return Ok(true); }
    }
    if slot.phase == Phase::Unequip {
        let path = slot.current.as_ref().map(|target| target.equipment.unequip_scene(slot.flags, sneaking))
            .transpose()?.flatten().map(str::to_owned);
        if let Some(path) = path {
            let handle = start(&path, side, frame)?;
            let slot = &mut frame.equipment_changes.slots[side];
            slot.handle = handle;
            slot.phase = Phase::Detach;
            return Ok(false);
        }
        slot.phase = Phase::Detach;
    }
    let slot = &mut frame.equipment_changes.slots[side];
    if slot.phase == Phase::Detach && slot.handle != 0 && !frame.midnight.finished(slot.handle) { return Ok(false); }
    if matches!(slot.phase, Phase::Detach | Phase::Attach) { return Ok(false); }
    if slot.phase == Phase::Equip {
        let path = slot.current.as_ref().map(|target| {
            if target.equipment.archetype == 5 {
                if slot.flags & 0x10 != 0 { return Ok(None); }
                Ok(if sliding { decl.power_switch_slide.as_ref().or(decl.power_switch.as_ref()) } else { decl.power_switch.as_ref() }.map(String::as_str))
            } else { target.equipment.equip_scene(slot.flags, sneaking) }
        }).transpose()?.flatten().map(str::to_owned);
        if let Some(path) = path {
            let handle = start(&path, side, frame)?;
            let slot = &mut frame.equipment_changes.slots[side];
            slot.handle = handle;
            slot.phase = Phase::Equipping;
        } else { frame.equipment_changes.slots[side].phase = Phase::Idle; }
    }
    let slot = &mut frame.equipment_changes.slots[side];
    if slot.phase == Phase::Equipping && frame.midnight.finished(slot.handle) {
        slot.handle = 0;
        slot.phase = Phase::Idle;
    }
    Ok(slot.phase == Phase::Idle && !slot.requested)
}

fn start(path: &str, side: usize, frame: &mut Frame<'_>) -> Result<u32, String> {
    let scene = frame.scenes.get(path).ok_or_else(|| format!("equipment scene was not acquired: {path}"))?;
    // 836ad0: actual arm and its equipment, independently of the other hand.
    frame.midnight.start_with_parts(scene, frame.md6, 1.0, frame.animations, if side == 0 { 0xa } else { 0x14 }, frame.random)
}

impl Player {
    pub fn set_owned_equipment_hidden(&mut self, hidden: bool) {
        self.equipment_changes.set_owned_hidden(hidden);
        for tree in self.body.equipment.iter_mut().flatten() { tree.visible = !hidden; }
    }

    /// 7fb350 tests controller+2d0, the swimming controller(+258)+78,
    /// not possession/cinematic state. 88a8c0/88a7a0 use the same1..3 range
    /// for the active swimming mode represented by this player owner.
    pub fn shortcut_equipment_flags(&self) -> u32 {
        if self.swimming_active { 0x30 } else { 0x20 }
    }
    /// 619380 creates one runtime per admitted inventory equipment entity.
    /// Registration alone neither selects nor advances an equipment action.
    pub fn register_equipment_target(&mut self, mut target: Target) -> Result<(), String> {
        if target.equipment.slot > 1 { return Err("equipment target has no first-person arm".into()); }
        if self.equipment_changes.slots.iter().any(|slot| slot.current.as_ref()
            .into_iter().chain(slot.incoming.as_ref()).any(|known| known.identity == target.identity)) { return Ok(()); }
        if let Some(tree) = &mut target.tree {
            tree.bind_equipped(false);
            tree.visible = !target.equipment.hide_unequipped;
        }
        self.equipment_changes.owned.entry(target.identity.clone()).or_insert(target);
        Ok(())
    }
    pub fn has_equipment_target(&self, identity: &str) -> bool {
        self.equipment_changes.owned.contains_key(identity) || self.equipment_changes.slots.iter()
            .any(|slot| slot.current.as_ref().into_iter().chain(slot.incoming.as_ref()).any(|target| target.identity == identity))
    }
    /// Borrow the actual owned entity for wheel presentation. No declaration
    ///acquisition or playback copy is needed when opening the menu.
    pub fn equipment_target_metadata(&self, identity: &str) -> Option<(&Equipment, Option<&super::weapons::Ranged>, Option<&str>)> {
        let target = self.equipment_changes.owned.get(identity).or_else(||
            self.equipment_changes.slots.iter().flat_map(|slot|
                slot.current.iter().chain(slot.incoming.iter()))
                .find(|target| target.identity == identity))?;
        let ranged = self.registered_ranged(identity);
        Some((&target.equipment, ranged.map(|runtime| &*runtime.settings),
            ranged.map(|runtime| runtime.ammunition().item.as_str())))
    }
    pub fn request_equipment_selection_by_identity(&mut self, identity: &str, flags: u32) -> Result<u32, String> {
        if let Some(target) = self.equipment_changes.owned.remove(identity) {
            return self.request_equipment_selection(target, flags);
        }
        let side = self.equipment_changes.slots.iter().position(|slot| slot.current.as_ref()
            .is_some_and(|target| target.identity == identity)).ok_or_else(|| format!("equipment entity is not acquired: {identity}"))?;
        let slot = &mut self.equipment_changes.slots[side];
        Runtime::request_flags(slot, flags, StateId::NULL);
        slot.restore = true;
        slot.reselect = true;
        self.machine.request(if side == 0 { StateId::EQUIP_LEFT } else { StateId::EQUIP_RIGHT });
        Ok(slot.uid)
    }
    pub fn request_equipment_change(&mut self, mut target: Target, flags: u32, source: StateId) -> Result<u32, String> {
        let id = match target.equipment.slot { 1 => StateId::EQUIP_LEFT, 0 => StateId::EQUIP_RIGHT,
            slot => return Err(format!("equipment slot {slot} is not an arm")) };
        if self.machine.state(id).is_none() { return Err(format!("player mode lacks equipment state {}", id.0)); }
        if let Some(tree) = &mut target.tree {
            tree.bind_equipped(false);
            tree.visible = !target.equipment.hide_unequipped;
        }
        let uid = self.equipment_changes.request(target, flags, source)?;
        self.machine.request(id);
        Ok(uid)
    }
    /// 7f8d60 passes the no-state requester91, not a locomotion state.
    pub fn request_equipment_selection(&mut self, target: Target, flags: u32) -> Result<u32, String> {
        self.request_equipment_change(target, flags, StateId::NULL)
    }
    pub fn take_equipment_callbacks(&mut self) -> Vec<Callback> { self.equipment_changes.take_callbacks() }

    /// 619640: depletion destroys the actual source entity, independently of
    /// whether it is selected. Geometry retirement remains the renderer owner.
    pub fn retire_equipment(&mut self, identity: &str) {
        self.equipment_changes.owned.remove(identity);
        for side in 0..2 {
            let slot = &mut self.equipment_changes.slots[side];
            if slot.incoming.as_ref().is_some_and(|target| target.identity == identity) {
                slot.incoming = None;
            }
            if slot.current.as_ref().is_some_and(|target| target.identity == identity) {
                self.body.replace_equipment_tree(side, None);
                self.equipment_archetypes[side] = None;
                slot.current = None;
                slot.equipped = false;
                slot.phase = if slot.incoming.is_some() { Phase::Attach } else { Phase::Idle };
                slot.handle = 0;
                slot.requested = slot.incoming.is_some();
            }
        }
    }

    /// bevy_backend: presentation pauses the game clock during loading, while
    /// 7f8d60's actual instant request still needs its zero-duration spawn
    /// callbacks. Process only explicitly skipped equip+unequip requests; no
    /// physics, input, animation clock or unrelated player state advances.
    pub fn advance_equipment_selection(&mut self, _world: &dyn super::character_proxy::PlayerWorld) {
        self.machine.admit_instant_equipment(&mut self.equipment_changes);
        self.apply_equipment_changes();
        for slot in &mut self.equipment_changes.slots {
            if slot.phase == Phase::Equip && slot.flags & 0x10 != 0 {
                slot.phase = Phase::Idle;
            }
        }
    }

    pub(super) fn apply_equipment_changes(&mut self) {
        for side in 0..2 {
            let slot = &mut self.equipment_changes.slots[side];
            // 836f60 ->770840/770860 resets the owning arm's locomotion to
            // its default Empty equipment while the outgoing scene plays.
            if slot.phase == Phase::Detach {
                if let Some(arm) = self.body.arms.get_mut(side) { arm.request_equipment("Empty"); }
            }
            if slot.phase == Phase::Detach && (slot.handle == 0 || self.body.midnight.finished(slot.handle)) {
                if let Some(current) = slot.current.as_mut() {
                    current.tree = self.body.replace_equipment_tree(side, None);
                    if let Some(tree) = &mut current.tree {
                        tree.bind_equipped(false);
                        tree.visible = !current.equipment.hide_unequipped;
                    }
                    self.equipment_changes.callbacks.push_back(Callback::Unequipped { identity: current.identity.clone(), side });
                }
                slot.equipped = false;
                self.equipment_archetypes[side] = None;
                slot.handle = 0;
                slot.phase = if slot.incoming.is_some() || slot.restore { Phase::Attach } else { Phase::Idle };
            }
            if slot.phase == Phase::Attach {
                if let Some(incoming) = slot.incoming.take() {
                    if let Some(previous) = slot.current.replace(incoming) {
                        if previous.identity != slot.current.as_ref().unwrap().identity {
                            self.equipment_changes.callbacks.push_back(Callback::Cancelled { identity: previous.identity.clone(), side });
                            self.equipment_changes.owned.insert(previous.identity.clone(), previous);
                        }
                    }
                }
                let Some(target) = slot.current.as_mut() else { slot.phase = Phase::Idle; continue };
                if let Some(tree) = &mut target.tree {
                    tree.bind_equipped(true);
                    tree.visible = true;
                }
                self.body.replace_equipment_tree(side, target.tree.take());
                if let Some(name) = target.equipment.loco_equip_state.as_deref() {
                    if let Some(arm) = self.body.arms.get_mut(side) { arm.request_equipment(name); }
                }
                self.equipment_archetypes[side] = Some(target.equipment.archetype);
                self.equipment_changes.callbacks.push_back(Callback::Equipped { identity: target.identity.clone(), side });
                slot.equipped = true;
                slot.restore = false;
                slot.reselect = false;
                slot.phase = Phase::Equip;
            }
        }
    }
}
