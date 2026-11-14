//! Entity graph receiver retirement, before the shared identity is removed.
//! Kiscule declaration instances and completion queues stay with the existing
//! world/player FX manager; this adapter never constructs another manager.
use bevy::prelude::*;
use crate::gameplay::entity_lifetime::EntityId;

pub(super) fn retire(world: &mut World, entity: EntityId) -> Result<(), String> {
    if let Some(mut effects) = world.get_resource_mut::<crate::rendering::bevy_backend::camera_scene::FxSimulation>() {
        effects.retire_entity_graph(entity)?;
    }
    Ok(())
}
