//! Source DamageZone callback phases over the canonical component roster.
use crate::gameplay::entity_lifetime::LiveEntities;
use bevy::prelude::World;
fn phase(world: &mut World, gather: bool) -> Result<(), String> {
    let Some(clock) = world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
    else {
        return Ok(());
    };
    if !clock.advanced() {
        return Ok(());
    }
    let marker = clock.clock()?.frame_marker();
    if !world
        .get_resource::<super::physics_scene::PhysicsScene>()
        .is_some_and(|scene| scene.damage_zone_callbacks_pending(gather, marker))
    {
        return Ok(());
    }
    let Some(mut scene) = world.remove_resource::<super::physics_scene::PhysicsScene>() else {
        return Ok(());
    };
    let Some(mut live) = world.remove_resource::<LiveEntities>() else {
        world.insert_resource(scene);
        return Err("DamageZone live identity owner absent".into());
    };
    let Some(mut events) = world.remove_resource::<crate::gameplay::entity_events::EntityEvents>()
    else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("DamageZone entity event owner absent".into());
    };
    let result = (|| {
        if gather {
            scene.gather_damage_zones(world, &mut live, &mut events, marker)
        } else {
            scene.update_damage_zones(world, &live, marker)
        }
    })();
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    result
}
pub(super) fn gather(world: &mut World) -> Result<(), String> {
    phase(world, true)
}
pub(super) fn update(world: &mut World) -> Result<(), String> {
    phase(world, false)
}
