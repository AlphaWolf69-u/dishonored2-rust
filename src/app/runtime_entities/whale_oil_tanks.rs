//!Placed tank child demands use the same prototype lease and startup owner.
use super::*;
pub(super) fn advance(world: &mut World) -> Result<(), String> {
    let owner = world.resource::<RuntimeEntities>();
    if owner.cancelled.load(Ordering::Acquire) || owner.placed_whale_oil_tanks.is_empty() {
        return Ok(());
    }
    let requests = world
        .resource::<RuntimeEntities>()
        .placed_whale_oil_tanks
        .iter()
        .enumerate()
        .filter(|(_, plan)| !plan.requested)
        .map(|(index, plan)| (index, plan.scope.clone(), plan.pusher.clone()))
        .collect::<Vec<_>>();
    for (index, scope, child) in requests {
        if let Some(child) = child {
            request_for(world, scope, child)?;
        }
        world
            .resource_mut::<RuntimeEntities>()
            .placed_whale_oil_tanks[index]
            .requested = true;
    }
    for plan in &world.resource::<RuntimeEntities>().placed_whale_oil_tanks {
        if let Some(child) = &plan.pusher {
            if !ready(world, child)? {
                return Ok(());
            }
        }
    }
    let mut plans = std::mem::take(
        &mut world
            .resource_mut::<RuntimeEntities>()
            .placed_whale_oil_tanks,
    );
    let result = with_scene(world, |world, scene, live, events| {
        scene.admit_prepared_whale_oil_tanks(world, live, events, &mut plans)
    });
    if let Err(error) = result {
        let entered = world
            .resource::<super::super::physics_scene::PhysicsScene>()
            .runtime_startup_owner();
        let parent = entered
            .as_ref()
            .ok()
            .copied()
            .flatten()
            .filter(|id| plans.iter().any(|plan| plan.actor == *id));
        world
            .resource_mut::<RuntimeEntities>()
            .placed_whale_oil_tanks = plans;
        if let Some(parent) = parent {
            world
                .resource_mut::<RuntimeEntities>()
                .actors
                .entry(parent)
                .or_insert_with(Actor::placed)
                .rollback = true;
        }
        return Err(error);
    }
    let mut owner = world.resource_mut::<RuntimeEntities>();
    for plan in plans {
        if let Some(child) = plan.pusher {
            owner
                .placed_leases
                .entry(plan.actor)
                .or_default()
                .push(placed::Lease {
                    scope: plan.scope,
                    children: vec![child],
                });
        }
    }
    Ok(())
}
pub(super) fn shutdown(world: &mut World) -> Result<(), String> {
    let plans = std::mem::take(
        &mut world
            .resource_mut::<RuntimeEntities>()
            .placed_whale_oil_tanks,
    );
    for plan in plans {
        if plan.requested {
            if let Some(child) = plan.pusher {
                release_for(world, &plan.scope, &child)?;
            }
        }
    }
    Ok(())
}
