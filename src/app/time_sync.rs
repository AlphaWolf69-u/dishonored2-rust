//! Time-manager root synchronization80e910/80cee0 over actual entity owners.
use super::physics_scene::PhysicsScene;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    game_clock_runtime::ClockRuntime,
};
use bevy::prelude::*;

fn time_root(
    scene: &PhysicsScene,
    live: &LiveEntities,
    clock: &mut ClockRuntime,
    mut entity: EntityId,
) -> Result<EntityId, String> {
    let mut visited = Vec::new();
    //80cee0 first follows explicit time-space attachments (+2b8).
    loop {
        if visited.contains(&entity) {
            return Err("cyclic entity time attachment".into());
        }
        visited.push(entity);
        let (space, _) = scene.entity_time_binding(live, entity)?;
        clock.register_entity_time(entity, space)?;
        let Some(parent) = clock.entity_time_parent(entity).map_err(str::to_owned)? else {
            break;
        };
        entity = parent;
    }
    //Then877530's actual actor binding chain, stopping BELOW an excluded
    //parent. Managed owner(+ac) is deliberately not an attachment parent.
    while let Some(parent) = live.parent(entity)? {
        let (space, excluded) = scene.entity_time_binding(live, parent)?;
        if excluded {
            break;
        }
        if visited.contains(&parent) {
            return Err("cyclic entity time binding".into());
        }
        visited.push(parent);
        clock.register_entity_time(parent, space)?;
        entity = parent;
    }
    Ok(entity)
}

///7ee0e0 calls this after the real child release and before7278e0 instigator
///publication. Only the existing time manager owns countdowns and root queue.
pub(super) fn synchronize_entity_time(
    world: &mut World,
    live: &LiveEntities,
    parent: EntityId,
    child: EntityId,
) -> Result<(), String> {
    world
        .try_resource_scope(|world, mut clock: Mut<ClockRuntime>| {
            if !clock.time_is_bent().map_err(str::to_owned)? {
                return Ok(());
            }
            let scene = world
                .get_resource::<PhysicsScene>()
                .ok_or("time synchronization scene owner absent")?;
            if scene.entity_time_binding(live, parent)?.1
                || scene.entity_time_binding(live, child)?.1
            {
                return Ok(());
            }
            let parent = time_root(scene, live, &mut clock, parent)?;
            let child = time_root(scene, live, &mut clock, child)?;
            if parent == child {
                return Ok(());
            }
            let a = clock
                .entity_time_state(parent)
                .map_err(str::to_owned)?
                .1
                .unwrap_or(0.);
            let b = clock
                .entity_time_state(child)
                .map_err(str::to_owned)?
                .1
                .unwrap_or(0.);
            //Native80e9ec COMISS/CMOVBE selects child on equality/unordered.
            let mut duration = if a > b { a } else { b };
            if duration == f32::from_bits(0x7149_f2ca) {
                //61c890 reads worldspawn+4e4 (m_contactUnbentTimeDuration).
                //This desktop client hosts the actual loaded game world, not
                //the unsupported editor/game-mode4 branch of that getter.
                duration = scene.contact_unbent_duration().unwrap_or(0.);
            }
            if duration > 0. {
                //set_entity_time_override preserves an existing infinite root
                //while entering each real identity in the same source queue.
                clock.set_entity_time_override(parent, duration)?;
                clock.set_entity_time_override(child, duration)?;
            }
            Ok(())
        })
        .ok_or("time synchronization clock owner absent")?
}

///87ce70/87d050 publish model-clock state before the normal AD/AC event.
///The retained source FIFO is acknowledged only after all actual owners accept.
pub(super) fn publish_transitions(world: &mut World) -> Result<(), String> {
    let Some(mut clock) = world.remove_resource::<ClockRuntime>() else {
        return Ok(());
    };
    let result = (|| {
        while let Some((entity, space, stage, changed)) = clock.entity_transition_front()? {
            let live = world
                .get_resource::<LiveEntities>()
                .ok_or("entity clock transition live owner absent")?;
            if live.get(entity).is_none() {
                return Err("entity clock transition generation stale".into());
            }
            match stage {
                0 => {
                    let (old, excluded) = world
                        .get_resource::<PhysicsScene>()
                        .ok_or("entity time transition scene absent")?
                        .entity_time_binding(live, entity)?;
                    clock.register_entity_time(entity, old)?;
                    clock.prepare_entity_transition(entity, space, excluded)?;
                }
                1 => {
                    if changed {
                        world
                            .try_resource_scope(|world, mut scene: Mut<PhysicsScene>| {
                                scene.publish_entity_timespace(
                                    world.resource::<LiveEntities>(),
                                    entity,
                                    space,
                                )
                            })
                            .ok_or("entity time transition scene absent")??;
                    }
                    clock.advance_entity_transition(entity, space, 1)?;
                }
                2 => {
                    if changed {
                        let (group, offset) =
                            clock.entity_model_clock(entity).map_err(str::to_owned)?;
                        super::runtime_entities::set_shader_clock(world, entity, group, offset)?;
                    }
                    clock.advance_entity_transition(entity, space, 2)?;
                }
                3 => {
                    if changed {
                        world
                            .get_resource_mut::<EntityEvents>()
                            .ok_or("entity clock transition event owner absent")?
                            .post(Event {
                                id: if space == 1 { 0xad } else { 0xac },
                                target: Some(entity),
                                payload: Payload::None,
                            });
                    }
                    clock.advance_entity_transition(entity, space, 3)?;
                }
                4 => clock.finish_entity_transition(entity, space, live.children(entity)?)?,
                _ => return Err("entity time publication stage invalid".into()),
            }
        }
        Ok(())
    })();
    world.insert_resource(clock);
    result
}
