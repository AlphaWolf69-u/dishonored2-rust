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
#[derive(Clone, Copy, PartialEq, Eq)]
enum ContactPass {
    World,
    Entity,
    ///631020 temporary entity list: reverse extraction, second focus pass.
    EntityWorldFocus,
}
#[derive(Clone, Copy)]
struct ContactFrame {
    weapon: ReferenceFrame,
    length: f32,
    origin: Vec3,
    eye: Vec3,
    camera: Mat3,
    forward: bool,
}
struct Delivery {
    world: bool,
    raw_hit: ShapeRayContact,
    raw_target: Option<EntityId>,
    world_receipt: Option<u64>,
    hit: ShapeRayContact,
    target: Option<EntityId>,
    attacker: EntityId,
    velocity: Vec3,
    direction: Vec3,
    focused: bool,
    factor: f32,
    recoil: bool,
    damageable: bool,
    frame: ContactFrame,
    effect_hit: Option<ShapeRayContact>,
    contact_enabled: bool,
    phase: u8,
    contact: Option<super::physics_scene::explosion_effects::Contact>,
}
#[derive(Default)]
struct WorldContacts {
    serial: u64,
    last: Option<WorldContact>,
}
struct WorldContact {
    serial: u64,
    equipment: EntityId,
    window: u64,
    activation_player_time: f64,
    callback_entity_time: f64,
    callback_frame: Option<u32>,
    request_app_frame: Option<u32>,
    raw: (Option<EntityId>, crate::gameplay::collision_hit::ShapeHitInfo),
    selected: (Option<EntityId>, crate::gameplay::collision_hit::ShapeHitInfo),
    focused: bool,
    forward: bool,
    source_flags: u32,
    target_flags: Option<u32>,
    recoil_requested: bool,
    result: &'static str,
    ticket: Option<super::usable_effects::EffectTicket>,
}
/// Retained receipts for F8/action diagnostics, not another attack history.
/// No formatting, allocation or geometry copying occurs while recording.
#[derive(Default, Debug)]
struct Receipts {
    windows: u64,
    queries: u64,
    completed: u64,
    raw_contacts: u64,
    eligible_contacts: u64,
    retargeted: u64,
    outside_focus: u64,
    world_dwell: u64,
    accepted: u64,
    damage: u64,
    damage_accepted: u64,
    health_damage: f32,
    contact_fx: u64,
    contact_tickets: u64,
    last_damage_target: Option<EntityId>,
    last_damage_serial: u64,
    last_damage_accepted: bool,
    last_health_damage: f32,
    last_damage_amount: f32,
    last_damage_factor: f32,
    last_damage_focused: bool,
    last_damage_forward: bool,
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
    hits: Vec<(Contact, ContactPass)>,
    seen: Vec<Option<EntityId>>,
    deliveries: VecDeque<Delivery>,
    tails: Vec<super::usable_effects::EffectTicket>,
    world_hit_duration: f32,
    shape_length: f32,
    receipts: Receipts,
    last_sweep: Option<([f32; 3], [f32; 3], [f32; 3], f32)>,
    physics_base: [i32; 2],
    npc_range: [i32; 2],
    activation_player_time: f64,
}
#[derive(Resource, Default)]
pub(super) struct Melee {
    owned: HashMap<EntityId, Owned>,
    queries: MeleeQueries,
    generation: u64,
    error: Option<String>,
    active_frame_events: u64,
    last_contact: Option<(Option<EntityId>, crate::gameplay::collision_hit::ShapeHitInfo)>,
    world_contacts: WorldContacts,
}

impl Melee {
    /// Last accepted physical callback, including non-damageable world/props.
    /// F8 borrows this single receipt; it does not run another collision query.
    pub(super) fn diagnostic_last_contact(&self) -> Option<(Option<EntityId>, crate::gameplay::collision_hit::ShapeHitInfo)> {
        self.last_contact
    }
    pub(super) fn write_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(report, "  melee owners={} query_work_pending={} active_frame_events={} error={:.240}", self.owned.len(), self.queries.has_pending(), self.active_frame_events, self.error.as_deref().unwrap_or("<none>"));
        for entry in self.owned.values().take(4) {
            let _ = writeln!(report, "    entity={:?} ready={} requested={} retiring={} active={} stop_pending={} forward={} heavy_block={} damage_factor={} previous_frame={} hit_count={} seen_count={} contact_callbacks={} tails={} world_hit_duration={}", entry.entity, entry.ready, entry.requested, entry.retiring, entry.active, entry.stop_after_update, entry.forward_attack, entry.heavy_block, entry.factor, entry.previous.is_some(), entry.hits.len(), entry.seen.len(), entry.deliveries.len(), entry.tails.len(), entry.world_hit_duration);
            let _ = writeln!(report, "      receipts={:?} last_sweep={:?}", entry.receipts, entry.last_sweep);
            let _ = writeln!(report, "      damage_input amount={} sword_factor={} target={:?} accepted={} health_damage={} focused={} forward={}",
                entry.receipts.last_damage_amount, entry.receipts.last_damage_factor,
                entry.receipts.last_damage_target, entry.receipts.last_damage_accepted,
                entry.receipts.last_health_damage, entry.receipts.last_damage_focused,
                entry.receipts.last_damage_forward);
            if let Some(delivery) = entry.deliveries.front() {
                let _ = writeln!(report, "      front_contact target={:?} phase={} focused={} recoil={} velocity={:?}", delivery.target, delivery.phase, delivery.focused, delivery.recoil, delivery.velocity);
            }
        }
    }
}

/// Actual action-smoke receipts: no synthesized attack, collision or health.
pub(super) fn active_windows(world: &World) -> u64 {
    world.get_resource::<Melee>().map_or(0, |owner|
        owner.owned.values().map(|entry| entry.receipts.windows).sum())
}
pub(super) fn action_receipt(world: &World, target: EntityId) -> Option<(u64, bool, f32)> {
    world.get_resource::<Melee>()?.owned.values().find_map(|entry| {
        let receipt = &entry.receipts;
        (receipt.last_damage_target == Some(target)).then_some((receipt.last_damage_serial,
            receipt.last_damage_accepted, receipt.last_health_damage))
    })
}
pub(super) fn focused_action_receipt(world: &World, target: EntityId)
    -> Option<(u64, bool, f32, bool, bool)> {
    world.get_resource::<Melee>()?.owned.values().find_map(|entry| {
        let receipt = &entry.receipts;
        (receipt.last_damage_target == Some(target)).then_some((receipt.last_damage_serial,
            receipt.last_damage_accepted, receipt.last_health_damage,
            receipt.last_damage_focused, receipt.last_damage_forward))
    })
}

///Actual last WORLD callback, for the same production-input smoke path.
///The final bool means a real FX request ticket exists, not GPU/audio ready.
pub(super) fn world_action_receipt(world: &World)
    -> Option<(u64, crate::gameplay::collision_hit::ShapeHitInfo, bool, bool, bool, bool)> {
    let receipt = world.get_resource::<Melee>()?.world_contacts.last.as_ref()?;
    Some((receipt.serial, receipt.selected.1, receipt.focused, receipt.forward,
        receipt.recoil_requested, receipt.ticket.is_some()))
}

/// Clone only the weak request receipt; diagnostics cannot prolong its tail.
pub(super) fn world_effect_ticket(world: &World) -> Option<super::usable_effects::EffectTicket> {
    world.get_resource::<Melee>()?.world_contacts.last.as_ref()?.ticket.clone()
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
    if world.get_resource::<Melee>().is_some_and(|owner| owner.owned.contains_key(&entity)) {
        return Ok(());
    }
    if !world.contains_resource::<Melee>() {
        world.init_resource::<Melee>();
    }
    let physics_base = world.resource::<PhysicsScene>().melee_physics_base_range()?;
    let npc_range = world.resource::<PhysicsScene>().melee_npc_range()?;
    let mut owner = world.resource_mut::<Melee>();
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
            receipts: Receipts::default(),
            last_sweep: None,
            physics_base,
            npc_range,
            activation_player_time: 0.,
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
pub(super) fn debug_startup_pending(world:&World)->String {
    use std::fmt::Write;
    let Some(owner)=world.get_resource::<Melee>()else{return "melee owner=absent".into();};
    let mut report=String::new();owner.write_diagnostics(&mut report);
    if let Some(contact) = &owner.world_contacts.last {
        let _ = writeln!(report,
            "    last_WORLD_contact serial={} equipment={:?} window={} activation_player_time={} callback_entity_time={} callback_app_frame={:?} request_app_frame={:?} focused={} forward={} protected_target=None raw={:?} selected={:?} source_D2={:#04x} target_D3={:?} recoil_requested_before_FX={} result={}",
            contact.serial, contact.equipment, contact.window, contact.activation_player_time,
            contact.callback_entity_time, contact.callback_frame, contact.request_app_frame, contact.focused, contact.forward, contact.raw,
            contact.selected, (contact.source_flags >> 16) as u8,
            contact.target_flags.map(|flags| (flags >> 24) as u8),
            contact.recoil_requested, contact.result);
        if let Some(ticket) = &contact.ticket {
            let _ = writeln!(report, "      WORLD_FX {}", super::usable_effects::debug_ticket(world, ticket));
        }
    }
    for entry in owner.owned.values(){for scope in &entry.scopes{
        let _=writeln!(report,"    identity={} {}",entry.identity,super::explosions::debug_owned(world,&scope.key));
    }}report
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
    let activation_player_time = world.resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .player_time().map_err(str::to_owned)?;
    let Some(mut owner) = world.get_resource_mut::<Melee>() else {
        return Ok(());
    };
    owner.active_frame_events += 1;
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
            entry.activation_player_time = activation_player_time;
            entry.receipts.windows += 1;
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
            //633e50 calls81aa80(...,true): initial penetration4 uses
            //point+normal*distance.6340f0/81b1a0 uses the raw point for2.
            position: if hit.flags == 4 {
                std::array::from_fn(|axis| hit.position[axis] + hit.normal[axis] * hit.distance)
            } else { hit.position },
            normal: hit.normal,
            flags: hit.flags,
        },
    }
}

///630890 calls636630 to replace the swept witness with the first real hit,
///ignores its rejection return, then optionally selects the forward ray.
///The player supplies XY and the weapon supplies Z; target contacts are
///biased inward by .01 only for634320's explicit protected entity;631020
///passes a null protected entity for WORLD packets.
fn player_contact(
    world: &World,
    queries: &mut MeleeQueries,
    equipment: EntityId,
    attacker: EntityId,
    frame: ReferenceFrame,
    length: f32,
    target: Option<EntityId>,
    protected: Option<EntityId>,
    raw: ShapeRayContact,
    player_origin: Vec3,
    eye: Vec3,
    camera: Mat3,
    forward_attack: bool,
    physics_base: [i32; 2],
) -> Result<(ShapeRayContact, Option<EntityId>), String> {
    let from = Vec3::new(player_origin.x, player_origin.y, frame.position[2]);
    let mut to = Vec3::new(raw.position[0], raw.position[1], raw.position[2]);
    if protected.is_some() {
        to -= Vec3::new(raw.normal[0], raw.normal[1], raw.normal[2]) * f32::from_bits(0x3c23_d70a);
    }
    let ray = Ray {
        origin: from.to_array(),
        direction: ((to - from).normalize_or_zero() * length).to_array(),
        maximum_fraction: 1.,
        layers: 0x443e,
    };
    let scene = world.resource::<PhysicsScene>();
    let live = world.resource::<LiveEntities>();
    let contacts = scene.melee_ray_contacts(
        live,
        queries,
        &ray,
    )?;
    let mut selected = (raw, target);
    for contact in contacts {
        if contact.entity == Some(attacker) || contact.entity == Some(equipment) {
            continue;
        }
        let mut hit = witness(contact.hit);
        if protected.is_some() && contact.entity == protected {
            //636630 still publishes the ray position/normal/body, but the
            //protected target keeps the original collision/material pair.
            hit.target.user_data = raw.target.user_data;
            hit.target.material_id = raw.target.material_id;
        }
        selected = (hit, contact.entity);
        break;
    }
    if forward_attack {
        let ray = Ray { origin: eye.to_array(), direction: (camera.x_axis * length).to_array(),
            maximum_fraction: 1., layers: 0x443e };
        for contact in scene.melee_ray_contacts(live, queries, &ray)? {
            if contact.entity == Some(attacker) { continue; }
            if scene.melee_forward_candidate(live, contact.entity, physics_base)? {
                selected = (witness(contact.hit), contact.entity);
            }
            //630890 never looks through the first non-owner occluder.
            break;
        }
    }
    Ok(selected)
}

fn deliver(world: &mut World, entry: &mut Owned, queries: &mut MeleeQueries,
    last_contact: &mut Option<(Option<EntityId>, crate::gameplay::collision_hit::ShapeHitInfo)>,
    world_contacts: &mut WorldContacts) -> Result<(), String> {
    while let Some(delivery) = entry.deliveries.front_mut() {
        let attacker = delivery.attacker;
        if delivery.phase == 0 {
            if !world.resource::<PhysicsScene>().melee_contact_current(delivery.hit.target)? {
                entry.deliveries.pop_front();
                continue;
            }
            *last_contact = Some((delivery.target, delivery.hit.target));
            //631020 calls630810 before634890 and contact virtual110. The
            //host's asynchronous contact-FX publication must not delay the
            //actual Slash→Recoil request until a cold effect becomes ready.
            if delivery.recoil {
                if let Some(player) = world.get_resource_mut::<super::player::PlayerState>()
                    .as_mut().and_then(|state| state.player.as_mut()) {
                    player.machine.request_melee_recoil(1);
                }
            }
            if delivery.world {
                world_contacts.serial = world_contacts.serial.wrapping_add(1);
                let serial = world_contacts.serial;
                let (source_flags, target_flags) = world.resource::<PhysicsScene>()
                    .melee_contact_flags(entry.entity, delivery.target)?;
                world_contacts.last = Some(WorldContact {
                    serial, equipment: entry.entity, window: entry.receipts.windows,
                    activation_player_time: entry.activation_player_time,
                    callback_entity_time: world.resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                        .entity_time(entry.entity).map_err(str::to_owned)?,
                    callback_frame: world.get_resource::<bevy::diagnostic::FrameCount>().map(|frame|frame.0),
                    request_app_frame: None,
                    raw: (delivery.raw_target, delivery.raw_hit.target),
                    selected: (delivery.target, delivery.hit.target),
                    focused: delivery.focused, forward: delivery.frame.forward,
                    source_flags, target_flags, recoil_requested: delivery.recoil,
                    result: "callback accepted; ContactFX pending", ticket: None,
                });
                delivery.world_receipt = Some(serial);
            }
            //635140 ->8519d0: the actual contact witness receives a point
            //impulse; its vertical component uses the source absolute value.
            if !delivery.recoil
                && world
                    .resource::<PhysicsScene>()
                    .melee_body_dynamic(delivery.hit.target)?
            {
                let impulse = world.resource::<PhysicsScene>().melee_point_impulse(
                    world.resource::<LiveEntities>(), delivery.target, delivery.hit.position,
                    delivery.frame.origin.to_array(), delivery.velocity.to_array(), entry.definition.impulse)?;
                world
                    .resource_mut::<PhysicsScene>()
                    .projectile_contact_impulse(delivery.hit, Vec3::from_array(impulse))?;
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
            if !delivery.recoil && delivery.damageable && declaration.is_none() {
                //634320 returns before118/110 when the actual damage
                //declaration is null; it does not invent a contact callback.
                delivery.contact_enabled = false;
            }
            if let Some(declaration) = declaration.filter(|_| !delivery.recoil && delivery.damageable) {
                //635140's point impulse above uses the original packet.
                //634320 invokes player118 on a COPY for target/FX only;
                //damage position, direction and region retain the original.
                if delivery.effect_hit.is_none() {
                    let frame = delivery.frame;
                    let (copy, target) = player_contact(world, queries, entry.entity,
                        attacker, frame.weapon, frame.length, delivery.target, delivery.target, delivery.hit,
                        frame.origin, frame.eye, frame.camera, frame.forward, entry.physics_base)?;
                    if target != delivery.target { entry.receipts.retargeted += 1; }
                    delivery.target = target;
                    delivery.effect_hit = Some(copy);
                }
                let copy = delivery.effect_hit.expect("accepted damage callback packet retained");
                let target = delivery.target;
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
                            //6360a0 binds component+70 through143988130;
                            //initializer141386ec0 names SWORD_DAMAGE_FACTOR.
                            //The similarly named MELEE_TO_HEALTH attribute is
                            //a different provider and can legitimately be0.
                            .get("ARK_ATTRIBUTE_SWORD_DAMAGE_FACTOR")
                            .copied()
                    })
                    .unwrap_or(1.);
                parameters.set_amount(
                    declaration,
                    delivery.factor * factor * declaration.maximum,
                    false,
                );
                entry.receipts.last_damage_amount = parameters.amount;
                entry.receipts.last_damage_factor = factor;
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
                let result = world.resource_scope(|world, mut scene: Mut<PhysicsScene>| {
                    world.resource_scope(|world, mut live: Mut<LiveEntities>| {
                        scene
                            .damage_collision_target(
                                &mut live,
                                &mut world.resource_mut::<EntityEvents>(),
                                copy.target.user_data,
                                &parameters,
                            )
                    })
                })?;
                entry.receipts.damage += 1;
                if let Some(result) = result {
                    entry.receipts.damage_accepted += u64::from(result.accepted);
                    entry.receipts.health_damage += result.health_damage;
                    entry.receipts.last_damage_target = target;
                    entry.receipts.last_damage_serial = entry.receipts.damage;
                    entry.receipts.last_damage_accepted = result.accepted;
                    entry.receipts.last_health_damage = result.health_damage;
                    entry.receipts.last_damage_focused = delivery.focused;
                    entry.receipts.last_damage_forward = delivery.frame.forward;
                }
                //634320 records the returned target;635140 also retains
                //the raw body's target, already journaled at acceptance.
                if !entry.seen.contains(&target) { entry.seen.push(target); }
            }
            delivery.phase = 2;
        }
        if delivery.phase == 2 {
            //Player virtual110 is62fec0; its target-D3 gate precedes both
            //ordinary635c90 and focused-material contact packets.
            delivery.contact_enabled &= world.resource::<PhysicsScene>().melee_contact_enabled(
                world.resource::<LiveEntities>(), delivery.target)?;
            let material = if delivery.focused {
                entry
                    .definition
                    .focused_material
                    .as_ref()
                    .or(entry.definition.material.as_ref())
            } else {
                entry.definition.material.as_ref()
            };
            if let Some(material) = material.filter(|_| delivery.contact_enabled) {
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
                    delivery.effect_hit.as_ref().unwrap_or(&delivery.hit),
                    delivery.velocity.normalize_or_zero(),
                    context,
                )? {
                    super::explosions::ContactPost::Pending => return Ok(()),
                    super::explosions::ContactPost::Complete(Some(ticket)) => {
                        if let Some(receipt) = world_contacts.last.as_mut()
                            .filter(|receipt| Some(receipt.serial) == delivery.world_receipt) {
                            receipt.result = "ContactFX requested";
                            receipt.request_app_frame = world.get_resource::<bevy::diagnostic::FrameCount>().map(|frame|frame.0);
                            receipt.ticket = Some(ticket.clone());
                        }
                        entry.receipts.contact_tickets += 1;
                        entry.tails.push(ticket);
                    }
                    super::explosions::ContactPost::Complete(None) => {
                        if let Some(receipt) = world_contacts.last.as_mut()
                            .filter(|receipt| Some(receipt.serial) == delivery.world_receipt) {
                            receipt.result = "no material effect or source flag suppressed";
                        }
                    },
                }
                entry.receipts.contact_fx += 1;
            } else if let Some(receipt) = world_contacts.last.as_mut()
                .filter(|receipt| Some(receipt.serial) == delivery.world_receipt) {
                receipt.result = "contact callback suppressed by target flag or absent source material";
            }
            delivery.phase = 3;
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
        for entry in owner.owned.values_mut(){deliver(world,entry,&mut owner.queries,&mut owner.last_contact,&mut owner.world_contacts)?;}
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
            deliver(world, entry, &mut owner.queries, &mut owner.last_contact, &mut owner.world_contacts)?;
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
                    entry.receipts.completed += 1;
                    entry.receipts.raw_contacts += hits.len() as u64;
                    for hit in hits {
                        if hit.entity == Some(attacker)
                            || hit.entity == Some(entry.entity)
                            || entry.seen.contains(&hit.entity)
                        {
                            continue;
                        }
                        if !scene.melee_contact_allowed(live, entry.entity, hit)? {
                            continue;
                        }
                        entry.receipts.eligible_contacts += 1;
                        let world_hit = scene.melee_contact_world(live, hit, bank)?;
                        let pass = if world_hit { ContactPass::World } else { ContactPass::Entity };
                        if let Some(existing) = entry
                            .hits
                            .iter_mut()
                            .find(|(existing, category)| *category == pass && existing.entity == hit.entity)
                        {
                            //633a70/633e50 compare packet+54: squared distance
                            //from the real weapon origin to the prepared point.
                            //Penetration distance and cast fraction have
                            //different units and cannot be compared together.
                            let origin = Vec3::from_array(current.position);
                            let candidate = Vec3::from_slice(&witness(hit.hit).position).distance_squared(origin);
                            let previous = Vec3::from_slice(&witness(existing.0.hit).position).distance_squared(origin);
                            if candidate < previous {
                                existing.0 = *hit;
                            }
                        } else {
                            //633a70/633e50 select a packet inside its source
                            //list. Replacing it must retain that destination,
                            //not turn a prop ray into a world-bank contact.
                            entry.hits.push((*hit, pass));
                        }
                    }
                }
            }
            //636b40/633a70 select nearest packets before631020 filters.
            //Rejecting raw packets before dedup can choose a farther hit.
            let mut index = 0;
            while index < entry.hits.len() {
                let (hit, pass) = entry.hits[index];
                let point = Vec3::from_slice(&witness(hit.hit).position);
                let contains = |focus: crate::gameplay::player::melee::FocusShape|
                    focus.contains(origin, eye, camera, entry.definition.focus_step_back, point);
                let mut keep = contains(if pass == ContactPass::World {
                    entry.definition.world_focus
                } else { entry.definition.npc_focus });
                if keep && pass == ContactPass::Entity
                    && world.resource::<PhysicsScene>().melee_entity_needs_world_focus(
                        world.resource::<LiveEntities>(), hit.entity, entry.npc_range)? {
                    keep = contains(entry.definition.world_focus);
                    entry.hits[index].1 = ContactPass::EntityWorldFocus;
                }
                if keep { index += 1; } else {
                    entry.receipts.outside_focus += 1;
                    entry.hits.remove(index);
                }
            }
            if entry.active {
                //631020 extracts ordinary props backwards with ordered
                //RemoveIndex62d0f0, appends remaining NPC/Grenade records
                //with62e200, then635140 processes that ENTITY list before
                //the separate world-impact pass. Iterator order preserves
                //this without copying packets or allocating another list.
                let contacts = entry.hits.iter().rev().filter(|(_, pass)| *pass == ContactPass::EntityWorldFocus)
                    .chain(entry.hits.iter().filter(|(_, pass)| *pass == ContactPass::Entity))
                    .chain(entry.hits.iter().filter(|(_, pass)| *pass == ContactPass::World));
                for (hit, pass) in contacts {
                    let world_hit = *pass == ContactPass::World;
                    //631020 damages entity-bank contacts before considering
                    //a hard world impact; an admitted entity hit suppresses it.
                    if world_hit && !entry.seen.is_empty() {
                        continue;
                    }
                    let raw_witness = witness(hit.hit);
                    let (witness, target) = if world_hit { player_contact(
                        world,
                        &mut owner.queries,
                        entry.entity,
                        attacker,
                        current,
                        entry.shape_length,
                        hit.entity,
                        None,
                        raw_witness,
                        origin,
                        eye,
                        camera,
                        entry.forward_attack,
                        entry.physics_base,
                    )? } else { (raw_witness, hit.entity) };
                    if target != hit.entity { entry.receipts.retargeted += 1; }
                    if entry.seen.contains(&target) { continue; }
                    let point = Vec3::new(
                        witness.position[0],
                        witness.position[1],
                        witness.position[2],
                    );
                    //World impacts are checked again after630890 has
                    //replaced their packet; entity damage follows634320.
                    if world_hit && !entry.definition.world_focus.contains(
                        origin,
                        eye,
                        camera,
                        entry.definition.focus_step_back,
                        point,
                    ) {
                        entry.receipts.outside_focus += 1;
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
                    let recoil = world_hit && world.resource::<PhysicsScene>().melee_world_blocks(
                        world.resource::<LiveEntities>(),
                        witness.target,
                        target,
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
                            entry.receipts.world_dwell += 1;
                            continue;
                        }
                    }
                    entry.deliveries.push_back(Delivery {
                        world: world_hit,
                        raw_hit: raw_witness,
                        raw_target: hit.entity,
                        world_receipt: None,
                        hit: witness,
                        target,
                        attacker,
                        velocity,
                        direction: (Vec3::from_array(current.position) - point).normalize_or_zero(),
                        focused,
                        //635140's ordinary entity damage virtual138 passes1.
                        //3424 belongs to the separate NPC combat branches.
                        factor: 1.,
                        recoil,
                        damageable: world.resource::<PhysicsScene>().melee_damageable(
                            world.resource::<LiveEntities>(), target)?,
                        frame: ContactFrame { weapon: current, length: entry.shape_length,
                            origin, eye, camera, forward: entry.forward_attack },
                        effect_hit: None,
                        contact_enabled: true,
                        phase: 0,
                        contact: None,
                    });
                    entry.receipts.accepted += 1;
                    entry.seen.push(target);
                    if recoil {
                        entry.active = false;
                        entry.stop_after_update = false;
                        entry.seen.clear();
                        entry.world_hit_duration = 0.;
                        break;
                    }
                }
            }
            deliver(world, entry, &mut owner.queries, &mut owner.last_contact, &mut owner.world_contacts)?;
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
    if owner.owned.is_empty() {
        owner.last_contact = None;
        owner.world_contacts.last = None;
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
    entry.last_sweep = Some((sweep.from.to_array(), sweep.to.to_array(),
        sweep.blade_direction.to_array(), sweep.shape_length));
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
    entry.receipts.queries += entry.queries.iter().filter(|id| id.is_valid()).count() as u64;
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
