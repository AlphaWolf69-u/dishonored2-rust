//! Owned melee components, authored active frames and deferred query results.
//! 630180/6302a0 own dependencies,637160 submits,636b40 receives,631020
//! filters,634320 damages and635c90 posts the canonical material-contact FX.
use super::physics_scene::{
    PhysicsScene,
    melee::{Contact, Hit, MeleeQueries, Ray, Shape, Sweep},
};
use crate::gameplay::{
    collision_queries::QueryId,
    entity_events::EntityEvents,
    entity_lifetime::{EntityId, LiveEntities},
    player::{
        AnimationEvent,
        melee::{ActiveFrames, Declaration},
    },
    reference_frame::{ReferenceFrame, direction_basis},
    shape_ray_query::ShapeRayContact,
};
use bevy::prelude::*;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

struct ContactScope {
    material: String,
    key: Arc<str>,
}
struct Delivery {
    hit: ShapeRayContact,
    target: Option<EntityId>,
    attacker: EntityId,
    velocity: Vec3,
    direction: Vec3,
    focused: bool,
    factor: f32,
    recoil: bool,
    phase: u8,
    contact: Option<super::physics_scene::explosion_effects::Contact>,
}
struct Owned {
    identity: Arc<str>,
    entity: EntityId,
    descriptor: Arc<crate::assets::equipment::Descriptor>,
    definition: Arc<Declaration>,
    scopes: Vec<ContactScope>,
    requested: bool,
    ready: bool,
    retiring: bool,
    active: bool,
    stop_after_update: bool,
    forward_attack: bool,
    heavy_block: bool,
    factor: f32,
    previous: Option<ReferenceFrame>,
    queries: [QueryId; 4],
    shape: Option<Arc<Shape>>,
    hits: Vec<(Contact, bool)>,
    seen: Vec<Option<EntityId>>,
    deliveries: VecDeque<Delivery>,
    tails: Vec<super::usable_effects::EffectTicket>,
    world_hit_duration: f32,
    shape_length: f32,
}
#[derive(Resource, Default)]
pub(super) struct Melee {
    owned: HashMap<EntityId, Owned>,
    queries: MeleeQueries,
    generation: u64,
    error: Option<String>,
}

impl Melee {
    pub(super) fn write_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(report, "  melee owners={} query_work_pending={} error={:.240}", self.owned.len(), self.queries.has_pending(), self.error.as_deref().unwrap_or("<none>"));
        for entry in self.owned.values().take(4) {
            let _ = writeln!(report, "    entity={:?} ready={} requested={} retiring={} active={} stop_pending={} forward={} heavy_block={} damage_factor={} previous_frame={} hit_count={} seen_count={} contact_callbacks={} tails={} world_hit_duration={}", entry.entity, entry.ready, entry.requested, entry.retiring, entry.active, entry.stop_after_update, entry.forward_attack, entry.heavy_block, entry.factor, entry.previous.is_some(), entry.hits.len(), entry.seen.len(), entry.deliveries.len(), entry.tails.len(), entry.world_hit_duration);
            if let Some(delivery) = entry.deliveries.front() {
                let _ = writeln!(report, "      front_contact target={:?} phase={} focused={} recoil={} velocity={:?}", delivery.target, delivery.phase, delivery.focused, delivery.recoil, delivery.velocity);
            }
        }
    }
}

pub(super) fn request_owned(
    world: &mut World,
    identity: Arc<str>,
    descriptor: Arc<crate::assets::equipment::Descriptor>,
) -> Result<(), String> {
    let Some(definition) = descriptor.melee.clone() else {
        return Ok(());
    };
    let entity = super::equipment_identity::id(world, &identity)
        .ok_or("owned melee equipment identity is not admitted")?;
    if !world.contains_resource::<Melee>() {
        world.init_resource::<Melee>();
    }
    let mut owner = world.resource_mut::<Melee>();
    if owner.owned.contains_key(&entity) {
        return Ok(());
    }
    let mut shape = None;
    super::physics_scene::melee::prepare_shape(&mut shape, definition.dimensions)?;
    owner.generation = owner.generation.wrapping_add(1);
    let mut scopes = Vec::new();
    for material in definition.materials() {
        if scopes
            .iter()
            .any(|scope: &ContactScope| scope.material == material)
        {
            continue;
        }
        scopes.push(ContactScope {
            material: material.to_owned(),
            key: Arc::from(format!(
                "melee:{}:{}:{}",
                owner.generation,
                identity,
                scopes.len()
            )),
        });
    }
    let shape_length = definition.dimensions.x;
    owner.owned.insert(
        entity,
        Owned {
            identity,
            entity,
            descriptor,
            definition,
            scopes,
            requested: false,
            ready: false,
            retiring: false,
            active: false,
            stop_after_update: false,
            forward_attack: false,
            heavy_block: false,
            factor: 1.,
            previous: None,
            queries: [QueryId::INVALID; 4],
            shape,
            hits: Vec::new(),
            seen: Vec::new(),
            deliveries: VecDeque::new(),
            tails: Vec::new(),
            world_hit_duration: 0.,
            shape_length,
        },
    );
    Ok(())
}
pub(super) fn startup_pending(world: &World) -> bool {
    world.get_resource::<Melee>().is_some_and(|owner| {
        owner
            .owned
            .values()
            .any(|entry| !entry.retiring && !entry.ready)
    })
}
pub(super) fn startup_error(world: &World) -> Option<&str> {
    world
        .get_resource::<Melee>()
        .and_then(|owner| owner.error.as_deref())
}
pub(super) fn ready_owned(world: &World, identity: &str) -> Result<bool, String> {
    if let Some(error) = startup_error(world) {
        return Err(error.to_owned());
    }
    Ok(world
        .get_resource::<Melee>()
        .and_then(|owner| {
            owner
                .owned
                .values()
                .find(|entry| entry.identity.as_ref() == identity)
        })
        .is_some_and(|entry| entry.ready && !entry.retiring))
}
pub(super) fn remove_owned(world: &mut World, identity: &str) {
    if let Some(mut owner) = world.get_resource_mut::<Melee>() {
        for entry in owner
            .owned
            .values_mut()
            .filter(|entry| entry.identity.as_ref() == identity)
        {
            entry.retiring = true;
            entry.active = false;
            entry.ready = false;
        }
    }
}
pub(super) fn retired(world: &World, identity: &str) -> bool {
    world.get_resource::<Melee>().is_none_or(|owner| {
        !owner
            .owned
            .values()
            .any(|entry| entry.identity.as_ref() == identity)
    })
}

pub(super) fn handle_animation_event(
    world: &mut World,
    event: &AnimationEvent,
    equipment: [Option<EntityId>; 2],
    bound: &[EntityId],
) -> Result<(), String> {
    if event.class != "arkMidnightKeyPropTimedEvent_WeaponActiveFrames" {
        return Ok(());
    }
    let Some(active) = event.active else {
        return Err("weapon active frames arrived without a timed-event phase".into());
    };
    let settings = ActiveFrames::read(&event.properties)?;
    let Some(mut owner) = world.get_resource_mut::<Melee>() else {
        return Ok(());
    };
    for entry in owner.owned.values_mut().filter(|entry| !entry.retiring) {
        let receives = match event.target {
            0 => bound.contains(&entry.entity),
            1 | 2 => equipment[event.target - 1] == Some(entry.entity),
            _ => false,
        };
        if !receives {
            continue;
        }
        if !settings.targets(entry.descriptor.equipment.slot) {
            continue;
        }
        if active {
            if !entry.ready {
                return Err("melee active frames reached an unprepared owned component".into());
            }
            entry.active = true;
            entry.forward_attack = settings.forward_attack;
            entry.heavy_block = settings.heavy_block;
            entry.factor = settings.damage_factor;
            entry.stop_after_update = false;
        } else if entry.active {
            entry.stop_after_update = true;
        }
    }
    Ok(())
}

fn witness(hit: Hit) -> ShapeRayContact {
    match hit {
        Hit::Ray(hit) => hit,
        Hit::Shape(hit) => ShapeRayContact {
            target: hit.target,
            fraction: hit.fraction,
            position: hit.position,
            normal: hit.normal,
            flags: hit.flags,
        },
    }
}

///636630 validates the swept witness against the first real occluder.
///The player supplies XY and the weapon supplies Z; target contacts are
///biased inward by the source .01 before extending to the active shape length.
fn unobstructed(
    world: &World,
    queries: &mut MeleeQueries,
    equipment: EntityId,
    attacker: EntityId,
    frame: ReferenceFrame,
    length: f32,
    target: Option<EntityId>,
    raw: ShapeRayContact,
    player_origin: Vec3,
) -> Result<Option<ShapeRayContact>, String> {
    let from = Vec3::new(player_origin.x, player_origin.y, frame.position[2]);
    let mut to = Vec3::new(raw.position[0], raw.position[1], raw.position[2]);
    if target.is_some() {
        to -= Vec3::new(raw.normal[0], raw.normal[1], raw.normal[2]) * f32::from_bits(0x3c23_d70a);
    }
    let ray = Ray {
        origin: from.to_array(),
        direction: ((to - from).normalize_or_zero() * length).to_array(),
        maximum_fraction: 1.,
        layers: 0x443e,
    };
    let contacts = world.resource::<PhysicsScene>().melee_ray_contacts(
        world.resource::<LiveEntities>(),
        queries,
        &ray,
    )?;
    for contact in contacts {
        if contact.entity == Some(attacker) || contact.entity == Some(equipment) {
            continue;
        }
        if contact.entity != target {
            return Ok(None);
        }
        return Ok(Some(witness(contact.hit)));
    }
    Ok(Some(raw))
}

fn deliver(world: &mut World, entry: &mut Owned) -> Result<(), String> {
    while let Some(delivery) = entry.deliveries.front_mut() {
        let attacker = delivery.attacker;
        if delivery.phase == 0 {
            //635140 ->8519d0: the actual contact witness receives a point
            //impulse; its vertical component uses the source absolute value.
            if !delivery.recoil
                && world
                    .resource::<PhysicsScene>()
                    .melee_body_dynamic(delivery.hit.target)?
            {
                let mut impulse = delivery.velocity.normalize_or_zero() * entry.definition.impulse;
                impulse.z = impulse.z.abs();
                world
                    .resource_mut::<PhysicsScene>()
                    .projectile_contact_impulse(delivery.hit, impulse)?;
            }
            delivery.phase = 1;
        }
        if delivery.phase == 1 {
            let declaration = if delivery.focused {
                entry
                    .definition
                    .focused_damage
                    .as_ref()
                    .or(entry.definition.damage.as_ref())
            } else {
                entry.definition.damage.as_ref()
            };
            if let Some(declaration) = declaration.filter(|_| !delivery.recoil) {
                let region = world
                    .resource::<PhysicsScene>()
                    .melee_hit_region(delivery.hit.target)?;
                let mut parameters =
                    crate::gameplay::damage::Parameters::from_declaration(declaration);
                let factor = world
                    .get_resource::<super::player::PlayerState>()
                    .and_then(|state| state.player.as_ref())
                    .and_then(|player| {
                        player
                            .attributes
                            .0
                            .get("ARK_ATTRIBUTE_PLAYER_MELEE_WEAPON_DAMAGE_TO_HEALTH_FACTOR")
                            .copied()
                    })
                    .unwrap_or(1.);
                parameters.set_amount(
                    declaration,
                    delivery.factor * factor * declaration.maximum,
                    false,
                );
                parameters.hit_location = Some(Vec3::new(
                    delivery.hit.position[0],
                    delivery.hit.position[1],
                    delivery.hit.position[2],
                ));
                parameters.hit_normal = Some(Vec3::new(
                    delivery.hit.normal[0],
                    delivery.hit.normal[1],
                    delivery.hit.normal[2],
                ));
                parameters.hit_direction = delivery.direction;
                parameters.hit_region = region;
                parameters.affected_regions = 1 << region;
                parameters.attacker = Some(attacker);
                parameters.inflictor = Some(entry.entity);
                world.resource_scope(|world, mut scene: Mut<PhysicsScene>| {
                    world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                        scene
                            .damage_collision_target(
                                &mut live,
                                &mut world.resource_mut::<EntityEvents>(),
                                delivery.hit.target.user_data,
                                &parameters,
                            )
                            .map(|_| ())
                    })
                })?;
            }
            delivery.phase = 2;
        }
        if delivery.phase == 2 {
            let material = if delivery.focused {
                entry
                    .definition
                    .focused_material
                    .as_ref()
                    .or(entry.definition.material.as_ref())
            } else {
                entry.definition.material.as_ref()
            };
            if let Some(material) = material {
                let scope = entry
                    .scopes
                    .iter()
                    .find(|scope| scope.material == *material)
                    .ok_or("melee contact scope missing")?;
                let flags = world
                    .resource::<PhysicsScene>()
                    .melee_entity_flags(entry.entity)?;
                let context = crate::rendering::bevy_backend::world_effects::EntityEffectContext {
                    owner: Some(entry.entity),
                    culprit: Some(attacker),
                    cannot_incapacitate_player_and_allies: flags & 1 != 0,
                    player_impact: delivery.target == Some(attacker),
                    velocities: [delivery.velocity.to_array(), [0.; 3]],
                    contact_speed: delivery.velocity.length(),
                    contact_mass: 1.,
                    contact_stimulus_enabled: Some(true),
                };
                match super::explosions::post_contact_owned(
                    world,
                    &scope.key,
                    &mut delivery.contact,
                    material,
                    &delivery.hit,
                    delivery.velocity.normalize_or_zero(),
                    context,
                )? {
                    super::explosions::ContactPost::Pending => return Ok(()),
                    super::explosions::ContactPost::Complete(Some(ticket)) => entry.tails.push(ticket),
                    super::explosions::ContactPost::Complete(None) => {},
                }
            }
            delivery.phase = 3;
        }
        if delivery.phase == 3 && delivery.recoil {
            if let Some(player) = world
                .get_resource_mut::<super::player::PlayerState>()
                .as_mut()
                .and_then(|state| state.player.as_mut())
            {
                player.machine.request_melee_recoil(1);
            }
            delivery.phase = 4;
        }
        entry.deliveries.pop_front();
    }
    Ok(())
}

///Complete source634320/635c90 callbacks already accepted by a real query.
///This does not sample a blade, submit queries or advance another attack.
pub(super) fn finish_accepted(world: &mut World) -> Result<bool, String> {
    let Some(mut owner)=world.remove_resource::<Melee>() else{return Ok(true);};
    let result=(||{
        for entry in owner.owned.values_mut(){deliver(world,entry)?;}
        Ok(owner.owned.values().all(|entry|entry.deliveries.is_empty()))
    })();
    world.insert_resource(owner);result
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<Melee>() else {
        return Ok(());
    };
    let result = update_owned(world, &mut owner);
    if let Err(error) = &result {
        owner.error = Some(error.clone());
    }
    world.insert_resource(owner);
    result
}

fn update_owned(world: &mut World, owner: &mut Melee) -> Result<(), String> {
    let scene = world.get_resource::<PhysicsScene>();
    if owner.queries.has_pending() {
        scene
            .ok_or("melee query world missing")?
            .resolve_melee_queries(&mut owner.queries)?;
    }
    // Dependency/tail retirement proceeds with a paused gameplay clock too.
    for entry in owner.owned.values_mut() {
        entry
            .tails
            .retain(|ticket| !super::usable_effects::entity_retired(world, ticket));
        if entry.retiring {
            for bank in [2, 1, 0, 3] {
                let id = &mut entry.queries[bank];
                owner.queries.cancel(id)?;
            }
            deliver(world, entry)?;
            continue;
        }
        if !entry.requested {
            for scope in &entry.scopes {
                super::explosions::prepare_contact_owned(
                    world,
                    scope.key.clone(),
                    &scope.material,
                )?;
            }
            entry.requested = true;
        }
        entry.ready = true;
        for scope in &entry.scopes {
            entry.ready &=
                super::explosions::ready_contact_owned(world, &scope.key, &scope.material)?;
        }
    }
    let snapshot = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
        .and_then(|player| {
            let attacker = player.managed_entity()?;
            let (eye, camera) = player.published_camera_frame();
            Some((
                attacker,
                player.physics.position(),
                eye,
                camera,
                player.machine.melee_focused(),
            ))
        });
    if let Some((attacker, origin, eye, camera, focused)) = snapshot {
        for entry in owner
            .owned
            .values_mut()
            .filter(|entry| !entry.retiring && entry.ready)
        {
            let Some((current, _)) = super::equipment_identity::frame(world, entry.entity) else {
                continue;
            };
            let dt = world
                .resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                .entity_delta(entry.entity)
                .map_err(str::to_owned)?;
            if dt <= 0. {
                continue;
            }
            let previous = entry.previous.unwrap_or(current);
            let velocity =
                (Vec3::from_array(current.position) - Vec3::from_array(previous.position)) / dt;
            entry.hits.clear();
            for bank in [2, 1, 0, 3] {
                let id = &mut entry.queries[bank];
                let scene = world.resource::<PhysicsScene>();
                let live = world.resource::<LiveEntities>();
                if let Some(hits) = scene.poll_melee_query(live, &mut owner.queries, id)? {
                    for hit in hits {
                        if hit.entity == Some(attacker)
                            || hit.entity == Some(entry.entity)
                            || entry.seen.contains(&hit.entity)
                        {
                            continue;
                        }
                        if let Some(existing) = entry
                            .hits
                            .iter_mut()
                            .find(|(existing, _)| existing.entity == hit.entity)
                        {
                            if hit.hit.distance_parameter() < existing.0.hit.distance_parameter() {
                                existing.0 = *hit;
                            }
                        } else {
                            entry.hits.push((*hit, bank == 0 || bank == 3));
                        }
                    }
                }
            }
            if entry.active {
                for (hit, world_hit) in &entry.hits {
                    //631020 damages entity-bank contacts before considering
                    //a hard world impact; an admitted entity hit suppresses it.
                    if *world_hit && !entry.seen.is_empty() {
                        continue;
                    }
                    let raw_witness = witness(hit.hit);
                    let Some(witness) = unobstructed(
                        world,
                        &mut owner.queries,
                        entry.entity,
                        attacker,
                        current,
                        entry.shape_length,
                        hit.entity,
                        raw_witness,
                        origin,
                    )?
                    else {
                        continue;
                    };
                    let point = Vec3::new(
                        witness.position[0],
                        witness.position[1],
                        witness.position[2],
                    );
                    if !entry.definition.world_focus.contains(
                        origin,
                        eye,
                        camera,
                        entry.definition.focus_step_back,
                        point,
                    ) {
                        continue;
                    }
                    let damage = if focused {
                        entry
                            .definition
                            .focused_damage
                            .as_deref()
                            .or(entry.definition.damage.as_deref())
                    } else {
                        entry.definition.damage.as_deref()
                    };
                    let recoil = world.resource::<PhysicsScene>().melee_world_blocks(
                        world.resource::<LiveEntities>(),
                        witness.target,
                        hit.entity,
                        attacker,
                        damage,
                    )?;
                    if recoil {
                        //631020 waits the actual hard-contact dwell (.09s),
                        //except a forward attack which requests recoil now.
                        entry.world_hit_duration += dt;
                        if !entry.forward_attack
                            && entry.world_hit_duration < f32::from_bits(0x3db8_51ed)
                        {
                            continue;
                        }
                    }
                    entry.deliveries.push_back(Delivery {
                        hit: witness,
                        target: hit.entity,
                        attacker,
                        velocity,
                        direction: (Vec3::from_array(current.position) - point).normalize_or_zero(),
                        focused,
                        factor: entry.factor,
                        recoil,
                        phase: 0,
                        contact: None,
                    });
                    entry.seen.push(hit.entity);
                    if recoil {
                        entry.active = false;
                        entry.stop_after_update = false;
                        entry.seen.clear();
                        entry.world_hit_duration = 0.;
                        break;
                    }
                }
            }
            deliver(world, entry)?;
            if entry.stop_after_update {
                entry.active = false;
                entry.stop_after_update = false;
                entry.seen.clear();
                entry.world_hit_duration = 0.;
            }
            if entry.active {
                submit(
                    world,
                    &mut owner.queries,
                    entry,
                    previous,
                    current,
                    origin,
                    eye,
                    camera,
                )?;
            }
            entry.previous = Some(current);
        }
    }
    let mut ready = Vec::new();
    for (&id, entry) in &owner.owned {
        if entry.retiring
            && entry.tails.is_empty()
            && entry.deliveries.is_empty()
            && super::audio::contact_callbacks_complete(world, id)
        {
            ready.push(id);
        }
    }
    for id in ready {
        let entry = owner.owned.remove(&id).expect("retiring melee owner");
        for scope in entry.scopes {
            super::explosions::release_owned(world, &scope.key)?;
        }
    }
    Ok(())
}

fn submit(
    world: &World,
    queries: &mut MeleeQueries,
    entry: &mut Owned,
    previous: ReferenceFrame,
    current: ReferenceFrame,
    origin: Vec3,
    eye: Vec3,
    camera: Mat3,
) -> Result<(), String> {
    let sweep = crate::gameplay::player::melee::sweep_frame(
        &entry.definition,
        previous,
        current,
        origin,
        eye,
        camera,
        entry.forward_attack,
    );
    entry.shape_length = sweep.shape_length;
    let dimensions = Vec3::new(
        sweep.shape_length,
        entry.definition.dimensions.y,
        entry.definition.dimensions.z,
    );
    let shape = super::physics_scene::melee::prepare_shape(&mut entry.shape, dimensions)?;
    let basis = direction_basis(sweep.blade_direction);
    let axes: [[f32; 4]; 3] =
        std::array::from_fn(|axis| [basis.col(axis).x, basis.col(axis).y, basis.col(axis).z, 0.]);
    let transform = [
        axes[0],
        axes[1],
        axes[2],
        [sweep.from.x, sweep.from.y, sweep.from.z, 0.],
    ];
    // ShapeMatrix stores the same world-space source basis and translation.
    let delta = sweep.to - sweep.from;
    let distance = delta.length();
    let direction = delta.normalize_or_zero().to_array();
    for (index, layers) in [0x443e & 0x27, 0x443e & 0x4480, 0x443e & 0xbb58]
        .into_iter()
        .enumerate()
    {
        entry.queries[index] = queries.submit_shape(
            Sweep {
                shape: shape.clone(),
                transform,
                direction,
                maximum_distance: distance,
                layers,
            },
            world.contains_resource::<PhysicsScene>(),
        )?;
    }
    entry.queries[3] = queries.submit_ray(
        Ray {
            origin: sweep.to.to_array(),
            direction: (sweep.blade_direction * sweep.shape_length).to_array(),
            maximum_fraction: 1.,
            layers: 0x443e & 0xffffbb7f,
        },
        world.contains_resource::<PhysicsScene>(),
    )?;
    Ok(())
}

pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    if let Some(mut owner) = world.get_resource_mut::<Melee>() {
        for entry in owner.owned.values_mut() {
            entry.retiring = true;
            entry.active = false;
            entry.ready = false;
        }
    }
    update(world)?;
    let done = world
        .get_resource::<Melee>()
        .is_none_or(|owner| owner.owned.is_empty() && !owner.queries.has_pending());
    if done {
        world.remove_resource::<Melee>();
    }
    Ok(done)
}
