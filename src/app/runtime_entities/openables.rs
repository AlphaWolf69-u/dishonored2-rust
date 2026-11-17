//!Placed Openable demand leases and the single shared-registry startup fence.
use super::*;
pub(super) fn advance(world:&mut World)->Result<(),String>{
    let owner=world.resource::<RuntimeEntities>();
    if owner.cancelled.load(Ordering::Acquire)||owner.placed_openables.is_empty(){return Ok(());}
    let requests=world.resource::<RuntimeEntities>().placed_openables.iter().enumerate()
        .filter(|(_,plan)|!plan.requested).map(|(index,plan)|(index,plan.scope.clone(),plan.animated.clone())).collect::<Vec<_>>();
    for (index,scope,child) in requests{
        request_for(world,scope,child)?;
        world.resource_mut::<RuntimeEntities>().placed_openables[index].requested=true;
    }
    for plan in &world.resource::<RuntimeEntities>().placed_openables{
        if !ready(world,&plan.animated)?{return Ok(());}
    }
    if world.resource::<RuntimeEntities>().placed_openables.is_empty(){return Ok(());}
    let mut plans=std::mem::take(&mut world.resource_mut::<RuntimeEntities>().placed_openables);
    let result=with_scene(world,|world,scene,live,events|scene.admit_prepared_openables(world,live,events,&mut plans));
    if let Err(error)=result{
        let entered=world.resource::<super::super::physics_scene::PhysicsScene>().runtime_startup_owner();
        let parent=entered.as_ref().ok().copied().flatten().filter(|id|plans.iter().any(|plan|plan.actor==*id));
        world.resource_mut::<RuntimeEntities>().placed_openables=plans;
        if let Some(parent)=parent{world.resource_mut::<RuntimeEntities>().actors.entry(parent).or_insert_with(Actor::placed).rollback=true;}
        if let Err(cursor_error)=entered{return Err(format!("{error}; Openable constructor cursor: {cursor_error}"));}
        return Err(error);
    }
    let mut owner=world.resource_mut::<RuntimeEntities>();
    for plan in plans{
        owner.placed_leases.entry(plan.actor).or_default().push(placed::Lease{scope:plan.scope,children:vec![plan.animated]});
    }
    Ok(())
}
pub(super) fn shutdown(world:&mut World)->Result<(),String>{
    let plans=std::mem::take(&mut world.resource_mut::<RuntimeEntities>().placed_openables);
    for plan in plans{if plan.requested{release_for(world,&plan.scope,&plan.animated)?;}}
    Ok(())
}
