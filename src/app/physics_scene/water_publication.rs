//! Publish source809880 model/event notifications after the physics borrow.
use super::*;
use crate::gameplay::{entity_events::EntityEvents, entity_lifetime::LiveEntities};
use bevy::prelude::World;

pub(in crate::app) fn update(world: &mut World) -> Result<(), String> {
    if world
        .get_resource::<PhysicsScene>()
        .is_none_or(|scene| scene.water_notifications.is_empty())
    {
        return Ok(());
    }
    let mut scene = world
        .remove_resource::<PhysicsScene>()
        .ok_or("water publication scene absent")?;
    let Some(live) = world.remove_resource::<LiveEntities>() else {
        world.insert_resource(scene);
        return Err("water publication identity owner absent".into());
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("water publication event owner absent".into());
    };
    let result = scene.drain_water_notifications(world, &live, &mut events);
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    result
}
/// Withdraw each accepted ring lease while the canonical ocean queue exists.
pub(in crate::app) fn release_queries(world: &mut World) -> Result<(), String> {
    if let Some(mut scene) = world.get_resource_mut::<PhysicsScene>() {
        scene.release_water_queries()?;
    }
    Ok(())
}
