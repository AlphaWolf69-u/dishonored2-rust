//! arkCpntRigidBodyThrowable state,4c6600/4c7930/4c7fc0/4c8140.
//! The actual component owns its checked identities and pending binding; body,
//! geometry, actor and contact delivery remain in the canonical physics scene.
use crate::gameplay::{
    collision_settings::CollisionSettings, component_state::entity::ComponentHandle,
    entity_lifetime::EntityId, reference_frame::ReferenceFrame,
};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub(crate) struct Settings {
    pub high_quality: bool,
    pub sticky: bool,
    pub notify_hit: bool,
    pub player_damage_using_raycast: bool,
    pub attached_collision: CollisionSettings,
    pub stick_distance: f32,
    pub head_damage: Option<Arc<crate::assets::damage::Declaration>>,
    pub damping_after_first_hit: f32,
    pub dropped_reset_distance: f32,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Binding {
    pub parent: EntityId,
    /// Full WorldStorage body identity, never its masked slot alone.
    pub body: u32,
    /// The actual collision shape key; a skeletal host resolves its real tag.
    pub shape_key: u32,
    pub frame: ReferenceFrame,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StickPhase {
    Quality,
    Frame,
    Bind,
    Notify,
    Complete,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingStick {
    pub request: Binding,
    pub phase: StickPhase,
    pub dynamic_target: bool,
}

/// Typed projection of the source 0x90-byte contact4 record. Resource IDs and
/// checked component/entity generations replace borrowed executable pointers.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ContactBody {
    pub body: u32,
    pub component: ComponentHandle,
    pub entity: EntityId,
    pub material: u16,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Contact {
    pub bodies: [ContactBody; 2],
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub velocity: [f32; 3],
    pub distance_squared: f32,
    pub other_frame: ReferenceFrame,
    pub damage: f32,
}

pub(crate) struct State {
    pub source: Arc<crate::assets::throwable::Definition>,
    /// Native+438 defaults0x42; callers change only the source-owned bits.
    flags: u8,
    pub initialized: bool,
    /// +439 bit0 is the deferred collision update request.
    collision_update_pending: bool,
    pub center: [f32; 3],
    pub damping_profile: Option<u16>,
    pub instigator: Option<EntityId>,
    pub binding: Option<Binding>,
    pub pending_stick: Option<PendingStick>,
    listener: Option<(usize, ComponentHandle, u32, EntityId)>,
    contacts: [Option<Contact>; 4],
    contact_count: usize,
    contact_delivery: u8,
    damage_frame: i32,
    damage_targets: Vec<EntityId>,
}
impl State {
    pub fn new(source: Arc<crate::assets::throwable::Definition>) -> Self {
        Self {
            source,
            flags: 0x42,
            initialized: false,
            collision_update_pending: false,
            center: [0.; 3],
            damping_profile: None,
            instigator: None,
            binding: None,
            pending_stick: None,
            listener: None,
            contacts: [None; 4],
            contact_count: 0,
            contact_delivery: 0,
            damage_frame: -1,
            damage_targets: Vec::new(),
        }
    }
    pub fn flags(&self) -> u8 {
        self.flags
    }
    /// 4c6580 gives each component four inline entries; 4c7c50 rejects a
    /// full list before calling the source duplicate/nearest-point rule.
    pub fn push_contact(&mut self, contact: Contact) -> Result<(), String> {
        if self.contact_count == self.contacts.len() {
            return Err("Throwable contact list is full".into());
        }
        for slot in &mut self.contacts[..self.contact_count] {
            let old = slot.as_mut().ok_or("Throwable contact list has a hole")?;
            if old.bodies[0].component == contact.bodies[0].component
                && old.bodies[1].component == contact.bodies[1].component
            {
                if old.distance_squared <= contact.distance_squared {
                    return Ok(());
                }
                *old = contact;
                //4c64c0 replaces the nearer duplicate then appends it as well.
                break;
            }
        }
        self.contacts[self.contact_count] = Some(contact);
        self.contact_count += 1;
        Ok(())
    }
    /// 4c9560 drains last-in-first-out, acknowledging only after its callback.
    pub fn pending_contact(&self) -> Option<Contact> {
        self.contact_count
            .checked_sub(1)
            .and_then(|i| self.contacts[i])
    }
    pub fn finish_contact(&mut self) -> Result<(), String> {
        let i = self
            .contact_count
            .checked_sub(1)
            .ok_or("Throwable contact list empty")?;
        self.contacts[i] = None;
        self.contact_count = i;
        self.contact_delivery = 0;
        Ok(())
    }
    ///4c7f40 retires all inline contacts during the actual component Stop.
    pub fn clear_contacts(&mut self) {
        self.contacts.fill(None);
        self.contact_count = 0;
        self.contact_delivery = 0;
        self.damage_targets.clear();
    }
    pub fn contact_delivery(&self) -> u8 {
        self.contact_delivery
    }
    pub fn delivered_contact_stage(&mut self, stage: u8) {
        self.contact_delivery = stage;
    }
    pub fn first_hit(&self) -> bool {
        self.flags & 0x10 != 0
    }
    pub fn npc_hit(&self) -> bool {
        self.flags & 0x20 != 0
    }
    pub fn mark_first_hit(&mut self) {
        self.flags |= 0x10;
    }
    pub fn mark_npc_hit(&mut self) {
        self.flags |= 0x20;
    }
    ///8071e0 suppresses duplicate damage to the same target in one frame.
    pub fn damaged_in_frame(&mut self, frame: i32, target: EntityId) -> bool {
        if self.damage_frame != frame {
            self.damage_frame = frame;
            self.damage_targets.clear();
        }
        self.damage_targets.contains(&target)
    }
    pub fn mark_damaged_in_frame(&mut self, target: EntityId) -> Result<(), String> {
        self.damage_targets
            .try_reserve(1)
            .map_err(|e| format!("Throwable damage target admission: {e}"))?;
        self.damage_targets.push(target);
        Ok(())
    }
    pub fn reserve_damage_target(&mut self) -> Result<(), String> {
        self.damage_targets
            .try_reserve(1)
            .map_err(|e| format!("Throwable damage target admission: {e}"))
    }
    pub fn thrown(&self) -> bool {
        self.flags & 8 != 0
    }
    pub fn dropped(&self) -> bool {
        self.flags & 4 != 0
    }
    pub fn stuck(&self) -> bool {
        self.flags & 0x80 != 0
    }
    /// 4c94e0 records the actual launcher inventory dependency without waking
    /// a dormant projectile body. The host resolves its real managed entity.
    pub fn set_instigator(&mut self, entity: EntityId) {
        self.instigator = Some(entity);
        self.flags |= 0x48;
    }
    /// 4c7020 marks the actual player drop, then applies the player's collision
    /// filter to this same body through the scene's existing filter owner.
    pub fn mark_dropped(&mut self, entity: EntityId) {
        self.instigator = Some(entity);
        self.flags |= 0x4c;
    }
    pub fn reset_dropped_distance(&mut self, position: [f32; 3], player: [f32; 3]) {
        if self.dropped() {
            let d = std::array::from_fn::<_, 3, _>(|i| player[i] - position[i]);
            let distance = (d[1] * d[1] + d[0] * d[0]) + d[2] * d[2];
            if self.source.settings.dropped_reset_distance
                * self.source.settings.dropped_reset_distance
                < distance
            {
                self.flags &= !4;
            }
        }
    }
    ///4c8f00: quality3 while disabled,10 while enabled,11 if sticky.
    pub fn quality(&self, enabled: bool) -> Option<u8> {
        self.source.settings.high_quality.then_some(if enabled {
            10 + u8::from(self.source.settings.sticky)
        } else {
            3
        })
    }
    pub fn quality_applied(&mut self, enabled: bool) {
        self.flags = (self.flags & !2) | (u8::from(enabled) << 1);
    }
    ///4c7930 queries actual component ancestry, not an entity-name heuristic.
    pub fn set_contact_damage_receiver(&mut self, present: bool) {
        self.flags = (self.flags & !1) | u8::from(present);
    }
    pub fn contact_events(&self, motion_type: u8) -> bool {
        motion_type == 2
            && (self.source.settings.sticky
                || self.source.settings.notify_hit
                || self.flags & 1 != 0)
    }
    pub fn listener(&self) -> Option<(usize, ComponentHandle, u32, EntityId)> {
        self.listener
    }
    pub fn set_listener(&mut self, value: Option<(usize, ComponentHandle, u32, EntityId)>) {
        self.listener = value;
    }
    pub fn request_collision_update(&mut self) {
        self.collision_update_pending = true;
    }
    pub fn collision_update_pending(&self) -> bool {
        self.collision_update_pending
    }
    ///4c7570 clears sticking only after the real unbind/filter restoration.
    pub fn detached(&mut self) {
        self.flags &= !0x80;
        self.collision_update_pending = false;
        self.binding = None;
    }
    pub fn mark_stuck(&mut self, binding: Binding) {
        self.binding = Some(binding);
        self.flags |= 0x80;
        self.pending_stick = None;
    }
    /// Release only after actual unregister/body+binding retirement. This is
    /// not an acknowledgement for pending actor or physics publication work.
    pub fn can_release(&self) -> bool {
        self.listener.is_none() && self.pending_stick.is_none()
    }
    pub fn retire(&mut self) -> Result<(), String> {
        if !self.can_release() {
            return Err("Throwable retains listener or body binding during retirement".into());
        }
        self.instigator = None;
        self.damping_profile = None;
        self.binding = None;
        Ok(())
    }
}
