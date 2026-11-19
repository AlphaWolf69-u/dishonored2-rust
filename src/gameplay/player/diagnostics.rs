//! Player state and recent traversal results for F8 captures.
use super::Player;
use std::fmt::Write;

impl Player {
    /// Borrow actual switch-owner state for opt-in action diagnostics.
    pub fn equipment_action_status(&self, side: usize) -> Option<(&str, bool, bool)> {
        Some((self.equipment_changes.preferred_identity(side)?,
            self.equipment_changes.equipped(side), self.equipment_changes.transitioning(side)))
    }
    pub fn write_combat_diagnostics(&self, report: &mut String) {
        let _ = writeln!(report, "\nCombat current snapshot (not action history): swimming={} equipment_handles={:?} animation_events={} weapon_callbacks={} throwable_callbacks={} target_entity={:?} target_point={:?}", self.swimming_active, self.equipment_action_handles, self.animation_events.len(), self.weapon_effects.len(), self.throwable_effects.len(), self.ranged_target_entity, self.ranged_target_hit);
        let _ = writeln!(report, "  ranged identity={:.160} holstered={} throwable identity={:.160} holstered={} heart={:?}", self.weapon_identity.as_deref().unwrap_or("<none>"), self.holstered_weapons.len(), self.throwable_identity.as_deref().unwrap_or("<none>"), self.holstered_throwables.len(), self.heart_entity);
        for side in 0..2 {
            let _ = writeln!(report, "  equipment side={} present={} equipped={} transitioning={} socket_published={}", side, self.equipment_changes.preferred(side).is_some(), self.equipment_changes.equipped(side), self.equipment_changes.transitioning(side), self.equipment_frames[side].is_some());
        }
        let _ = writeln!(report, "  INTERACT down={} pressed={} released={} interaction_descriptor_present={}",self.controller.down(22),self.controller.pressed(22),self.controller.released(22),self.interaction_present);
        self.equipment_changes.write_diagnostics(report);
        self.carrying.write_trace(report);
        if let Some(weapon) = &self.weapon { weapon.write_diagnostics(report); }
        if let Some(weapon) = &self.throwable {
            let _ = writeln!(report, "  throwable entity={:?} ammo={} magazine={} total={} ready={} scene={} cook_duration={} cook_stage={} low_throw={} armed={} exploded={} flags={:#x} accepted_source={}", weapon.entity, weapon.ammunition_index, weapon.magazine, weapon.total_ammo, weapon.ready, weapon.active_scene, weapon.cooking_duration, weapon.cooking_stage, weapon.low_throw, weapon.grenade_state.is_some_and(|g| g.armed), weapon.grenade_state.is_some_and(|g| g.exploded), weapon.target_flags, weapon.source.is_some());
        }
        self.machine.write_combat_diagnostics(report, "land");
        self.swimming_machine.write_combat_diagnostics(report, "swim");
        self.powers.write_diagnostics(report);
        let _ = writeln!(report, "  latest_scene_start_error={:?}", self.body.midnight.last_start_error);
        self.body.midnight.write_combat_diagnostics(report);
    }
}
use super::{character_proxy::WorldHit, physics::PlayerPhysics, states::StateId};
use crate::gameplay::powers::traversal::{BlinkFailure, BlinkRequest};
use bevy_math::Vec3;

/// The F8 report can be taken several seconds after the movement has ended.
/// Keep scalar values from recent transitions. No gameplay resources are retained.
#[derive(Default)]
pub(super) struct TraversalHistory {
    events: [Option<TraversalSample>; 16],
    next: usize,
    followup: u8,
}

#[derive(Debug)]
#[allow(dead_code, reason = "Field names and values are included in the F8 report via Debug")]
pub(super) enum TraversalEvent {
    BlinkBegin { request: BlinkRequest, previous_velocity: Vec3 },
    BlinkPlace { interrupted_by: Option<BlinkFailure>, position: Vec3 },
    BlinkNavigate { fitted_state: StateId, falling_requested: bool },
    BlinkInterrupted,
    VaultBegin { obstacle: Vec3, height: f32, forward: Vec3, speed: f32, start_animation: Vec3, exit: Vec3 },
    VaultRestoreStanding,
    VaultEnd { phase: u8, corrected_position: Vec3 },
    AfterPhysics { ticks: u8, state: Option<StateId>, requested_velocity: Option<Vec3>, dt: f32 },
}

struct TraversalSample {
    event: TraversalEvent,
    time: f64,
    position: Vec3,
    velocity: Vec3,
    posture: usize,
    ground_distance: f32,
    ground_query_length: f32,
    ceiling_distance: f32,
    contact_count: usize,
    contacts: [Option<WorldHit>; 3],
}

impl TraversalHistory {
    pub(super) fn record(&mut self, physics: &PlayerPhysics, time: f64, event: TraversalEvent) {
        if matches!(event, TraversalEvent::BlinkPlace { .. } | TraversalEvent::VaultEnd { .. } | TraversalEvent::BlinkInterrupted) {
            self.followup = 1;
        }
        let mut contacts = physics.proxy.contact_hits();
        self.events[self.next] = Some(TraversalSample {
            time, event, position: physics.position(), velocity: physics.velocity,
            posture: physics.posture, ground_distance: physics.ground_distance, ground_query_length: physics.ground_query_length,
            ceiling_distance: physics.ceil_distance, contact_count: physics.proxy.contact_count(),
            contacts: std::array::from_fn(|_| contacts.next()),
        });
        self.next = (self.next + 1) % self.events.len();
    }

    pub(super) fn after_physics(&mut self, physics: &PlayerPhysics, time: f64, state: Option<StateId>, requested_velocity: Option<Vec3>, dt: f32) {
        let ticks = self.followup;
        if ticks == 0 { return; }
        if matches!(ticks, 1 | 4 | 16 | 64) {
            self.record(physics, time, TraversalEvent::AfterPhysics { ticks, state, requested_velocity, dt });
        }
        self.followup = if ticks < 64 { ticks + 1 } else { 0 };
    }

    pub(super) fn write(&self, report: &mut String) {
        use std::fmt::Write;
        for s in self.events[self.next..].iter().chain(&self.events[..self.next]).flatten() {
            let _ = writeln!(report, "    traversal t={} {:?} feet={:?} velocity={:?} posture={} ground={} ground_query_length={} ceiling={} contacts={} witnesses={:?}",
                s.time, s.event, s.position, s.velocity, s.posture, s.ground_distance, s.ground_query_length, s.ceiling_distance, s.contact_count, s.contacts);
        }
    }
}
