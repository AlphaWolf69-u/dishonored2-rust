//! Reflected delayed entity functions,883f60/884030/8851f0/8852e0.
//! The source uses clock group2 and stable insertion after equal deadlines.
//! Removal joins the ordinary managed-entity FIFO; no callback frees a slot.
use crate::gameplay::{
    damage::Parameters,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
    game_clock_runtime::ClockRuntime,
};
use bevy::prelude::*;
#[derive(Clone, Copy)]
enum Function {
    Activate,
    Remove,
    RemoveOrGhost,
    RemoveWhenHidden { retry_delay:f32, retries:i32 },
    Damage(Parameters),
}
#[derive(Clone, Copy)]
struct Scheduled {
    target: EntityId,
    due: f64,
    function: Function,
}
#[derive(Resource, Default)]
pub(super) struct EntityFunctions {
    pending: Vec<Scheduled>,
    closed: bool,
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<EntityFunctions>();
}
fn enqueue(
    world: &mut World,
    target: EntityId,
    delay: f32,
    function: Function,
) -> Result<(), String> {
    if !delay.is_finite() {
        return Err("non-finite delayed entity function".into());
    }
    if world.resource::<LiveEntities>().get(target).is_none() {
        return Err("scheduled entity identity is stale".into());
    }
    schedule(world, target, delay, function)
}
/// Component callbacks may hold the canonical live owner outside `World`
/// while Stop/Deinitialize runs. Validate against that same owner, never a
/// second lookup of a temporarily withdrawn Bevy resource.
fn enqueue_with_live(
    world: &mut World,
    live: &LiveEntities,
    target: EntityId,
    delay: f32,
    function: Function,
) -> Result<(), String> {
    if !delay.is_finite() {
        return Err("non-finite delayed entity function".into());
    }
    if live.get(target).is_none() {
        return Err("scheduled entity identity is stale".into());
    }
    schedule(world, target, delay, function)
}
fn schedule(world: &mut World, target: EntityId, delay: f32, function: Function) -> Result<(), String> {
    let now = world
        .resource::<ClockRuntime>()
        .clock()?
        .time(2)
        .ok_or("entity-event clock group absent")?;
    let owner = &mut *world.resource_mut::<EntityFunctions>();
    if owner.closed {
        return Err("entity-function manager is closing".into());
    }
    let entry = Scheduled {
        target,
        due: now + f64::from(delay),
        function,
    };
    let position = owner
        .pending
        .partition_point(|existing| existing.due <= entry.due);
    owner.pending.insert(position, entry);
    Ok(())
}
pub(super) fn enqueue_removal(
    world: &mut World,
    target: EntityId,
    delay: f32,
) -> Result<(), String> {
    enqueue(world, target, delay, Function::Remove)
}
pub(super) fn enqueue_removal_with_live(
    world: &mut World,
    live: &LiveEntities,
    target: EntityId,
    delay: f32,
) -> Result<(), String> {
    enqueue_with_live(world, live, target, delay, Function::Remove)
}
///Reflected activate uses the same deferred function manager as removal;
///875290 dispatches the concrete class hook and its normal C5 notification.
pub(super) fn enqueue_activation(world: &mut World, target: EntityId, delay: f32) -> Result<(), String> {
    enqueue(world, target, delay, Function::Activate)
}
///883f60 -> idEntity8cf620 caseB2 ->876ff0. Keep this source event
///distinct from Remove; actual declared AITampering changes its result.
pub(super) fn enqueue_remove_or_ghost(world: &mut World, target: EntityId, delay: f32) -> Result<(), String> {
    enqueue(world,target,delay,Function::RemoveOrGhost)
}
pub(super) fn enqueue_remove_or_ghost_with_live(
    world: &mut World,
    live: &LiveEntities,
    target: EntityId,
    delay: f32,
) -> Result<(), String> {
    enqueue_with_live(world, live, target, delay, Function::RemoveOrGhost)
}
///627620/627780 ->883f80 schedules removeWhenHidden877140 on the
///canonical entity-function clock, independent of a component's time space.
pub(super) fn enqueue_hidden_removal(world:&mut World,target:EntityId,delay:f32,retry_delay:f32,retries:i32)->Result<(),String>{
    if !retry_delay.is_finite(){return Err("non-finite hidden removal retry delay".into())}
    enqueue(world,target,delay,Function::RemoveWhenHidden{retry_delay,retries})
}
pub(super) fn enqueue_damage(
    world: &mut World,
    target: EntityId,
    params: Parameters,
    delay: f32,
) -> Result<(), String> {
    enqueue(world, target, delay, Function::Damage(params))
}
/// Run after source-effect journals, before the ordinary removal batch snapshot.
/// Even delay0 is queued until this phase (never freed inside the producer).
pub(super) fn update(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<EntityFunctions>() {
        return Ok(());
    }
    let now = world
        .resource::<ClockRuntime>()
        .clock()?
        .time(2)
        .ok_or("entity-event clock group absent")?;
    world.resource_scope(|world, mut owner: Mut<EntityFunctions>| {
        while let Some(entry) = owner
            .pending
            .first()
            .copied()
            .filter(|entry| entry.due <= now)
        {
            let mut reschedule=None;
            // Generation-bearing identities make cancellation safe after another
            // source already removed this target; never resolve by recycled name.
            if world.resource::<LiveEntities>().get(entry.target).is_some() {
                match entry.function {
                    Function::Activate => super::runtime_entities::activate(world, entry.target)?,
                    Function::Remove => {
                        world.resource_mut::<LiveEntities>().request_removal(entry.target)?;
                        //884980 invokes the native function, then883d10 sends
                        //its actual linked notice_EV_Remove to observers.
                        world.resource_scope(|world,mut scene:Mut<super::physics_scene::PhysicsScene>| {
                            scene.dispatch_remove_notice(&mut world.resource_mut::<LiveEntities>(),entry.target)
                        })?;
                    },
                    Function::RemoveOrGhost => {
                        let removes=world.resource::<super::physics_scene::PhysicsScene>()
                            .remove_or_ghost_removes(world.resource::<LiveEntities>(),entry.target)?;
                        if removes { world.resource_mut::<LiveEntities>().request_removal(entry.target)?; }
                    }
                    Function::RemoveWhenHidden{retry_delay,retries}=>{
                        let visible=super::world_models::render_state(world,entry.target)
                            .is_some_and(|model|!model.hidden&&(model.current||model.previous));
                        if retries!=0&&visible {
                            reschedule=Some(Scheduled{target:entry.target,due:now+f64::from(retry_delay),
                                function:Function::RemoveWhenHidden{retry_delay,retries:if retries<1{-1}else{retries-1}}});
                        }else{world.resource_mut::<LiveEntities>().request_removal(entry.target)?;}
                    }
                    Function::Damage(params) => {
                        world.resource_scope(
                            |world, mut scene: Mut<super::physics_scene::PhysicsScene>| {
                                world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                                    let mut events = world.resource_mut::<EntityEvents>();
                                    scene
                                        .damage_entity(
                                            &mut live,
                                            &mut events,
                                            entry.target,
                                            &params,
                                        )
                                        .map(|_| ())
                                })
                            },
                        )?;
                    }
                }
            }
            owner.pending.remove(0);
            if let Some(entry)=reschedule {
                let index=owner.pending.partition_point(|scheduled|scheduled.due<=entry.due);
                owner.pending.insert(index,entry);
            }
        }
        Ok(())
    })
}
/// Source883f60 deadlines use the same frozen group2 clock during shutdown.
/// Stop/Deinit may have accepted delay0 work after this frame's normal pump;
/// its real target/notice owners must survive until that work is executed.
pub(super) fn due_pending(world:&World)->Result<bool,String>{
    let Some(entry)=world.get_resource::<EntityFunctions>().and_then(|owner|owner.pending.first()) else{return Ok(false)};
    let now=world.get_resource::<ClockRuntime>().ok_or("accepted entity function has no live clock owner")?
        .clock()?.time(2).ok_or("entity-event clock group absent")?;
    Ok(entry.due<=now)
}
pub(super) fn retire_entity(world: &mut World, target: EntityId) {
    if let Some(mut owner) = world.get_resource_mut::<EntityFunctions>() {
        owner.pending.retain(|entry| entry.target != target);
    }
}
pub(super) fn begin_retirement(world: &mut World) {
    if let Some(mut owner) = world.get_resource_mut::<EntityFunctions>() {
        owner.closed = true;
        owner.pending.clear();
    }
}
