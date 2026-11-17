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
    pub rumbles: usize,
    pub fovs: usize,
    pub quads: usize,
    pub sub_effects: usize,
    pub retired: u64,
}
impl OwnerSnapshot {
    pub(crate) fn only_decal_tails(&self) -> bool {
        !self.handles.is_empty() && self.pending_starts == 0
            && self.stopping == self.handles.len()
            && self.particles == 0 && self.sounds == 0 && self.rumbles == 0 && self.fovs == 0 && self.quads == 0 && self.decals != 0
    }
}
fn sub_chains(sub: &super::super::world_sub_effects::SubEffect, sample: &mut OwnerSnapshot) {
    sample.sub_effects += 1;
    sample.particles += usize::from(sub.head != u16::MAX);
    sample.sounds += usize::from(sub.sound_head != u16::MAX);
    sample.decals += usize::from(sub.decal_head != u16::MAX);
    sample.rumbles += usize::from(sub.rumble_head != u16::MAX);
    sample.fovs += usize::from(sub.fov_head != u16::MAX);
    sample.quads += usize::from(sub.quad_head != u16::MAX);
    for child in &sub.children { sub_chains(child, sample); }
}
impl WorldParticleState {
    /// Actual sound-object identities for one already admitted actor. This
    /// borrows existing main/sub chains; it never posts or drains an event.
    pub(crate) fn observe_owner_audio(&self,owner:&str)->Result<Vec<crate::gameplay::sound::Emitter>,String>{
        fn children(state:&WorldParticleState,sub:&super::super::world_sub_effects::SubEffect,
            result:&mut Vec<crate::gameplay::sound::Emitter>)->Result<(),String>{
            state.sound_systems.visit_objects(sub.sound_head,|object|{if !result.contains(&object){result.push(object)}})?;
            for child in &sub.children{children(state,child,result)?;}
            Ok(())
        }
        let mut result=Vec::new();
        for instance in self.slots.iter().flatten(){
            if !self.entities.get(&instance.entity).is_some_and(|entity|
                matches!(&entity.owner,FxEntityOwner::Actor{name,..} if name==owner)){continue}
            self.sound_systems.visit_objects(instance.sound_head,|object|{if !result.contains(&object){result.push(object)}})?;
            for sub in &instance.sub_effects{children(self,sub,&mut result)?;}
        }
        Ok(result)
    }
    /// One-shot candidates from actual started chains, never declaration-only
    /// admission guesses. The returned text owns no source or material lease.
    pub(crate) fn observe_started_fx(&self) -> Vec<String> {
        let mut result = Vec::new();
        for (slot, instance) in self.slots.iter().enumerate().filter_map(|(slot, instance)| instance.as_ref().map(|instance| (slot, instance))) {
            if self.pending_starts.contains(&(slot as u16)) { continue; }
            let mut chains = OwnerSnapshot::default();
            chains.particles = usize::from(instance.head != u16::MAX);
            chains.sounds = usize::from(instance.sound_head != u16::MAX);
            chains.decals = usize::from(instance.decal_head != u16::MAX);
            chains.rumbles = usize::from(instance.rumble_head != u16::MAX);
            chains.fovs = usize::from(instance.fov_head != u16::MAX);
            chains.quads = usize::from(instance.quad_head != u16::MAX);
            for sub in &instance.sub_effects { sub_chains(sub, &mut chains); }
            if chains.particles == 0 && chains.sounds == 0 && chains.decals == 0 && chains.rumbles == 0 && chains.fovs == 0 && chains.quads == 0 { continue; }
            let owner = self.entities.get(&instance.entity).map(|entity| match &entity.owner {
                FxEntityOwner::Actor { name, .. } => format!("actor:{name}"),
                FxEntityOwner::Node { graph, node } => format!("graph:{} node:{node}", self.graphs.owner(*graph)),
                FxEntityOwner::Detached => "detached".to_owned(),
            });
            result.push(format!("effect={:?} owner={owner:?} entity={} handle={} particle_heads={} sound_heads={} decal_heads={} rumble_heads={} fov_heads={} quad_heads={} sub_effects={}",
                instance.parameters.effect, instance.entity, instance.id,
                chains.particles, chains.sounds, chains.decals, chains.rumbles, chains.fovs, chains.quads, chains.sub_effects));
        }
        result.sort();
        result
    }

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
            sample.rumbles += usize::from(instance.rumble_head != u16::MAX);
            sample.fovs += usize::from(instance.fov_head != u16::MAX);
            sample.quads += usize::from(instance.quad_head != u16::MAX);
            for sub in &instance.sub_effects { sub_chains(sub, &mut sample); }
        }
        sample.handles.sort_unstable();
        sample
    }
    pub(crate) fn observe_pool_counts(&self) -> (usize, usize, u16, u16) {
        (self.handles.len(), self.pending_starts.len(), self.sound_systems.allocated_count(), self.decals.allocated_count())
    }
}
