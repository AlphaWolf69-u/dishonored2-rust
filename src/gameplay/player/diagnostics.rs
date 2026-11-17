//! Read-only current-state formatting, called only by the F8 report.
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
        if let Some(weapon) = &self.weapon { weapon.write_diagnostics(report); }
        if let Some(weapon) = &self.throwable {
            let _ = writeln!(report, "  throwable entity={:?} ammo={} magazine={} total={} ready={} scene={} cook_duration={} cook_stage={} low_throw={} armed={} exploded={} flags={:#x} accepted_source={}", weapon.entity, weapon.ammunition_index, weapon.magazine, weapon.total_ammo, weapon.ready, weapon.active_scene, weapon.cooking_duration, weapon.cooking_stage, weapon.low_throw, weapon.grenade_state.is_some_and(|g| g.armed), weapon.grenade_state.is_some_and(|g| g.exploded), weapon.target_flags, weapon.source.is_some());
        }
        self.machine.write_combat_diagnostics(report, "land");
        self.swimming_machine.write_combat_diagnostics(report, "swim");
        self.powers.write_diagnostics(report);
        self.body.midnight.write_combat_diagnostics(report);
    }
}
