//! Canonical level explosion request queue (7b57d0/7b2210).
//! Request admission precedes ammunition consumption; the actual physics
//!scene executes grouped recipients, occlusion, health and body effects.
use crate::{assets::explosions::Declaration, gameplay::explosions::Request};
use bevy::prelude::*;
use std::collections::VecDeque;
#[derive(Resource, Default)]
pub(super) struct Explosions {
    pub(super) pending: VecDeque<Request>,
    queries: super::physics_scene::explosions::ExplosionQueries,
    failure: Option<String>,
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<Explosions>();
}
pub(super) fn prepare(world: &mut World, declaration: &Declaration) -> Result<(), String> {
    if !world.contains_resource::<Explosions>() {
        return Err("canonical explosion manager is not installed".into());
    }
    // Declaration/damage dependencies are retained by their Catalog registries.
    // Contact FX must be requested through their existing owner before the
    //host can publish that authored callback, rather than read in this queue.
    if declaration.contact_material.is_some()
        || declaration.ground_material.is_some()
        || declaration.water_fx.is_some()
    {
        return Err("explosion contact/ground/water effect dependencies require canonical contact FX preparation".into());
    }
    Ok(())
}
pub(super) fn ready(world: &World, declaration: &Declaration) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<Explosions>() else {
        return Ok(false);
    };
    if let Some(error) = &owner.failure {
        return Err(error.clone());
    }
    if declaration.contact_material.is_some()
        || declaration.ground_material.is_some()
        || declaration.water_fx.is_some()
    {
        return Ok(false);
    }
    let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() else {
        return Ok(false);
    };
    scene.explosion_queries_ready()
}
pub(super) fn enqueue(world: &mut World, request: Request) -> Result<(), String> {
    if !ready(world, &request.declaration)? {
        return Err("explosion source dependencies are not ready".into());
    }
    world
        .resource_mut::<Explosions>()
        .pending
        .push_back(request);
    Ok(())
}

///7b3830/7b4ab0: run at the canonical post-physics request-consumer phase,
///before EntityEvents are pumped. Requests stay owned until dispatch succeeds.
pub(super) fn update(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<Explosions>() {
        return Ok(());
    }
    world.resource_scope(|world,mut owner:Mut<Explosions>|{
        if let Some(error)=&owner.failure{return Err(error.clone())}
        while let Some(request)=owner.pending.front().cloned(){
            let result=world.resource_scope(|world,mut scene:Mut<super::physics_scene::PhysicsScene>|{
                world.resource_scope(|world,mut live:Mut<crate::gameplay::entity_lifetime::LiveEntities>|{
                    world.resource_scope(|world,mut events:Mut<crate::gameplay::entity_events::EntityEvents>|{
                        scene.execute_explosion(&mut owner.queries,&mut live,&mut events,&request,
                            Default::default(),|live,id|super::runtime_entities::bounds_for(world,live,id))
                    })
                })
            });
            if let Err(error)=result{owner.failure=Some(error.clone());return Err(error)}
            owner.pending.pop_front();
        }
        Ok(())
    })
}
