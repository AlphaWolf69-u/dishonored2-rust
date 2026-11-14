//! Read-only diagnostics of the production FX manager; never advances a clock.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct OwnerSnapshot {
    pub frame: Option<u32>,
    pub handles: Vec<u32>,
    pub pending_starts: usize,
    pub stopping: usize,
    pub particles: usize,
    pub sounds: usize,
    pub decals: usize,
    pub sub_effects: usize,
    pub retired: u64,
}
impl OwnerSnapshot {
    pub(crate) fn only_decal_tails(&self) -> bool {
        !self.handles.is_empty() && self.pending_starts == 0
            && self.stopping == self.handles.len()
            && self.particles == 0 && self.sounds == 0 && self.decals != 0
    }
}
fn sub_chains(sub: &super::super::world_sub_effects::SubEffect, sample: &mut OwnerSnapshot) {
    sample.sub_effects += 1;
    sample.particles += usize::from(sub.head != u16::MAX);
    sample.sounds += usize::from(sub.sound_head != u16::MAX);
    sample.decals += usize::from(sub.decal_head != u16::MAX);
    for child in &sub.children { sub_chains(child, sample); }
}
impl WorldParticleState {
    /// Family counts are actual nonempty chain heads (including nested FX),
    /// not fabricated particle counts or inferred declaration activity.
    pub(crate) fn observe_owner(&self, owner: &str) -> OwnerSnapshot {
        let mut sample = OwnerSnapshot { frame: self.frame, retired: self.dynamic_retired, ..Default::default() };
        for (slot, instance) in self.slots.iter().enumerate().filter_map(|(slot, instance)| instance.as_ref().map(|instance| (slot, instance))) {
            let matches = self.entities.get(&instance.entity).is_some_and(|entity|
                matches!(&entity.owner, FxEntityOwner::Actor { name, .. } if name == owner));
            if !matches { continue; }
            sample.handles.push(instance.id);
            sample.stopping += usize::from(self.dynamic_retiring.contains(&instance.entity));
            sample.pending_starts += usize::from(self.pending_starts.contains(&(slot as u16)));
            sample.particles += usize::from(instance.head != u16::MAX);
            sample.sounds += usize::from(instance.sound_head != u16::MAX);
            sample.decals += usize::from(instance.decal_head != u16::MAX);
            for sub in &instance.sub_effects { sub_chains(sub, &mut sample); }
        }
        sample.handles.sort_unstable();
        sample
    }
    pub(crate) fn observe_pool_counts(&self) -> (usize, usize, u16, u16) {
        (self.handles.len(), self.pending_starts.len(), self.sound_systems.allocated_count(), self.decals.allocated_count())
    }
}
