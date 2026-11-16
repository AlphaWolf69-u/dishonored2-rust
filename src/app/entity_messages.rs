//! Game-local normal numeric-message pump (8101f0/810000). The registry and
//! its actual registered component generations remain the existing owners.
use crate::gameplay::{entity_events::EntityEvents, entity_lifetime::LiveEntities};
use bevy::prelude::*;

/// Preserve real callback failures at outgoing lifetime boundaries.
pub(super) fn finish_accepted(world: &mut World) -> Result<(), String> {
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        return Ok(());
    };
    // 6170b0/615840/6147f0 notifications join the same normal queue as
    // damage and power messages, after mutation and before listener delivery.
    let inventory_result = super::player_lifetime::id(world).map_or(Ok(()), |player| {
        world
            .get_resource_mut::<super::interaction_focus::DiscoveredDocuments>()
            .map_or(Ok(()), |mut inventory| {
                inventory
                    .publish_gameplay_events(player, &mut events)
                    .map(|_| ())
            })
    });
    let result = inventory_result.and_then(|()| {
        if world.contains_resource::<super::physics_scene::PhysicsScene>()
            && world.contains_resource::<LiveEntities>()
        {
            world.resource_scope(
                |world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                    scene.pump_entity_messages(
                        world.resource::<LiveEntities>(),
                        &mut events,
                        world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>(),
                    )
                },
            )
        } else {
            // The canonical pump normally discards an unsubscribed message.
            // A real registered recipient without its owner is an actual error.
            let errors = events.pump(|listener, _, _| {
                Err(format!(
                    "entity-message callback owner is absent: {listener:?}"
                ))
            });
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            }
        }
    });
    world.insert_resource(events);
    result
}

pub(super) fn update(world: &mut World) {
    if let Err(error) = finish_accepted(world) {
        error!("Entity message delivery failed: {error}");
        crate::platform::client_diagnostics::record(&format!(
            "Entity message delivery failed: {error}"
        ));
        world.write_message(AppExit::error());
    }
}
