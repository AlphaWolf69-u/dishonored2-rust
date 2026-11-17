//! Time-manager root synchronization80e910/80cee0 over actual entity owners.
use super::physics_scene::PhysicsScene;
use crate::gameplay::{
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    game_clock_runtime::ClockRuntime,
};
use bevy::prelude::*;

fn entity_binding(scene:&PhysicsScene, live:&LiveEntities, clock:&ClockRuntime, entity:EntityId)->Result<(u32,bool),String> {
    if clock.is_player_entity(entity) {
        if live.get(entity).is_none() { return Err("Player time identity is stale".into()); }
        //The actual Player bridge is constructed idEntity874c50: D0 is
        //zeroed and D1 bit7 set, leaving bit2 clear. No duplicate Scene header.
        return Ok((clock.entity_time_state(entity).map_err(str::to_owned)?.0,false));
    }
    scene.entity_time_binding(live,entity)
}
fn binding_root(live:&LiveEntities,mut entity:EntityId)->Result<EntityId,String> {
    let mut visited=Vec::new();
    while let Some(parent)=live.parent(entity)? {
        if visited.contains(&parent) {return Err("cyclic entity binding root".into());}
        visited.push(entity); entity=parent;
    }
    Ok(entity)
}
fn ordinary_time_root(scene:&PhysicsScene,live:&LiveEntities,clock:&mut ClockRuntime,mut entity:EntityId)->Result<EntityId,String> {
    let mut visited=Vec::new();
    while let Some(parent)=live.parent(entity)? {
        let (space,excluded)=entity_binding(scene,live,clock,parent)?;
        if excluded {break;}
        if visited.contains(&parent) {return Err("cyclic entity time binding".into());}
        visited.push(entity); clock.register_entity_time(parent,space)?; entity=parent;
    }
    Ok(entity)
}

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
        let (space, _) = entity_binding(scene, live, clock, entity)?;
        clock.register_entity_time(entity, space)?;
        let Some(parent) = clock.entity_time_parent(entity).map_err(str::to_owned)? else {
            break;
        };
        entity = parent;
    }
    //Then877530's actual actor binding chain, stopping BELOW an excluded
    //parent. Managed owner(+ac) is deliberately not an attachment parent.
    ordinary_time_root(scene,live,clock,entity)
}

fn detach_with_clock(scene:&PhysicsScene,live:&LiveEntities,clock:&mut ClockRuntime,child:EntityId)->Result<(),String> {
    let Some(parent)=clock.entity_time_parent(child).map_err(str::to_owned)? else {return Ok(());};
    if live.get(child).is_none() {clock.unlink_entity_time(child)?;return Ok(());}
    let transfer=if clock.entity_time_state(child).map_err(str::to_owned)?.0==1 {
        let old_root=time_root(scene,live,clock,parent)?;
        let mut duration=clock.entity_time_state(old_root).map_err(str::to_owned)?.1.unwrap_or(0.);
        if duration==f32::from_bits(0x7149_f2ca) {
            duration=scene.contact_unbent_duration().ok_or("time detach requires actual worldspawn contact-unbent duration")?;
        }
        //80e050 clears+2b8 before80cee0(child): resolve the ordinary
        //binding root directly while planning this fallible operation.
        Some((ordinary_time_root(scene,live,clock,child)?,duration))
    } else {None};
    clock.unlink_entity_time(child)?;
    if let Some((root,duration))=transfer {clock.set_entity_time_override(root,duration)?;}
    Ok(())
}
///83f590→80cd60, after actual47 eligibility and before76e2b0 Begin.
pub(super) fn attach_entity_time(world:&mut World,scene:&PhysicsScene,live:&LiveEntities,parent:EntityId,child:EntityId)->Result<(),String> {
    let mut clock=world.get_resource_mut::<ClockRuntime>().ok_or("time attachment clock absent")?;
    let parent_binding=entity_binding(scene,live,&clock,parent)?;
    let child_binding=entity_binding(scene,live,&clock,child)?;
    //80cea3 tail-diagnoses/returns;80cddc diagnoses/returns as well.
    //Neither branch attaches after166830 returns in the installed binary.
    if parent_binding.1||child_binding.1||binding_root(live,parent)?==binding_root(live,child)? {return Ok(());}
    clock.register_entity_time(parent,parent_binding.0)?;
    clock.register_entity_time(child,child_binding.0)?;
    if clock.entity_time_parent(child).map_err(str::to_owned)?.is_some() {detach_with_clock(scene,live,&mut clock,child)?;}
    clock.attach_entity_time(parent,child)
}
///841500/83fe70→80e050, before the actual completion/exit acknowledgement.
pub(super) fn detach_entity_time(world:&mut World,child:EntityId)->Result<(),String> {
    world.try_resource_scope(|world,mut clock:Mut<ClockRuntime>| {
        if clock.entity_time_parent(child).is_err() {return Ok(());}
        if clock.entity_time_parent(child).map_err(str::to_owned)?.is_none() {return Ok(());}
        let scene=world.get_resource::<PhysicsScene>().ok_or("time detach Scene absent")?;
        let live=world.get_resource::<LiveEntities>().ok_or("time detach identities absent")?;
        detach_with_clock(scene,live,&mut clock,child)
    }).ok_or("time detach clock absent")?
}
pub(super) fn detach_entity_time_with_owners(world:&mut World,scene:&PhysicsScene,live:&LiveEntities,child:EntityId)->Result<(),String> {
    let mut clock=world.get_resource_mut::<ClockRuntime>().ok_or("time detach clock absent")?;
    if clock.entity_time_parent(child).is_err() {return Ok(());}
    detach_with_clock(scene,live,&mut clock,child)
}
///Detach surviving children before losing the actual Scene/binding owners.
pub(super) fn retire_entity_time_with_owners(scene:&PhysicsScene,live:&LiveEntities,clock:&mut ClockRuntime,entity:EntityId)->Result<(),String> {
    while let Some(child)=clock.entity_time_children(entity).ok().and_then(|children|children.first()).copied() {
        detach_with_clock(scene,live,clock,child)?;
    }
    if clock.entity_time_parent(entity).is_ok_and(|parent|parent.is_some()) {detach_with_clock(scene,live,clock,entity)?;}
    clock.unregister_entity_time(entity); Ok(())
}
pub(super) fn retire_entity_time(world:&mut World,entity:EntityId)->Result<(),String> {
    world.try_resource_scope(|world,mut clock:Mut<ClockRuntime>| {
        if clock.entity_time_children(entity).is_err() {clock.unregister_entity_time(entity);return Ok(());}
        if clock.entity_time_children(entity).map_err(str::to_owned)?.is_empty()
            && clock.entity_time_parent(entity).map_err(str::to_owned)?.is_none() {
            clock.unregister_entity_time(entity);return Ok(());
        }
        let scene=world.get_resource::<PhysicsScene>().ok_or("time retirement Scene absent")?;
        let live=world.get_resource::<LiveEntities>().ok_or("time retirement identities absent")?;
        retire_entity_time_with_owners(scene,live,&mut clock,entity)
    }).ok_or("time retirement clock absent")?
}

///490e70/490840 call80d8c0 on the same actual entity time-root owner.
///The caller already holds Scene during its synchronous component update.
pub(super) fn request_entity_unbent(world:&mut World, scene:&PhysicsScene,
    live:&LiveEntities, entity:EntityId, duration:f32)->Result<(),String> {
    let mut clock=world.get_resource_mut::<ClockRuntime>()
        .ok_or("entity unbent clock owner absent")?;
    if !clock.time_is_bent().map_err(str::to_owned)? {return Ok(());}
    let root=time_root(scene,live,&mut clock,entity)?;
    clock.set_entity_time_override(root,duration)
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
                    let scene = world
                        .get_resource::<PhysicsScene>()
                        .ok_or("entity time transition scene absent")?;
                    let (old, excluded)=entity_binding(scene,live,&clock,entity)?;
                    clock.register_entity_time(entity, old)?;
                    clock.prepare_entity_transition(entity, space, excluded)?;
                }
                1 => {
                    if changed && !clock.is_player_entity(entity) {
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
                4 => {
                    let mut attached=Vec::new();
                    for child in clock.entity_time_children(entity).map_err(str::to_owned)? {
                        attached.push(binding_root(live,*child)?);
                    }
                    clock.finish_entity_transition(entity,space,live.children(entity)?,&attached)?;
                },
                _ => return Err("entity time publication stage invalid".into()),
            }
        }
        Ok(())
    })();
    world.insert_resource(clock);
    result
}
