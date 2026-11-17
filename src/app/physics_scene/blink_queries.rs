//! Blink reads the actual admitted rigid component's body generation. The
//! level triangle adapter's owner index is never treated as a Havok body ID.
use super::*;
use crate::gameplay::{entity_lifetime::{LiveEntities, Stage}, component_state::entity_lookup::ComponentLookupState};
impl PhysicsScene {
    pub(in crate::app) fn blink_collision_body(&self, live: &LiveEntities, name: &str) -> Result<Option<u32>, String> {
        let Some(id) = live.find(name) else { return Ok(None) };
        if live.stage(id) != Some(Stage::Spawned) { return Ok(None); }
        let Some(&index) = self.shared_indices.get(&id) else { return Ok(None) };
        let entity = &self.entities[index];
        let range = self.rigid_class_range()?;
        let Some(handle) = entity.components.find_component(range, [0x20,0], |handle| {
            let component = self.rigid_component(handle)?;
            Ok(ComponentLookupState { flags: component.owner.flags, start_phases: self.declarations.get(&handle.packed()).map(|decl| decl.scheduling.start_phases) })
        })? else { return Ok(None) };
        Ok(self.rigid_component(handle)?.body())
    }
}
