//! Game-local normal numeric-message pump (8101f0/810000). The registry and
//! its actual registered component generations remain the existing owners.
use crate::gameplay::{entity_events::EntityEvents, entity_lifetime::LiveEntities};
use bevy::prelude::*;

///7ec5b0(stage,1) drains this same normal FIFO after each accepted source
///stage. Callers already borrowing the Scene/Live/Event owners do not remove
///or copy those large resources a second time merely to dispatch messages.
pub(super) fn finish_borrowed(world:&mut World,scene:&mut super::physics_scene::PhysicsScene,
    live:&LiveEntities,events:&mut EntityEvents)->Result<(),String>{
    let mut ai_worlds=world.remove_resource::<crate::gameplay::ai_world::Worlds>();
    let mut hearts=world.remove_resource::<super::heart::Hearts>();
    let mut statistics=world.remove_resource::<super::player_statistics::PlayerStatistics>();
    let result=if world.contains_resource::<super::player::PlayerState>() {
        world.resource_scope(|world,mut player:Mut<super::player::PlayerState>|{
            scene.pump_entity_messages(live,events,world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>(),
                player.player.as_mut(),ai_worlds.as_mut(),hearts.as_mut(),statistics.as_mut())
        })
    }else{
        scene.pump_entity_messages(live,events,world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>(),
            None,ai_worlds.as_mut(),hearts.as_mut(),statistics.as_mut())
    };
    if let Some(owner)=ai_worlds{world.insert_resource(owner);}
    if let Some(owner)=hearts{world.insert_resource(owner);}
    if let Some(owner)=statistics{world.insert_resource(owner);}
    result?;
    scene.process_openable_uses(world,live,events)
}

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
    let mut ai_worlds = world.remove_resource::<crate::gameplay::ai_world::Worlds>();
    let mut hearts=world.remove_resource::<super::heart::Hearts>();
    let mut statistics=world.remove_resource::<super::player_statistics::PlayerStatistics>();
    let result = inventory_result.and_then(|()| {
        if world.contains_resource::<super::physics_scene::PhysicsScene>()
            && world.contains_resource::<LiveEntities>()
        {
            world.resource_scope(
                |world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                    let result=if world.contains_resource::<super::player::PlayerState>() {
                        world.resource_scope(|world,mut player:Mut<super::player::PlayerState>| {
                            scene.pump_entity_messages(world.resource::<LiveEntities>(),&mut events,
                                world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>(),player.player.as_mut(), ai_worlds.as_mut(),hearts.as_mut(),statistics.as_mut())
                        })
                    } else {
                        scene.pump_entity_messages(world.resource::<LiveEntities>(),&mut events,
                            world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>(),None, ai_worlds.as_mut(),hearts.as_mut(),statistics.as_mut())
                    };
                    result?;
                    world.resource_scope(|world,live:Mut<LiveEntities>|scene.process_openable_uses(world,&live,&mut events))
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
    if let Some(owner) = ai_worlds { world.insert_resource(owner); }
    if let Some(owner)=hearts {world.insert_resource(owner);}
    if let Some(owner)=statistics {world.insert_resource(owner);}
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
