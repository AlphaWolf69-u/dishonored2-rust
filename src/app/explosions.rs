//! Canonical level explosion request queue (7b57d0/7b2210).
//! Request admission precedes ammunition consumption; the actual physics
//!scene executes grouped recipients, occlusion, health and body effects.
use super::physics_scene::explosion_effects;
use crate::{assets::explosions::Declaration, gameplay::explosions::Request};
use bevy::prelude::*;
use std::collections::VecDeque;
use std::sync::Arc;
#[derive(Resource, Default)]
pub(super) struct Explosions {
    pending: VecDeque<Pending>,
    queries: super::physics_scene::explosions::ExplosionQueries,
    failure: Option<String>,
    effects: explosion_effects::Owner,
    effect_queries: explosion_effects::Queries,
    damage_done: bool,
    packets: Option<Vec<explosion_effects::Packet>>,
    effect_cursor: usize,
}
#[derive(Clone)]
struct Pending {
    request: Request,
    _source_lease: Option<Arc<()>>,
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<Explosions>()
        .add_systems(PreUpdate, pump_effects);
}
fn pump_effects(world: &mut World) -> Result<(), bevy::ecs::error::BevyError> {
    world.resource_scope(|world, mut owner: Mut<Explosions>| owner.effects.pump(world)).map_err(Into::into)
}
pub(super) fn prepare_owned(
    world: &mut World,
    scope: Arc<str>,
    declaration: &Declaration,
) -> Result<(), String> {
    if !world.contains_resource::<Explosions>() {
        return Err("canonical explosion manager is not installed".into());
    }
    world.resource_scope(|world, mut owner: Mut<Explosions>| {
        owner.effects.prepare(world, scope, declaration)
    })
}
pub(super) fn ready_owned(
    world: &World,
    scope: &str,
    declaration: &Declaration,
) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<Explosions>() else {
        return Ok(false);
    };
    if let Some(error) = &owner.failure {
        return Err(error.clone());
    }
    Ok(owner.effects.ready(world, scope, declaration)?
        && world
            .get_resource::<super::physics_scene::PhysicsScene>()
            .map(super::physics_scene::PhysicsScene::explosion_queries_ready)
            .transpose()?
            .unwrap_or(false))
}
pub(super) fn release_owned(world: &mut World, scope: &str) -> Result<(), String> {
    if world.contains_resource::<Explosions>() {
        world.resource_scope(|world, mut owner: Mut<Explosions>| {
            owner.effects.release(world, scope)
        });
    }
    Ok(())
}
pub(super) fn prepare_contact_owned(
    world: &mut World,
    scope: Arc<str>,
    source: &str,
) -> Result<(), String> {
    if !world.contains_resource::<Explosions>() {
        return Err("canonical contact FX manager is not installed".into());
    }
    world.resource_scope(|world, mut owner: Mut<Explosions>| {
        owner.effects.prepare_contact(world, scope, source)
    })
}
pub(super) fn ready_contact_owned(
    world: &World,
    scope: &str,
    source: &str,
) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<Explosions>() else {
        return Ok(false);
    };
    owner.effects.ready_contact(world, scope, source)
}
pub(super) fn post_contact_owned(
    world: &mut World,
    scope: &str,
    source: &str,
    hit: &crate::gameplay::shape_ray_query::ShapeRayContact,
    direction: bevy_math::Vec3,
    context: crate::rendering::bevy_backend::world_effects::EntityEffectContext,
) -> Result<Option<super::usable_effects::EffectTicket>, String> {
    world.resource_scope(|world, mut owner: Mut<Explosions>| {
        owner
            .effects
            .post_contact(world, scope, source, hit, direction, context)
    })
}
pub(super) fn ready(world: &World, declaration: &Declaration) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<Explosions>() else {
        return Ok(false);
    };
    if let Some(error) = &owner.failure {
        return Err(error.clone());
    }
    if !owner.effects.ready_for(world, declaration)? {
        return Ok(false);
    }
    let Some(scene) = world.get_resource::<super::physics_scene::PhysicsScene>() else {
        return Ok(false);
    };
    scene.explosion_queries_ready()
}
pub(super) fn enqueue(world: &mut World, request: Request) -> Result<(), String> {
    enqueue_inner(world, request, None)
}
pub(super) fn enqueue_with_lease(
    world: &mut World,
    request: Request,
    lease: Arc<()>,
) -> Result<(), String> {
    enqueue_inner(world, request, Some(lease))
}
fn enqueue_inner(
    world: &mut World,
    mut request: Request,
    lease: Option<Arc<()>>,
) -> Result<(), String> {
    if !ready(world, &request.declaration)? {
        return Err("explosion source dependencies are not ready".into());
    }
    //7b57d0 +96 substitutes the current real player, or the source null
    //identity if there is none; it does not synthesize another attacker.
    if request.declaration.attacker_always_player {
        request.attacker = world
            .get_resource::<super::player::PlayerState>()
            .and_then(|state| state.player.as_ref())
            .and_then(|player| player.managed_entity());
    }
    world
        .resource_mut::<Explosions>()
        .pending
        .push_back(Pending {
            request,
            _source_lease: lease,
        });
    Ok(())
}
///44e0c0 forwards accepted spread damage here before projectile birth.
///7b3830/7b4ab0 consume that retained request without a physical time step;
///finish before Breakable callbacks and their actual FX/source scopes close.
pub(super) fn finish_accepted(world: &mut World) -> Result<bool, String> {
    update(world)?;
    Ok(world.get_resource::<Explosions>().is_none_or(|owner|owner.pending.is_empty()))
}

pub(super) fn shutdown(world: &mut World) {
    if world.contains_resource::<Explosions>() {
        world.resource_scope(|world, mut owner: Mut<Explosions>| {
            owner.effects.close(world);
            owner.pending.clear();
            owner.failure = None;
            owner.queries = Default::default();
            owner.damage_done = false;
            owner.packets = None;
            owner.effect_cursor = 0;
        });
    }
}
pub(super) fn retirement_complete(world: &mut World) -> Result<bool, String> {
    if !world.contains_resource::<Explosions>() {
        return Ok(true);
    }
    world.resource_scope(|world, mut owner: Mut<Explosions>| {
        owner.effects.pump(world)?;
        Ok(owner.effects.drained())
    })
}

///7b3830/7b4ab0: run at the canonical post-physics request-consumer phase,
///before EntityEvents are pumped. Requests stay owned until dispatch succeeds.
pub(super) fn update(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<Explosions>() {
        return Ok(());
    }
    world.resource_scope(|world,mut owner:Mut<Explosions>|{
        owner.effects.pump(world)?;
        if let Some(error)=&owner.failure{return Err(error.clone())}
        while let Some(pending)=owner.pending.front().cloned(){
            let request=pending.request;
            let result=(||->Result<(),String>{
            if !owner.damage_done{let(modifiers,full)=modifiers(world,&request)?;
            world.resource_scope(|world,mut scene:Mut<super::physics_scene::PhysicsScene>|{
                world.resource_scope(|world,mut live:Mut<crate::gameplay::entity_lifetime::LiveEntities>|{
                    world.resource_scope(|world,mut events:Mut<crate::gameplay::entity_events::EntityEvents>|{
                        scene.execute_explosion(&mut owner.queries,&mut live,&mut events,&request,
                            modifiers,full,|live,id|super::runtime_entities::bounds_for(world,live,id))
                    })
                })
            })?;owner.damage_done=true;}
            if owner.packets.is_none(){
                let contact_hits=owner.queries.contact_hits.clone();
                let(contacts,height)=world.resource_mut::<super::physics_scene::PhysicsScene>()
                    .explosion_effect_contacts(&mut owner.effect_queries,&request,&contact_hits)?;
                let water=height.map(|height|{
                    let yaw=world.get_resource_mut::<super::player::PlayerState>().and_then(|mut state|state.player.as_mut().map(|player|
                        player.inventory_random()*f32::from_bits(0x40c90fdb))).ok_or("explosion water FX requires the actual gameplay random owner")?;
                    Ok::<_,String>((height,yaw))
                }).transpose()?;
                owner.packets=Some(owner.effects.packets(&request,&contacts,water)?);
            }
            let mut packets=owner.packets.take().unwrap();
            let mut cursor=owner.effect_cursor;
            let result=owner.effects.dispatch(world,&packets,&mut cursor);owner.effect_cursor=cursor;
            if result.is_err(){owner.packets=Some(std::mem::take(&mut packets));}result
            })();
            if let Err(error)=result{owner.failure=Some(error.clone());return Err(error)}
            owner.pending.pop_front();
            owner.damage_done=false;owner.packets=None;owner.effect_cursor=0;
        }
        Ok(())
    })
}

///7b57d0's real player attributes;7b4310 includes the inflictor's actual
///entity attacker field, independently of attachment or Throwable launchers.
fn modifiers(
    world: &World,
    request: &Request,
) -> Result<(crate::gameplay::explosions::RadiusModifiers, bool), String> {
    let Some(player) = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
    else {
        return Ok((Default::default(), false));
    };
    let Some(id) = player.managed_entity() else {
        return Ok((Default::default(), false));
    };
    let owned = request.inflictor == Some(id)
        || request.attacker == Some(id)
        || request
            .inflictor
            .map(|inflictor| {
                let live = world
                    .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                    .ok_or("explosion live entity owner absent")?;
                if live.get(inflictor).is_none() {
                    Ok(None)
                } else {
                    live.owner(inflictor)
                }
            })
            .transpose()?
            .flatten()
            == Some(id);
    let attribute = |name| player.attributes.0.get(name).copied().unwrap_or(1.);
    let source_radius = match request.declaration.source {
        1 => attribute("ARK_ATTRIBUTE_INCENDIARY_BOLT_RANGE_FACTOR"),
        2 => attribute("ARK_ATTRIBUTE_BOTTLE_EXPLOSION_RANGE_FACTOR"),
        3 => attribute("ARK_ATTRIBUTE_HOWLING_BOLTS_RANGE_FACTOR"),
        _ => 1.,
    };
    let player_owned_radius = if owned && matches!(request.declaration.source, 2 | 5 | 6) {
        attribute("ARK_ATTRIBUTE_PLAYER_INFLICTED_EXPLOSION_RADIUS_FACTOR")
    } else {
        1.
    };
    let full = owned
        && player
            .attributes
            .0
            .get("ARK_ATTRIBUTE_PLAYER_INFLICTED_EXPLOSION_DAMAGE_DO_NOT_FALOFF")
            .copied()
            .unwrap_or(0.)
            > 0.;
    Ok((
        crate::gameplay::explosions::RadiusModifiers {
            source_radius,
            player_owned_radius,
        },
        full,
    ))
}

///651020's real two-material SurfaceImpact packet uses the same ContactFX
///owner without constructing an artificial ray-query witness.
pub(super) fn prepare_contact_pair_owned(world:&mut World,scope:Arc<str>,source:&str,target:&str)->Result<(),String>{
    world.resource_scope(|world,mut owner:Mut<Explosions>|owner.effects.prepare_contact_pair(world,scope,source,target))
}
pub(super) fn post_contact_pair_owned(world:&mut World,scope:&str,contact:super::physics_scene::explosion_effects::Contact,
    context:crate::rendering::bevy_backend::world_effects::EntityEffectContext,
)->Result<Option<super::usable_effects::EffectTicket>,String>{
    world.resource_scope(|world,mut owner:Mut<Explosions>|owner.effects.post_contact_value(world,scope,contact,context))
}
