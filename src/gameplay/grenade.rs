//! Actual arkCpntGrenade fuse state490430/490600/491a20.
//! The scene executes FX, component-reference, notification and removal actions.
use super::{
    component_state::{ComponentInstance, ComponentOwner},
    pickable_component::CheckedComponent,
};
use crate::assets::grenades::Definition;
use std::sync::Arc;

pub const ARMED: u8 = 1;
pub const UNDERWATER: u8 = 2;
pub const EXPLODE_ON_CONTACT: u8 = 4;
pub const EXPLODED: u8 = 8;
pub const DANGER_REGISTERED: u8 = 16;
pub const PICKED_UP: u8 = 32;
pub const CANNOT_INCAPACITATE: u8 = 64;

pub struct Component {
    pub owner: ComponentOwner,
    pub definition: Arc<Definition>,
    pub pickable: Option<CheckedComponent>,
    pub total: f32,
    pub remaining: f32,
    pub minimum: f32,
    ///The low seven bits are exactly component+38, not host event flags.
    pub flags: u8,
    pub initialized: bool,
    pub started: bool,
    pub updated_frame: Option<i32>,
    ///bevy_backend: fuse subtraction precedes fallible effect publication.
    pub ticked_frame: Option<i32>,
    ///bevy_backend: acknowledgements within synchronous4911b0 Start.
    pub start_phase: u8,
}
impl Component {
    pub fn new(definition: Arc<Definition>) -> Self {
        Self {
            owner: ComponentOwner::default(),
            definition,
            pickable: None,
            total: 0.,
            remaining: 0.,
            minimum: 0.,
            flags: 0,
            initialized: false,
            started: false,
            updated_frame: None,
            ticked_frame: None,
            start_phase: 0,
        }
    }
    pub fn armed(&self) -> bool {
        self.flags & ARMED != 0
    }
    pub fn exploded(&self) -> bool {
        self.flags & EXPLODED != 0
    }
    ///490600's state change. Caller supplies490ef0's actual player/enemy attribute.
    ///True bypass_hit_floor is the actual CookIdle call's R8B=1.
    pub fn arm_fuse(&mut self, delay_factor: f32, bypass_hit_floor: bool) -> bool {
        if self.exploded() || self.armed() {
            return false;
        }
        self.flags |= ARMED;
        self.total = delay_factor * self.definition.detonation_delay;
        self.remaining = if self.total <= self.definition.minimum_after_hit {
            self.definition.minimum_after_hit
        } else {
            self.total
        };
        self.minimum = if bypass_hit_floor {
            0.
        } else {
            self.definition.minimum_after_hit
        };
        true
    }
    ///4907a0 leaves timers and other status bits intact.
    pub fn disarm_fuse(&mut self) -> bool {
        if !self.armed() || self.exploded() {
            return false;
        }
        self.flags &= !ARMED;
        true
    }
    ///491640 is distinct from Disarm: both armed/exploded bits and timers clear.
    pub fn reset_fuse(&mut self) {
        self.flags &= !(ARMED | EXPLODED);
        self.total = 0.;
        self.remaining = 0.;
        self.minimum = 0.;
    }
    ///490f70 copies only1/2/8; contact/pickup/danger flags remain the target's.
    pub fn copy_fuse(&mut self, source: &Self) {
        const COPIED: u8 = ARMED | UNDERWATER | EXPLODED;
        self.flags = (self.flags & !COPIED) | (source.flags & COPIED);
        self.total = source.total;
        self.remaining = source.remaining;
        self.minimum = source.minimum;
    }
    ///491a20 subtracts the entity-local delta and clamps to the hit floor.
    ///Hidden ordinary grenades clear that floor; held equipment does not.
    pub fn advance_fuse(&mut self, delta: f32, hidden: bool, held_equipment: bool) -> bool {
        if !self.armed() || self.exploded() {
            return false;
        }
        if hidden && !held_equipment {
            self.minimum = 0.;
        }
        let next = self.remaining - delta;
        self.remaining = if next <= self.minimum {
            self.minimum
        } else {
            next
        };
        if self.remaining <= 0. {
            self.remaining = 0.;
            return true;
        }
        false
    }
    ///490de0: first contact clears the minimum and admits the actual danger zone.
    pub fn contact(&mut self) {
        self.minimum = 0.;
    }
    ///491550: first pickup guarantees the authored remaining delay once.
    pub fn picked_up(&mut self) {
        if self.armed() && !self.exploded() && self.flags & PICKED_UP == 0 {
            if self.remaining <= self.definition.minimum_on_pickup {
                self.remaining = self.definition.minimum_on_pickup;
            }
            self.flags |= PICKED_UP;
        }
    }
    ///4915a0 uses total delay rather than current remaining time.
    pub fn restore_delay(&mut self) {
        if self.armed() && !self.exploded() {
            self.remaining = if self.total <= self.definition.minimum_after_hit {
                self.definition.minimum_after_hit
            } else {
                self.total
            };
        }
    }
    ///490e70: only explicitly contact-explosive grenades shorten to zero.
    pub fn explode_contact(&mut self) -> bool {
        if self.armed() && self.flags & EXPLODE_ON_CONTACT != 0 {
            self.remaining = 0.;
            self.minimum = 0.;
            return true;
        }
        false
    }
    ///490d00 after the real positive damage gate.
    pub fn damage_fuse(&mut self, cannot_incapacitate: bool) {
        if self.definition.damaged_delay <= self.remaining {
            self.remaining = self.definition.damaged_delay;
        }
        self.minimum = 0.;
        self.flags = (self.flags & !CANNOT_INCAPACITATE)
            | if cannot_incapacitate {
                CANNOT_INCAPACITATE
            } else {
                0
            };
    }
    pub fn progress(&self) -> f32 {
        if self.armed() {
            1. - self.remaining / self.total
        } else {
            0.
        }
    }
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}
