//! Projectile components and retained collision scratch over the canonical world.
//! Admission/removal are delegated to runtime_entities, never a private factory.
use crate::{
    assets::projectiles::Prepared,
    gameplay::{
        collision_filter::CollisionQueryFilter,
        collision_hit::ShapeHitInfo,
        component_state::{ComponentInstance, ComponentOwner},
        entity_lifetime::EntityId,
        projectiles::{Projectile, SpawnRequest},
        shape_ray_query::ShapeRayContact,
        world_point_query::level::LevelPointQueries,
        world_ray_query::{WorldRayQuery, WorldRayRequest},
    },
};
use bevy::prelude::{App, Resource, World};
use std::sync::Arc;

/// Host dispatch acknowledgements are retained across a fallible callback.
/// A successful finite-flight tick must never be repeated to retry its FX,
/// body handoff, or retirement callback.
struct Delivery {
    handle: crate::gameplay::component_state::entity::ComponentHandle,
    step: super::physics_scene::projectiles::ProjectileStep,
    phase: u8,
    handoff_phase: u8,
    contact_cursor: usize,
    contact: Option<super::physics_scene::explosion_effects::Contact>,
    recovery_disabled: bool,
    stick_recoverable: Option<bool>,
    stick_start_requested: Option<bool>,
    stick_children: Option<Pickup>,
}
struct Trail {
    key: Arc<str>,
    ticket: super::usable_effects::EffectTicket,
    stopped: bool,
    suspended: bool,
}
#[derive(Resource, Default)]
pub(super) struct Projectiles {
    batch: Vec<(
        crate::gameplay::component_state::entity::ComponentHandle,
        EntityId,
    )>,
    deliveries: std::collections::HashMap<EntityId, Delivery>,
    hull: Vec<[f32; 4]>,
    trails: std::collections::HashMap<EntityId, Trail>,
    impacts: std::collections::HashMap<EntityId, Trail>,
    completed: Vec<EntityId>,
    trail_tails: Vec<(EntityId, Trail)>,
    // Generic contact sounds may acknowledge their source after impact. Keep
    // the actual entity generation alive until the canonical FX owner drains.
    contacts: Vec<(EntityId, super::usable_effects::EffectTicket)>,
    effect_stops: Vec<(
        crate::gameplay::component_state::entity::ComponentHandle,
        EntityId,
    )>,
}
pub(super) fn install(app: &mut App) {
    app.init_resource::<Projectiles>();
}
pub(super) fn begin_retirement(world: &mut World, id: EntityId) {
    let keys = world
        .get_resource_mut::<Projectiles>()
        .map(|mut owner| {
            let trail = owner.trails.get_mut(&id).and_then(|trail| {
                if trail.stopped {
                    None
                } else {
                    trail.stopped = true;
                    Some(trail.key.clone())
                }
            });
            let impact = owner.impacts.get_mut(&id).and_then(|impact| {
                if impact.stopped {
                    None
                } else {
                    impact.stopped = true;
                    Some(impact.key.clone())
                }
            });
            [trail, impact]
        })
        .unwrap_or([None, None]);
    for key in keys.into_iter().flatten() {
        world
            .resource_mut::<super::usable_effects::EffectRequests>()
            .stop_entity(key);
    }
}
pub(super) fn retired(world: &mut World, id: EntityId) -> bool {
    if !super::audio::contact_callbacks_complete(world, id) {
        return false;
    }
    if world
        .get_resource::<Projectiles>()
        .and_then(|owner| owner.trails.get(&id))
        .is_some_and(|trail| !super::usable_effects::entity_retired(world, &trail.ticket))
    {
        return false;
    }
    if world
        .get_resource::<Projectiles>()
        .and_then(|owner| owner.impacts.get(&id))
        .is_some_and(|impact| !super::usable_effects::entity_retired(world, &impact.ticket))
    {
        return false;
    }
    if world.get_resource::<Projectiles>().is_some_and(|owner| {
        owner.contacts.iter().any(|(entity, ticket)| {
            *entity == id && !super::usable_effects::entity_retired(world, ticket)
        }) || owner.trail_tails.iter().any(|(entity, trail)| {
            *entity == id && !super::usable_effects::entity_retired(world, &trail.ticket)
        })
    }) {
        return false;
    }
    if let Some(mut owner) = world.get_resource_mut::<Projectiles>() {
        owner.trails.remove(&id);
        owner.impacts.remove(&id);
        owner.deliveries.remove(&id);
        owner.trail_tails.retain(|(entity, _)| *entity != id);
        owner.contacts.retain(|(entity, _)| *entity != id);
    }
    true
}
pub(super) fn shutdown(world: &mut World) -> bool {
    let Some(mut owner) = world.remove_resource::<Projectiles>() else {
        return true;
    };
    for trail in owner
        .trails
        .values_mut()
        .chain(owner.impacts.values_mut())
        .chain(owner.trail_tails.iter_mut().map(|(_, trail)| trail))
    {
        if !trail.stopped {
            world
                .resource_mut::<super::usable_effects::EffectRequests>()
                .stop_entity(trail.key.clone());
            trail.stopped = true;
        }
    }
    let done = owner
        .trails
        .values()
        .chain(owner.impacts.values())
        .chain(owner.trail_tails.iter().map(|(_, trail)| trail))
        .all(|trail| super::usable_effects::entity_retired(world, &trail.ticket))
        && owner
            .contacts
            .iter()
            .all(|(_, ticket)| super::usable_effects::entity_retired(world, ticket))
        && owner.deliveries.is_empty();
    if done {
        owner.trails.clear();
        owner.impacts.clear();
        owner.trail_tails.clear();
        owner.contacts.clear();
        owner.completed.clear();
        owner.batch.clear();
        owner.effect_stops.clear();
        owner.hull.clear();
    }
    world.insert_resource(owner);
    done
}
fn update_trail(
    world: &mut World,
    owner: &mut Projectiles,
    handle: crate::gameplay::component_state::entity::ComponentHandle,
    id: EntityId,
    dt: f32,
) -> Result<(), String> {
    use super::usable_effects::{EffectRequests, EntityPlacement};
    let runtime = world
        .resource::<super::physics_scene::PhysicsScene>()
        .projectile_runtime(handle)?;
    if runtime.flags.disable_trail
        || runtime.impact_stopped
        || owner.trails.get(&id).is_some_and(|trail| trail.stopped)
    {
        return Ok(());
    }
    let declaration = runtime.declaration.clone();
    let (timespace, remaining) = world
        .resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .entity_time_state(id)
        .map_err(str::to_owned)?;
    let suspended = declaration.switch_trail_on_time_suspended
        && timespace != 1
        && dt == 0.
        && remaining.unwrap_or(0.) <= 0.
        && world
            .get_resource::<super::power_world::PowerWorld>()
            .is_some_and(|power| power.bend_time_active());
    if owner
        .trails
        .get(&id)
        .is_some_and(|trail| trail.suspended == suspended)
    {
        return Ok(());
    }
    if let Some(mut old) = owner.trails.remove(&id) {
        world
            .resource_mut::<EffectRequests>()
            .stop_entity(old.key.clone());
        old.stopped = true;
        owner.trail_tails.push((id, old));
    }
    let path = if suspended {
        &declaration.effects.time_suspended_trail
    } else {
        &declaration.effects.trail
    };
    if let Some(path) = path {
        let name = world
            .resource::<crate::gameplay::entity_lifetime::LiveEntities>()
            .get(id)
            .ok_or("projectile trail owner stale")?
            .0
            .to_owned();
        let key: Arc<str> = Arc::from(format!("projectile_trail:{name}:{suspended}"));
        let placement = EntityPlacement {
            target: crate::gameplay::event_graph::EntityReference::Named(name),
            tag: String::new(),
            local: Default::default(),
            attached: true,
        };
        let ticket =
            world
                .resource_mut::<EffectRequests>()
                .start_entity(key.clone(), path, placement);
        owner.trails.insert(
            id,
            Trail {
                key,
                ticket,
                stopped: false,
                suspended,
            },
        );
    }
    Ok(())
}

///629590's player branch queries the actual current posture hull with a
///zero-extent point. It must not use the looser render bounds used by other
///actor classes. The host borrows the already constructed proxy hull.
fn overlaps_player(
    player: &crate::gameplay::player::Player,
    position: bevy_math::Vec3,
    hull: &[[f32; 4]],
) -> Result<bool, String> {
    let proxy = &player.physics.proxy;
    let local = proxy.rotation.transpose() * (position - proxy.position);
    let identity = [
        [1., 0., 0., 0.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ];
    Ok(crate::gameplay::convex_support::distance::query_point(
        [local.x, local.y, local.z, 0.],
        hull,
        identity,
        0.,
        0.,
        0.,
    )?
    .is_some())
}

/// The source's finite-flight component batch and its host consumers share a
///single tick. A fallible host callback resumes at its delivery cursor rather
///than simulating flight, damage, or RNG twice.
pub(super) fn update(world: &mut World) -> Result<(), String> {
    advance(world, true)
}

///627f60/628... accepted hit/handoff callbacks retain their own cursors
///independently of finite flight. Do not snapshot another live flight batch
///or integrate gravity/collision while completing these during exit.
pub(super) fn finish_accepted(world: &mut World) -> Result<bool, String> {
    advance(world, false)?;
    //Phase9 has forwarded accepted gameplay operations. A final trail tail
    //retains its real journal until FXshutdown acknowledges it; waiting for
    //that tail here would prevent the shutdown which ends the same tail.
    Ok(world.get_resource::<Projectiles>().is_none_or(|owner|
        owner.deliveries.values().all(|delivery|delivery.phase >= 9)))
}

///After actual FX shutdown, acknowledge terminal trails and enqueue their
///ordinary removals before component and managed identity destruction.
pub(super) fn finish_retirement(world: &mut World) -> Result<bool, String> {
    advance(world, false)?;
    Ok(world.get_resource::<Projectiles>().is_none_or(|owner|owner.deliveries.is_empty()))
}

fn advance(world: &mut World, simulate: bool) -> Result<(), String> {
    use super::{physics_scene::PhysicsScene, player::PlayerState, usable_effects::EffectRequests};
    use crate::gameplay::{
        entity_events::EntityEvents,
        entity_lifetime::LiveEntities,
        game_clock_runtime::ClockRuntime,
        projectiles::{Handoff, Response},
    };
    if !world.contains_resource::<PhysicsScene>() || !world.contains_resource::<Projectiles>() {
        return Ok(());
    }
    let mut owner = world.remove_resource::<Projectiles>().unwrap();
    let result = (|| {
        owner
            .trail_tails
            .retain(|(_, trail)| !super::usable_effects::entity_retired(world, &trail.ticket));
        owner
            .contacts
            .retain(|(_, ticket)| !super::usable_effects::entity_retired(world, ticket));
        //6289c0's real FX completion callback removes its actual projectile
        //only after the FX/particle/audio owner acknowledges the whole chain.
        owner.completed.clear();
        for (&id, impact) in &owner.impacts {
            if !impact.stopped && super::usable_effects::entity_retired(world, &impact.ticket) {
                owner.completed.push(id);
            }
        }
        for &id in &owner.completed {
            if world.resource::<LiveEntities>().get(id).is_some() {
                world.resource_mut::<LiveEntities>().request_removal(id)?;
            }
            owner.impacts.get_mut(&id).unwrap().stopped = true;
        }
        owner.batch.clear();
        owner.effect_stops.clear();
        if simulate {
            world.resource::<PhysicsScene>()
                .projectile_batch(&mut owner.batch, &mut owner.effect_stops)?;
        }
        for &(handle, id) in &owner.effect_stops {
            if let Some(trail) = owner.trails.get_mut(&id) {
                if !trail.stopped {
                    world
                        .resource_mut::<EffectRequests>()
                        .stop_entity(trail.key.clone());
                    trail.stopped = true;
                }
            }
            world
                .resource_mut::<PhysicsScene>()
                .acknowledge_projectile_effect_stop(handle)?;
        }
        if owner.batch.is_empty() && owner.deliveries.is_empty() {
            return Ok(());
        }
        // Host adapter: borrow one posture hull snapshot for this batch;
        //the source shape itself is already constructed by PlayerPhysics.
        owner.hull.clear();
        if let Some(player) = world
            .get_resource::<PlayerState>()
            .and_then(|state| state.player.as_ref())
        {
            owner.hull.extend(
                player
                    .physics
                    .proxy
                    .shape_vertices()
                    .iter()
                    .map(|v| [v.x, v.y, v.z, 0.]),
            );
        }
        for (&id, delivery) in &owner.deliveries {
            if !owner.batch.iter().any(|(_, entity)| *entity == id) {
                owner.batch.push((delivery.handle, id));
            }
        }
        'projectile: for index in 0..owner.batch.len() {
            let (handle, id) = owner.batch[index];
            if !owner.deliveries.contains_key(&id) {
                if !simulate { return Err("accepted projectile drain admitted a new flight tick".into()); }
                if world.resource::<LiveEntities>().stage(id)
                    != Some(crate::gameplay::entity_lifetime::Stage::Spawned)
                {
                    continue;
                }
                let dt = world
                    .resource::<ClockRuntime>()
                    .entity_delta(id)
                    .map_err(str::to_owned)?;
                update_trail(world, &mut owner, handle, id, dt)?;
                let gravity = world.resource::<PhysicsScene>().projectile_gravity()?;
                let (attacker, position, target, damage_type, sleep_dart) = {
                    let runtime = world
                        .resource::<PhysicsScene>()
                        .projectile_runtime(handle)?;
                    (
                        runtime.attacker,
                        runtime.position,
                        runtime.homing_target.clone(),
                        runtime
                            .declaration
                            .damage
                            .as_ref()
                            .map_or(0, |damage| damage.damage_type.0),
                        runtime.declaration.sleep_damage.is_some(),
                    )
                };
                let target = target
                    .filter(|target| {
                        world
                            .resource::<LiveEntities>()
                            .get(target.entity)
                            .is_some()
                    })
                    .map(|target| -> Result<bevy_math::Vec3, String> {
                        let scene = world.resource::<PhysicsScene>();
                        let frame = scene
                            .entity_frame(world.resource::<LiveEntities>(), target.entity)?
                            .ok_or("homing projectile target frame is absent")?;
                        //784e30 always starts with entity position. The
                        //authored offset is world-space and applies only to
                        //a successfully resolved current tag, never fallback.
                        if let Some(tag) = target.tag.as_deref().filter(|tag| !tag.is_empty()) {
                            if let (Some(rig), Some(matrices), Some((group, scale))) = (
                                scene.runtime_animation_rig(target.entity)?,
                                scene.runtime_animation_matrices(target.entity)?,
                                super::world_models::tag_model(
                                    world,
                                    world.resource::<LiveEntities>(),
                                    target.entity,
                                )?,
                            ) {
                                if let Some(joint) = super::joint_frames::current_tag_frame(
                                    rig, matrices, group, tag, scale, frame,
                                )? {
                                    return Ok(
                                        bevy_math::Vec3::from_array(joint.position) + target.offset
                                    );
                                }
                            }
                        }
                        Ok(bevy_math::Vec3::from_array(frame.position))
                    })
                    .transpose()?;
                let attacker_exists =
                    attacker.is_some_and(|id| world.resource::<LiveEntities>().get(id).is_some());
                let overlapping = if let Some(attacker) = attacker.filter(|_| attacker_exists) {
                    if let Some(player) = world
                        .get_resource::<PlayerState>()
                        .and_then(|state| state.player.as_ref())
                        .filter(|player| player.managed_entity() == Some(attacker))
                    {
                        overlaps_player(player, position, &owner.hull)?
                    } else {
                        super::runtime_entities::bounds(world, attacker)?.is_some_and(|bounds| {
                            (0..3).all(|axis| {
                                position[axis] >= bounds[0][axis]
                                    && position[axis] <= bounds[1][axis]
                            })
                        })
                    }
                } else {
                    false
                };
                let step =
                    world.resource_scope(|world, mut state: bevy::prelude::Mut<PlayerState>| {
                        let player = state
                            .player
                            .as_mut()
                            .ok_or("projectile batch has no shared gameplay RNG owner")?;
                        //624610; attribute-manager reflection1415f14b0 maps
                        //+80/+84 to player/enemy BOLT, +88/+8c to BULLET.
                        let tag = if damage_type & (1 << 8) != 0 {
                            Some(if attacker == player.managed_entity() {
                                "ARK_ATTRIBUTE_PLAYER_BOLT_DAMAGE_FACTOR"
                            } else {
                                "ARK_ATTRIBUTE_ENEMY_BOLT_DAMAGE_FACTOR"
                            })
                        } else if damage_type & (1 << 9) != 0 {
                            Some(if attacker == player.managed_entity() {
                                "ARK_ATTRIBUTE_PLAYER_BULLET_DAMAGE_FACTOR"
                            } else {
                                "ARK_ATTRIBUTE_ENEMY_BULLET_DAMAGE_FACTOR"
                            })
                        } else {
                            None
                        };
                        let damage_factor = if attacker_exists {
                            tag.and_then(|tag| player.attributes.0.get(tag))
                                .copied()
                                .unwrap_or(1.)
                        } else {
                            1.
                        };
                        let pickup_chance = (sleep_dart
                            && attacker.is_some()
                            && attacker == player.managed_entity())
                        .then(|| {
                            player
                                .attributes
                                .get("ARK_ATTRIBUTE_SLEEPDART_PICKUP_CHANCE")
                        });
                        world.resource_scope(
                            |world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                                world.resource_scope(
                                    |world, mut live: bevy::prelude::Mut<LiveEntities>| {
                                        world.resource_scope(
                                            |_, mut events: bevy::prelude::Mut<EntityEvents>| {
                                                scene.step_projectile(
                                                    &mut live,
                                                    &mut events,
                                                    handle,
                                                    dt,
                                                    gravity,
                                                    target,
                                                    damage_factor,
                                                    pickup_chance,
                                                    overlapping,
                                                    || player.inventory_random(),
                                                )
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    })?;
                if step.witness.is_some(){super::gameplay_action_diagnostics::projectile_contact(world,&step);}
                owner.deliveries.insert(
                    id,
                    Delivery {
                        handle,
                        step,
                        phase: 0,
                        handoff_phase: 0,
                        contact_cursor: 0,
                        contact: None,
                        recovery_disabled: false,
                        stick_recoverable: None,
                        stick_start_requested: None,
                        stick_children: None,
                    },
                );
            }
            let delivery = owner.deliveries.get_mut(&id).unwrap();
            if delivery.phase == 0 {
                if delivery.step.suppress_pickable && !delivery.recovery_disabled {
                    world.resource_scope(
                        |world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                            world.resource_scope(
                                |world, mut live: bevy::prelude::Mut<LiveEntities>| {
                                    scene.suppress_projectile_pickable(
                                        &mut live,
                                        &mut world.resource_mut::<EntityEvents>(),
                                        id,
                                    )
                                },
                            )
                        },
                    )?;
                    delivery.recovery_disabled = true;
                }
                if delivery.step.publish_motion {
                    world.resource_scope(
                        |world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                            scene.publish_projectile_motion(
                                world.resource::<LiveEntities>(),
                                id,
                                delivery.step.frame,
                                delivery.step.velocity.to_array(),
                            )
                        },
                    )?;
                }
                delivery.phase = 1;
            }
            if delivery.phase == 1 {
                if let Some(impulse) = delivery.step.contact_impulse {
                    world
                        .resource_mut::<PhysicsScene>()
                        .projectile_contact_impulse(
                            delivery
                                .step
                                .witness
                                .ok_or("projectile impulse lost its witness")?,
                            impulse,
                        )?;
                }
                delivery.phase = 2;
            }
            if delivery.phase == 2 {
                if delivery.handoff_phase == 0 {
                    if !matches!(delivery.step.handoff, Handoff::None) {
                        world
                            .resource_mut::<PhysicsScene>()
                            .detach_projectile_motion(handle)?;
                    }
                    delivery.handoff_phase = 1;
                }
                if delivery.handoff_phase == 1 {
                    if matches!(delivery.step.handoff, Handoff::Throwable) {
                        world.resource_scope(
                            |world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                                scene.publish_rigid_motion(
                                    world.resource::<LiveEntities>(),
                                    id,
                                    delivery.step.frame,
                                    delivery.step.velocity.to_array(),
                                    [0.; 3],
                                )
                            },
                        )?;
                    }
                    delivery.handoff_phase = 2;
                }
                if delivery.handoff_phase == 2 {
                    world.resource_scope(
                        |world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                            world.resource_scope(
                                |world, mut live: bevy::prelude::Mut<LiveEntities>| {
                                    world.resource_scope(
                                        |_, mut events: bevy::prelude::Mut<EntityEvents>| {
                                            match delivery.step.handoff {
                                                Handoff::None => Ok(()),
                                                Handoff::Throwable => scene.start_throwable(
                                                    &mut live,
                                                    id,
                                                    &mut events,
                                                ),
                                                Handoff::Stick => {
                                                    let hit = delivery
                                        .step
                                        .witness
                                        .ok_or("projectile sticking lost collision witness")?;
                                                    scene.stick_throwable(
                                        &mut live,
                                        id,
                                        delivery.step.frame,
                                        hit.target
                                            .body_id
                                            .ok_or("projectile sticking has no actual body")?,
                                        hit.target.shape_key,
                                        &mut events,
                                    )
                                                }
                                            }
                                        },
                                    )
                                },
                            )
                        },
                    )?;
                    if delivery.step.witness.is_some(){super::gameplay_action_diagnostics::projectile_attachment(world,id);}
                    delivery.handoff_phase = 3;
                }
                if matches!(delivery.step.handoff, Handoff::Stick) {
                    let prepared = world
                        .resource::<PhysicsScene>()
                        .projectile_prepared(handle)?;
                    if delivery.stick_recoverable.is_none() {
                        delivery.stick_recoverable = Some(
                            world
                                .resource::<PhysicsScene>()
                                .projectile_recoverable(id)?,
                        );
                    }
                    if delivery.handoff_phase == 3 {
                        if delivery.stick_recoverable.unwrap() {
                            let target = delivery
                                .step
                                .contact_entity
                                .ok_or("recoverable stuck projectile has no target entity")?;
                            world.resource_scope(
                                |world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                                    world.resource_scope(
                                        |world, mut live: bevy::prelude::Mut<LiveEntities>| {
                                            scene.register_stuck_projectile(
                                                &mut live,
                                                &mut world.resource_mut::<EntityEvents>(),
                                                id,
                                                target,
                                            )
                                        },
                                    )
                                },
                            )?;
                        }
                        delivery.handoff_phase = 4;
                    }
                    if delivery.handoff_phase == 4 {
                        //627f60 replaces the visual model after real sticking;
                        //sleep darts retain their pickup model if recoverable.
                        if prepared.declaration.sleep_damage.is_none()
                            || !delivery.stick_recoverable.unwrap()
                        {
                            if let Some(model) = &prepared.entity.stick_model {
                                if !super::world_models::replace(
                                    world,
                                    id,
                                    model.clone(),
                                    delivery.step.frame,
                                )? {
                                    continue;
                                }
                            }
                        }
                        delivery.handoff_phase = 5;
                    }
                    if delivery.handoff_phase == 5 {
                        //Only non-NPC contacts reach this adapter. Source627f60
                        //appends declared components to this same live actor.
                        if !super::runtime_entities::append_projectile_stick(
                            world,
                            id,
                            &prepared.entity,
                        )? {
                            continue;
                        }
                        delivery.handoff_phase = 6;
                    }
                }
                delivery.phase = 3;
            }
            if delivery.phase == 3 {
                if let Some(impulse) = delivery.step.impulse {
                    //629590 ->623540 posts FA to the projectile: three
                    //impulse floats followed by the real application point.
                    let hit = delivery
                        .step
                        .witness
                        .ok_or("projectile bounce lost its application point")?;
                    let mut payload = Vec::with_capacity(24);
                    for value in [
                        impulse.x,
                        impulse.y,
                        impulse.z,
                        hit.position[0],
                        hit.position[1],
                        hit.position[2],
                    ] {
                        payload.extend_from_slice(&value.to_le_bytes());
                    }
                    world.resource_mut::<EntityEvents>().post(
                        crate::gameplay::entity_events::Event {
                            id: 0xfa,
                            target: Some(id),
                            payload: crate::gameplay::entity_events::Payload::Bytes(payload),
                        },
                    );
                }
                delivery.phase = 4;
            }
            if delivery.phase == 4 {
                let name = world
                    .resource::<LiveEntities>()
                    .get(id)
                    .ok_or("projectile FX actor generation stale")?
                    .0
                    .to_owned();
                let declaration = world
                    .resource::<PhysicsScene>()
                    .projectile_runtime(handle)?
                    .declaration
                    .clone();
                if let Some(material) = &declaration.material {
                    if delivery.contact_cursor < delivery.step.contact_effects.len() {
                        let prepared = world
                            .resource::<PhysicsScene>()
                            .projectile_prepared(handle)?;
                        let scope = super::runtime_entities::contact_scope(world, &prepared.entity)
                            .ok_or("projectile contact source reservation absent")?;
                        let mut context = world
                            .resource::<PhysicsScene>()
                            .projectile_effect_context(world.resource::<LiveEntities>(), id)?;
                        context.contact_speed = delivery.step.contact_speed;
                        context.contact_mass = delivery.step.contact_mass;
                        context.contact_stimulus_enabled =
                            Some(delivery.step.contact_stimulus_enabled);
                        while delivery.contact_cursor < delivery.step.contact_effects.len() {
                            match super::explosions::post_contact_owned(
                                world,
                                &scope,
                                &mut delivery.contact,
                                material,
                                &delivery.step.contact_effects[delivery.contact_cursor],
                                delivery.step.hit_direction,
                                context,
                            )? {
                                super::explosions::ContactPost::Pending => continue 'projectile,
                                super::explosions::ContactPost::Complete(Some(ticket)) => owner.contacts.push((id, ticket)),
                                super::explosions::ContactPost::Complete(None) => {},
                            }
                            delivery.contact = None;
                            delivery.contact_cursor += 1;
                        }
                    }
                }
                if let Some(path) = &delivery.step.impact_fx {
                    if !owner.impacts.contains_key(&id) {
                        let hit = delivery
                            .step
                            .witness
                            .ok_or("projectile impact FX lost actual witness")?;
                        let context = world
                            .resource::<PhysicsScene>()
                            .projectile_effect_context(world.resource::<LiveEntities>(), id)?;
                        let key: Arc<str> = Arc::from(format!("projectile_impact:{name}"));
                        let ticket = super::physics_scene::explosion_effects::projectile_impact(
                            world,
                            key.clone(),
                            path,
                            bevy_math::Vec3::new(hit.position[0], hit.position[1], hit.position[2]),
                            bevy_math::Vec3::new(hit.normal[0], hit.normal[1], hit.normal[2]),
                            delivery.step.hit_direction,
                            context,
                        )?;
                        owner.impacts.insert(
                            id,
                            Trail {
                                key,
                                ticket,
                                stopped: false,
                                suspended: false,
                            },
                        );
                    }
                }
                if let Some(trail) = owner.trails.get_mut(&id) {
                    if !trail.stopped
                        && (delivery.step.retire
                            || declaration.stop_trail_on_hit && delivery.step.witness.is_some())
                    {
                        world
                            .resource_mut::<EffectRequests>()
                            .stop_entity(trail.key.clone());
                        trail.stopped = true;
                    }
                }
                if delivery.step.response == Response::Break {
                    if let Some(path) = &declaration.effects.break_fx {
                        //627390 publishes the break effect at the actual contact,
                        //with the entity's real axes and impact direction.
                        world.resource_mut::<EffectRequests>().start_world_once(
                            Arc::from(format!("projectile_break:{name}")),
                            path,
                            delivery.step.frame,
                        );
                    }
                }
                delivery.phase = 5;
            }
            if delivery.phase == 5 {
                if delivery.step.publish_motion || delivery.step.hidden {
                    let frame = if delivery.step.publish_motion {
                        delivery.step.frame
                    } else {
                        super::world_models::frame(world, id)
                            .ok_or("projectile hidden model lost frame")?
                    };
                    super::world_models::update(world, id, frame, !delivery.step.hidden)?;
                }
                delivery.phase = 6;
            }
            if delivery.phase == 6 {
                if delivery.step.remove_when_hidden {
                    let declaration = world
                        .resource::<PhysicsScene>()
                        .projectile_runtime(handle)?
                        .declaration
                        .clone();
                    super::entity_functions::enqueue_hidden_removal(
                        world,
                        id,
                        declaration.lifespan_after_hit,
                        declaration.lifespan_if_visible,
                        10,
                    )?;
                }
                delivery.phase = 7;
            }
            if delivery.phase == 7 {
                if delivery.step.stop_component {
                    if let Some(trail) = owner.trails.get_mut(&id) {
                        if !trail.stopped {
                            world
                                .resource_mut::<EffectRequests>()
                                .stop_entity(trail.key.clone());
                            trail.stopped = true;
                        }
                    }
                    world.resource_scope(
                        |world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                            world.resource_scope(
                                |world, mut live: bevy::prelude::Mut<LiveEntities>| {
                                    scene.request_projectile_stop(
                                        &mut live,
                                        &mut world.resource_mut::<EntityEvents>(),
                                        handle,
                                    )
                                },
                            )
                        },
                    )?;
                }
                delivery.phase = 8;
            }
            if delivery.phase == 8 {
                //629590 ends BOTH Stick(2) and Bounce(5) with624540.
                //The earlier Stick callback4c6b70 only binds; it does not
                //start the dormant Throwable or publish its query body.
                if matches!(delivery.step.handoff, Handoff::Stick) {
                    if delivery.stick_start_requested.is_none() {
                        delivery.stick_start_requested = Some(!world.resource::<PhysicsScene>()
                            .projectile_throwable_started(handle)?);
                    }
                    if delivery.stick_start_requested == Some(true) {
                        world.resource_scope(|world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                            world.resource_scope(|world, mut live: bevy::prelude::Mut<LiveEntities>| {
                                scene.start_throwable(&mut live, id, &mut world.resource_mut::<EntityEvents>())
                            })
                        })?;
                        //624540 returning false on the new request causes
                        //628df0(false), preserving actual child bindings.
                        if delivery.stick_children.is_none() {
                            delivery.stick_children = Some(world.resource::<PhysicsScene>()
                                .projectile_child_frames(world.resource::<LiveEntities>(), id, delivery.step.frame)?);
                        }
                        world.resource_scope(|world, mut scene: bevy::prelude::Mut<PhysicsScene>| {
                            world.resource_scope(|world, mut live: bevy::prelude::Mut<LiveEntities>| {
                                scene.publish_projectile_children(&mut live,
                                    &mut world.resource_mut::<EntityEvents>(),
                                    delivery.stick_children.as_mut().unwrap(), false)
                            })
                        })?;
                    }
                }
                delivery.phase = 9;
            }
            if delivery.step.retire {
                if owner.trails.get(&id).is_some_and(|trail| {
                    !super::usable_effects::entity_retired(world, &trail.ticket)
                }) {
                    continue;
                }
                //877870 enqueues removal; only the canonical snapshotted
                //release FIFO may prepare/release the real actor and slot.
                world.resource_mut::<LiveEntities>().request_removal(id)?;
            }
            owner.deliveries.remove(&id);
        }
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

pub(super) struct Component {
    pub owner: ComponentOwner,
    pub prepared: Arc<Prepared>,
    pub runtime: Option<Projectile>,
    pub request: Option<SpawnRequest>,
    pub throwable: Option<crate::gameplay::pickable_component::CheckedComponent>,
    pub reference_frame_registered: bool,
    pub movement_body: Option<usize>,
    pub effects_stop_pending: bool,
    pub deflect_listener: bool,
    pub(super) pickup: Option<Pickup>,
    pub hidden: bool,
    ray: WorldRayQuery,
    pub(super) hits: Vec<ShapeRayContact>,
    initial_hits: Vec<crate::gameplay::shape_convex_query::ConvexContact>,
}
pub(super) struct Pickup {
    pub children: Vec<PickupChild>,
    pub next: usize,
}
pub(super) struct PickupChild {
    pub entity: EntityId,
    pub frame: crate::gameplay::reference_frame::ReferenceFrame,
    pub unbound: bool,
}
impl Component {
    pub(super) fn new(
        prepared: Arc<Prepared>,
        bodies: &mut Vec<crate::gameplay::physics_actor::ActorBody>,
        free: &mut Vec<usize>,
    ) -> Result<Self, String> {
        //623730 constructs a distinct embedded reference owner. Reuse the
        //canonical movement-interface slots, never a projectile body table.
        let movement = if let Some(&index) = free.last() {
            let body = bodies
                .get(index)
                .ok_or("free projectile reference interface absent")?;
            if body.actor.is_some() || body.body_id & 0xffffff != 0xffffff {
                return Err("free projectile reference interface is still live".into());
            }
            free.pop();
            index
        } else {
            bodies
                .try_reserve(1)
                .map_err(|error| format!("projectile reference interface allocation: {error}"))?;
            let index = bodies.len();
            bodies.push(crate::gameplay::physics_actor::ActorBody {
                actor: None,
                body_id: 0xffffff,
            });
            index
        };
        Ok(Self {
            owner: Default::default(),
            prepared,
            runtime: None,
            request: None,
            throwable: None,
            reference_frame_registered: false,
            movement_body: Some(movement),
            effects_stop_pending: false,
            deflect_listener: false,
            pickup: None,
            hidden: false,
            ray: Default::default(),
            hits: Vec::new(),
            initial_hits: Vec::new(),
        })
    }
    ///626eb0 runs only after this actor's real identity and required Throwable
    ///component have been admitted by the canonical component factory.
    pub(super) fn initialize(
        &mut self,
        id: EntityId,
        request: SpawnRequest,
        throwable: crate::gameplay::pickable_component::CheckedComponent,
    ) -> Result<(), String> {
        if self.runtime.is_some() {
            return Err("projectile initialized twice".into());
        }
        self.runtime = Some(Projectile::from_request(
            self.prepared.declaration.clone(),
            id,
            request,
            self.prepared.bounds_max.x,
        ));
        self.throwable = Some(throwable);
        self.owner.mark_initialized();
        Ok(())
    }
    ///629590 retains the collector vector between actual physics updates;
    ///full body generation, material and shape witness survive sorting.
    pub(super) fn ray_hits<'a>(
        &'a mut self,
        world: &LevelPointQueries,
        point: &mut crate::gameplay::world_point_query::WorldPointQuery,
        start: bevy_math::Vec3,
        end: bevy_math::Vec3,
        overlapping_spawner: bool,
    ) -> Result<&'a [ShapeRayContact], String> {
        let mut layers = self.prepared.declaration.collision_layers;
        if overlapping_spawner {
            layers &= 0xfffffff8;
        }
        let request = WorldRayRequest {
            origin: start.to_array(),
            direction: (end - start).to_array(),
            maximum_fraction: 1.,
            hit: ShapeHitInfo {
                body_id: None,
                material_id: 0xffff,
                shape_key: u32::MAX,
                filter_info: 0,
                user_data: 0,
            },
            filter: Some(CollisionQueryFilter {
                layers,
                required_flags: 0x800,
            }),
        };
        world.ray_hits(&mut self.ray, &request, 2, &mut self.hits)?;
        let initial = crate::gameplay::world_point_query::WorldPointRequest {
            point: start.to_array(),
            hit: request.hit,
            filter: Some(CollisionQueryFilter {
                layers: self.prepared.declaration.collision_layers & 0xfffffff8,
                required_flags: 0x800,
            }),
        };
        world.point_hits(point, &initial, &mut self.initial_hits)?;
        self.hits
            .extend(self.initial_hits.iter().map(|hit| ShapeRayContact {
                target: hit.target,
                fraction: 0.,
                position: hit.position,
                normal: hit.normal,
                flags: hit.flags,
            }));
        self.hits.rotate_right(self.initial_hits.len());
        self.hits.sort_by(|a, b| a.fraction.total_cmp(&b.fraction));
        Ok(&self.hits)
    }
}
impl ComponentInstance for Component {
    fn owner(&self) -> &ComponentOwner {
        &self.owner
    }
    fn owner_mut(&mut self) -> &mut ComponentOwner {
        &mut self.owner
    }
}

pub(super) fn prepared_ready(world: &World, prepared: &Prepared) -> Result<bool, String> {
    if !world.contains_resource::<Projectiles>() {
        return Ok(false);
    }
    super::runtime_entities::ready(world, &prepared.entity)
}
pub(super) fn prepare_for(
    world: &mut World,
    owner: Arc<str>,
    prepared: &Prepared,
) -> Result<(), String> {
    super::runtime_entities::request_for(world, owner, prepared.entity.clone())
}
pub(super) fn release_prepared_for(
    world: &mut World,
    owner: &str,
    prepared: &Prepared,
) -> Result<(), String> {
    super::runtime_entities::release_for(world, owner, &prepared.entity)
}
pub(super) fn spawn(
    world: &mut World,
    prepared: Arc<Prepared>,
    request: SpawnRequest,
) -> Result<EntityId, String> {
    if !prepared_ready(world, &prepared)? {
        return Err("projectile source dependencies are not ready".into());
    }
    let options = super::runtime_entities::SpawnOptions {
        frame: request.frame,
        scale: None,
        hidden_disabled: false,
    };
    let sound = super::runtime_entities::fired_sound(world, &prepared.entity);
    let disable_trail = request.flags.disable_trail;
    let diagnostic_source=(request.attacker,request.equipment);
    let id = super::runtime_entities::spawn_projectile(
        world,
        prepared.entity.clone(),
        options,
        request,
    )?;
    super::gameplay_action_diagnostics::projectile_birth(world,id,diagnostic_source.0,diagnostic_source.1,Arc::as_ptr(&prepared.declaration)as usize);
    //627110 uses the actor's real optional Audio component. Its source and
    //banks were reserved during preparation; starting flight does no I/O.
    if let Some(event) = sound {
        if let Some(emitter) = world
            .resource::<super::physics_scene::PhysicsScene>()
            .projectile_audio_emitter(id)?
        {
            world.write_message(super::audio::SoundEvent(
                crate::gameplay::sound::Request::Post {
                    event: event.id,
                    emitter,
                },
            ));
        }
    }
    //627110 ->627c60 starts the trail at successful birth, including a
    //projectile that will hit something during its first finite update.
    if !disable_trail {
        if let Some(path) = &prepared.declaration.effects.trail {
            let name = world
                .resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .get(id)
                .ok_or("new projectile trail has no actual actor")?
                .0
                .to_owned();
            let key: Arc<str> = Arc::from(format!("projectile_trail:{name}"));
            let placement = super::usable_effects::EntityPlacement {
                target: crate::gameplay::event_graph::EntityReference::Named(name),
                tag: String::new(),
                local: Default::default(),
                attached: true,
            };
            let ticket = world
                .resource_mut::<super::usable_effects::EffectRequests>()
                .start_entity(key.clone(), path, placement);
            world.resource_mut::<Projectiles>().trails.insert(
                id,
                Trail {
                    key,
                    ticket,
                    stopped: false,
                    suspended: false,
                },
            );
        }
    }
    Ok(id)
}
