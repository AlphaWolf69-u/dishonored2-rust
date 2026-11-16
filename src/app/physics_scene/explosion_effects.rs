//! Explosion contact7b24f0, ground7b2e60 and water7b4060 callbacks, sharing
//! ContactDatabase002c60 and the existing FX reservation/instance owner.
use super::*;
use crate::{
    assets::{
        entities::{contacts::ContactDatabase, physics_material::resources::Definition},
        explosions::Declaration,
    },
    gameplay::{
        collision_filter::CollisionQueryFilter,
        collision_hit::ShapeHitInfo,
        explosions::Request,
        reference_frame::{ReferenceFrame, basis_product, direction_basis},
        shape_ray_query::ShapeRayContact,
        world_ray_query::{WorldRayQuery, WorldRayRequest},
    },
};
use bevy::prelude::World;
use bevy::tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once};
use bevy_math::Vec3;
use std::sync::Mutex;

#[derive(Clone, PartialEq, Eq)]
struct Spec {
    contact: Option<String>,
    contact_target: Option<String>,
    ground: Option<String>,
    water: Option<String>,
}
impl Spec {
    fn from(d: &Declaration) -> Self {
        Self {
            contact: d.contact_material.clone(),
            contact_target: None,
            ground: d.ground_material.clone(),
            water: d.water_fx.clone(),
        }
    }
}
struct Prepared {
    rows: HashMap<(String, String), Option<String>>,
    paths: Vec<String>,
    _materials: Vec<Arc<Definition>>,
    _database: Option<Arc<ContactDatabase>>,
}
struct Scope {
    spec: Spec,
    revision: usize,
    fx_key: Arc<str>,
    task: Option<Mutex<Task<Result<Prepared, String>>>>,
    prepared: Option<Prepared>,
    error: Option<String>,
    reserved: bool,
}
#[derive(Default)]
pub(in crate::app) struct Owner {
    scopes: HashMap<Arc<str>, Scope>,
    retiring: Vec<Mutex<Task<Result<Prepared, String>>>>,
    active: Vec<(Arc<str>, super::super::usable_effects::EffectTicket)>,
    sequence: u64,
}
pub(in crate::app) struct Packet {
    pub path: String,
    pub frame: ReferenceFrame,
}
#[derive(Clone)]
pub(in crate::app) struct Contact {
    pub source: String,
    pub target: String,
    pub position: Vec3,
    pub normal: Vec3,
    pub direction: Vec3,
}
#[derive(Default)]
pub(in crate::app) struct Queries {
    ray: WorldRayQuery,
    hits: Vec<ShapeRayContact>,
    water: Vec<ShapePointContact>,
}

fn acquire(
    catalog: Arc<archive::Catalog>,
    spec: Spec,
    targets: Vec<Arc<Definition>>,
) -> Result<Prepared, String> {
    let mut rows = HashMap::new();
    let mut paths = Vec::new();
    let mut materials = if let Some(target) = &spec.contact_target {
        vec![
            crate::assets::entities::physics_material::resources::acquire_with(
                &catalog,
                target,
                |path| crate::assets::declaration_texts::read_common_or_catalog(&catalog, path),
            )?,
        ]
    } else {
        targets
    };
    let target_count = materials.len();
    let database = if spec.contact.is_some() || spec.ground.is_some() {
        Some(ContactDatabase::acquire(&catalog)?)
    } else {
        None
    };
    for name in [spec.contact.as_deref(), spec.ground.as_deref()]
        .into_iter()
        .flatten()
    {
        let source = crate::assets::entities::physics_material::resources::acquire_with(
            &catalog,
            name,
            |path| crate::assets::declaration_texts::read_common_or_catalog(&catalog, path),
        )?;
        for target in &materials[..target_count] {
            let path = database
                .as_ref()
                .unwrap()
                .effect(source.contact_chain(), target.contact_chain())
                .map(str::to_owned);
            if let Some(path) = &path {
                paths.push(path.clone());
            }
            rows.insert(
                (name.to_ascii_lowercase(), target.name.to_ascii_lowercase()),
                path,
            );
        }
        materials.push(source);
    }
    paths.extend(spec.water);
    paths.sort_unstable();
    paths.dedup();
    Ok(Prepared {
        rows,
        paths,
        _materials: materials,
        _database: database,
    })
}
impl Owner {
    pub(in crate::app) fn prepare(
        &mut self,
        world: &World,
        key: Arc<str>,
        d: &Declaration,
    ) -> Result<(), String> {
        self.prepare_spec(world, key, Spec::from(d))
    }
    pub(in crate::app) fn prepare_contact(
        &mut self,
        world: &World,
        key: Arc<str>,
        source: &str,
    ) -> Result<(), String> {
        self.prepare_spec(
            world,
            key,
            Spec {
                contact: Some(source.to_owned()),
                contact_target: None,
                ground: None,
                water: None,
            },
        )
    }
    ///651020 supplies two actual material declarations, not every surface in
    ///the world. Restrict the existing owner to that immutable source pair.
    pub(in crate::app) fn prepare_contact_pair(
        &mut self,
        world: &World,
        key: Arc<str>,
        source: &str,
        target: &str,
    ) -> Result<(), String> {
        self.prepare_spec(
            world,
            key,
            Spec {
                contact: Some(source.to_owned()),
                contact_target: Some(target.to_owned()),
                ground: None,
                water: None,
            },
        )
    }
    fn prepare_spec(&mut self, world: &World, key: Arc<str>, spec: Spec) -> Result<(), String> {
        if let Some(scope) = self.scopes.get(&key) {
            return if scope.spec == spec {
                Ok(())
            } else {
                Err("explosion scope generation reused for another declaration".into())
            };
        }
        let (revision, targets) = if spec.contact_target.is_some() {
            (0, Vec::new())
        } else {
            world
                .get_resource::<PhysicsScene>()
                .ok_or("explosion scene is absent")?
                .explosion_materials()?
        };
        let catalog = world
            .get_resource::<super::super::SceneData>()
            .and_then(|scene| scene.catalog.clone())
            .ok_or("explosion catalog lease is absent")?;
        let requested = spec.clone();
        let task = IoTaskPool::get().spawn(async move { acquire(catalog, requested, targets) });
        let fx_key = Arc::from(format!("{key}:materials:{revision}"));
        self.scopes.insert(
            key,
            Scope {
                spec,
                revision,
                fx_key,
                task: Some(Mutex::new(task)),
                prepared: None,
                error: None,
                reserved: false,
            },
        );
        Ok(())
    }
    pub(in crate::app) fn pump(&mut self, world: &mut World) -> Result<(), String> {
        self.retiring.retain_mut(|task| {
            block_on(poll_once(task.get_mut().unwrap_or_else(|e| e.into_inner()))).is_none()
        });
        self.active
            .retain(|(_, ticket)| !super::super::usable_effects::entity_retired(world, ticket));
        let revision = world
            .get_resource::<PhysicsScene>()
            .map(PhysicsScene::explosion_material_revision)
            .transpose()?;
        for (key, scope) in &mut self.scopes {
            if scope.spec.contact_target.is_none()
                && scope.prepared.is_some()
                && Some(scope.revision) != revision
            {
                if scope.reserved {
                    world
                        .resource_mut::<super::super::usable_effects::EffectRequests>()
                        .release_reservation(scope.fx_key.clone());
                    scope.reserved = false;
                }
                let scene = world.resource::<PhysicsScene>();
                let (new_revision, targets) = scene.explosion_materials()?;
                let catalog = world
                    .get_resource::<super::super::SceneData>()
                    .and_then(|scene| scene.catalog.clone())
                    .ok_or("explosion catalog lease is absent")?;
                let spec = scope.spec.clone();
                scope.task = Some(Mutex::new(
                    IoTaskPool::get().spawn(async move { acquire(catalog, spec, targets) }),
                ));
                scope.revision = new_revision;
                scope.fx_key = Arc::from(format!("{key}:materials:{new_revision}"));
                scope.prepared = None;
                scope.error = None;
            }
            let Some(task) = scope.task.as_mut() else {
                continue;
            };
            let Some(result) =
                block_on(poll_once(task.get_mut().unwrap_or_else(|e| e.into_inner())))
            else {
                continue;
            };
            scope.task = None;
            match result {
                Ok(prepared) => {
                    world
                        .get_resource_mut::<super::super::usable_effects::EffectRequests>()
                        .ok_or("canonical FX request owner is absent")?
                        .reserve(scope.fx_key.clone(), prepared.paths.clone());
                    scope.reserved = true;
                    scope.prepared = Some(prepared);
                }
                Err(error) => scope.error = Some(error),
            }
        }
        Ok(())
    }
    pub(in crate::app) fn ready(
        &self,
        world: &World,
        key: &str,
        d: &Declaration,
    ) -> Result<bool, String> {
        let Some(scope) = self.scopes.get(key) else {
            return Ok(false);
        };
        if scope.spec != Spec::from(d) {
            return Err("explosion scope declaration differs".into());
        }
        if let Some(error) = &scope.error {
            return Err(error.clone());
        }
        let revision = world
            .get_resource::<PhysicsScene>()
            .ok_or("explosion scene is absent")?
            .explosion_material_revision()?;
        if scope.prepared.is_none() || scope.revision != revision {
            return Ok(false);
        }
        super::super::usable_effects::reservation_ready(world, &scope.fx_key)
    }
    pub(in crate::app) fn ready_for(&self, world: &World, d: &Declaration) -> Result<bool, String> {
        for (key, scope) in &self.scopes {
            if scope.spec == Spec::from(d) && self.ready(world, key, d)? {
                return Ok(true);
            }
        }
        Ok(false)
    }
    pub(in crate::app) fn ready_contact(
        &self,
        world: &World,
        key: &str,
        source: &str,
    ) -> Result<bool, String> {
        let Some(scope) = self.scopes.get(key) else {
            return Ok(false);
        };
        if scope.spec.contact.as_deref() != Some(source)
            || scope.spec.ground.is_some()
            || scope.spec.water.is_some()
        {
            return Err("contact scope source differs".into());
        }
        if let Some(error) = &scope.error {
            return Err(error.clone());
        }
        if scope.prepared.is_none()
            || (scope.spec.contact_target.is_none()
                && scope.revision
                    != world
                        .get_resource::<PhysicsScene>()
                        .ok_or("contact scene is absent")?
                        .explosion_material_revision()?)
        {
            return Ok(false);
        }
        super::super::usable_effects::reservation_ready(world, &scope.fx_key)
    }
    pub(in crate::app) fn post_contact(
        &mut self,
        world: &mut World,
        key: &str,
        source: &str,
        hit: &ShapeRayContact,
        direction: Vec3,
        context: crate::rendering::bevy_backend::world_effects::EntityEffectContext,
    ) -> Result<Option<super::super::usable_effects::EffectTicket>, String> {
        if !self.ready_contact(world, key, source)? {
            return Err("contact FX source dependencies are not ready".into());
        }
        let Some(mut contact) = world.resource::<PhysicsScene>().explosion_contact(
            source,
            Vec3::from_slice(&hit.position[..3]),
            hit,
        )?
        else {
            return Ok(None);
        };
        contact.direction = direction;
        self.post_contact_value(world, key, contact, context)
    }
    pub(in crate::app) fn post_contact_value(
        &mut self,
        world: &mut World,
        key: &str,
        contact: Contact,
        context: crate::rendering::bevy_backend::world_effects::EntityEffectContext,
    ) -> Result<Option<super::super::usable_effects::EffectTicket>, String> {
        let source = &contact.source;
        let path = self
            .scopes
            .get(key)
            .and_then(|scope| scope.prepared.as_ref())
            .ok_or("contact source reservation disappeared")?
            .rows
            .get(&(
                source.to_ascii_lowercase(),
                contact.target.to_ascii_lowercase(),
            ))
            .ok_or("contact target material was not prepared")?
            .clone();
        if let Some(path) = path {
            let frame = contact_frame(contact.position, contact.normal, contact.direction);
            let entity = context
                .owner
                .ok_or("contact FX source has no actual entity owner")?;
            if world
                .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .and_then(|live| live.get(entity))
                .is_none()
            {
                return Err("contact FX source generation is stale".into());
            }
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or("contact FX generation exhausted")?;
            let key: Arc<str> = Arc::from(format!("contact-fx:{}", self.sequence));
            let placement = super::super::usable_effects::EntityPlacement {
                target: crate::gameplay::event_graph::EntityReference::Invalid,
                tag: String::new(),
                local: ReferenceFrame::default(),
                attached: false,
            };
            let ticket = world
                .get_resource_mut::<super::super::usable_effects::EffectRequests>()
                .ok_or("canonical FX request owner is absent")?
                .start_breakable_once(key.clone(), &path, placement, frame, context);
            self.active.push((key, ticket.clone()));
            return Ok(Some(ticket));
        }
        Ok(None)
    }
    pub(in crate::app) fn release(&mut self, world: &mut World, key: &str) {
        let Some(scope) = self.scopes.remove(key) else {
            return;
        };
        if scope.reserved {
            if let Some(mut requests) =
                world.get_resource_mut::<super::super::usable_effects::EffectRequests>()
            {
                requests.release_reservation(scope.fx_key);
            }
        }
        if let Some(task) = scope.task {
            self.retiring.push(task);
        }
    }
    pub(in crate::app) fn close(&mut self, world: &mut World) {
        let keys = self.scopes.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            self.release(world, &key)
        }
        if let Some(mut requests) =
            world.get_resource_mut::<super::super::usable_effects::EffectRequests>()
        {
            for (key, _) in &self.active {
                requests.stop_entity(key.clone());
            }
        }
    }
    pub(in crate::app) fn drained(&self) -> bool {
        self.scopes.is_empty() && self.retiring.is_empty() && self.active.is_empty()
    }
    pub(in crate::app) fn packets(
        &self,
        request: &Request,
        contacts: &[Contact],
        water: Option<(f32, f32)>,
    ) -> Result<Vec<Packet>, String> {
        let spec = Spec::from(&request.declaration);
        let prepared = self
            .scopes
            .values()
            .filter(|scope| scope.spec == spec && scope.prepared.is_some())
            .max_by_key(|scope| scope.revision)
            .and_then(|scope| scope.prepared.as_ref())
            .ok_or("explosion FX source reservation is absent")?;
        let mut packets = Vec::new();
        for contact in contacts {
            let path = prepared
                .rows
                .get(&(
                    contact.source.to_ascii_lowercase(),
                    contact.target.to_ascii_lowercase(),
                ))
                .ok_or("explosion target material was not prepared before input")?;
            if let Some(path) = path {
                packets.push(Packet {
                    path: path.clone(),
                    frame: contact_frame(contact.position, contact.normal, contact.direction),
                })
            }
        }
        if let (Some(path), Some((height, yaw))) = (&spec.water, water) {
            let (s, c) = (crate::float::sinf(yaw), crate::float::cosf(yaw));
            packets.push(Packet {
                path: path.clone(),
                frame: ReferenceFrame {
                    position: [request.position.x, request.position.y, height],
                    basis: [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]],
                },
            });
        }
        Ok(packets)
    }
    pub(in crate::app) fn dispatch(
        &mut self,
        world: &mut World,
        packets: &[Packet],
        cursor: &mut usize,
    ) -> Result<(), String> {
        let mut requests = world
            .get_resource_mut::<super::super::usable_effects::EffectRequests>()
            .ok_or("canonical FX request owner is absent")?;
        while let Some(packet) = packets.get(*cursor) {
            self.sequence = self
                .sequence
                .checked_add(1)
                .ok_or("explosion FX generation exhausted")?;
            let key: Arc<str> = Arc::from(format!("explosion-fx:{}", self.sequence));
            let ticket = requests.start_world_once(key.clone(), &packet.path, packet.frame);
            self.active.push((key, ticket));
            *cursor += 1;
        }
        Ok(())
    }
}

///6250e0/6289c0 submit the real projectile's source FX and retain its ordinary
///completion ticket. The projectile owner must wait for that completion before
///running its removal callback; this adapter never creates another entity.
pub(in crate::app) fn projectile_impact(
    world: &mut World,
    owner: Arc<str>,
    path: &str,
    position: Vec3,
    normal: Vec3,
    direction: Vec3,
    context: crate::rendering::bevy_backend::world_effects::EntityEffectContext,
) -> Result<super::super::usable_effects::EffectTicket, String> {
    let id = context
        .owner
        .ok_or("projectile impact FX has no actual entity owner")?;
    if world
        .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
        .and_then(|live| live.get(id))
        .is_none()
    {
        return Err("projectile impact FX owner generation is stale".into());
    }
    let frame = contact_frame(
        position - direction * f32::from_bits(0x3d4ccccd),
        normal,
        direction,
    );
    let placement = super::super::usable_effects::EntityPlacement {
        target: crate::gameplay::event_graph::EntityReference::Invalid,
        tag: String::new(),
        local: ReferenceFrame::default(),
        attached: false,
    };
    Ok(world
        .get_resource_mut::<super::super::usable_effects::EffectRequests>()
        .ok_or("canonical FX request owner is absent")?
        .start_breakable_once(owner, path, placement, frame, context))
}

///60a8b0/6289c0: nearly vertical normals use a Y quarter-turn followed by
///incoming yaw; other normals use the common112000 direction frame.
pub(in crate::app) fn contact_frame(
    position: Vec3,
    normal: Vec3,
    mut direction: Vec3,
) -> ReferenceFrame {
    let basis = if f32::from_bits(0x3f7d70a4) <= normal.z.abs() {
        let angle = if normal.z < 0. {
            f32::from_bits(0x3fc90fdb)
        } else {
            direction = -direction;
            -f32::from_bits(0x3fc90fdb)
        };
        let (s, c) = (crate::float::sinf(angle), crate::float::cosf(angle));
        let y = [[c, 0., -s], [0., 1., 0.], [s, 0., c]];
        let yaw =
            if direction.x.abs() <= f32::MIN_POSITIVE && direction.y.abs() <= f32::MIN_POSITIVE {
                0.
            } else {
                let yaw = direction.y.atan2(direction.x);
                if yaw < 0. {
                    yaw + f32::from_bits(0x40c90fdb)
                } else {
                    yaw
                }
            };
        let (s, c) = (crate::float::sinf(yaw), crate::float::cosf(yaw));
        basis_product(y, [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]])
    } else {
        direction_basis(normal).to_cols_array_2d()
    };
    ReferenceFrame {
        position: position.to_array(),
        basis,
    }
}

impl PhysicsScene {
    fn explosion_materials(&self) -> Result<(usize, Vec<Arc<Definition>>), String> {
        let world = self
            .world
            .read()
            .map_err(|_| "explosion material world is poisoned")?;
        let owner = &world.storage.materials;
        Ok((
            owner.declaration_revision(),
            owner.contact_definitions().to_vec(),
        ))
    }
    fn explosion_material_revision(&self) -> Result<usize, String> {
        Ok(self
            .world
            .read()
            .map_err(|_| "explosion material world is poisoned")?
            .storage
            .materials
            .declaration_revision())
    }
    pub(in crate::app) fn explosion_effect_contacts(
        &mut self,
        query: &mut Queries,
        request: &Request,
        contact_hits: &[ShapeRayContact],
    ) -> Result<(Vec<Contact>, Option<f32>), String> {
        let mut contacts = Vec::new();
        let d = &request.declaration;
        if let Some(source) = &d.contact_material {
            for hit in contact_hits {
                if let Some(contact) = self.explosion_contact(source, request.position, hit)? {
                    contacts.push(contact);
                }
            }
        }
        if let Some(source) = d
            .ground_material
            .as_ref()
            .filter(|_| d.ground_distance > 0.)
        {
            let world = self
                .world
                .read()
                .map_err(|_| "explosion ground query world poisoned")?;
            let request_ray = WorldRayRequest {
                origin: request.position.to_array(),
                direction: [0., 0., -d.ground_distance],
                maximum_fraction: 1.,
                hit: ShapeHitInfo {
                    body_id: None,
                    material_id: u16::MAX,
                    shape_key: u32::MAX,
                    filter_info: 0,
                    user_data: 0,
                },
                filter: Some(CollisionQueryFilter {
                    layers: d.occlusion_layers,
                    required_flags: 0x800,
                }),
            };
            world.ray_hits(&mut query.ray, &request_ray, 2, &mut query.hits)?;
            let closest = query
                .hits
                .iter()
                .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
                .copied();
            drop(world);
            if let Some(hit) = closest {
                if let Some(mut contact) = self.explosion_contact(source, request.position, &hit)? {
                    contact.direction = Vec3::Z;
                    contacts.push(contact);
                }
            }
        }
        let mut water = None;
        if d.water_fx.is_some() {
            let world = self
                .world
                .read()
                .map_err(|_| "explosion water query world poisoned")?;
            world.storage.point_query(
                &mut self.executor,
                &WorldPointRequest::water(request.position.to_array()),
                |shape| {
                    Ok(WorldQueryShape {
                        shapes: &world
                            .geometry
                            .shapes
                            .get(shape)
                            .and_then(Option::as_ref)
                            .ok_or("explosion water query shape absent")?
                            .query_shapes,
                        materials: None,
                    })
                },
                &mut query.water,
            )?;
            for contact in &query.water {
                if contact.target.user_data == 0 {
                    continue;
                }
                let index = crate::gameplay::collision_user_data::CollisionUserData::decode(
                    contact.target.user_data,
                )?
                .entity(|handle| self.owner(handle).ok())?;
                if let Some(handle) = self.water_component(index)? {
                    let component = self
                        .waters
                        .get(handle.slot)
                        .ok_or("explosion water component absent")?;
                    if component.owner.flags & 0xc < 8 {
                        continue;
                    }
                    let height = -component
                        .plane()
                        .ok_or("started explosion water has no plane")?[3];
                    if height - d.water_distance < request.position.z {
                        water = Some(height)
                    }
                    break;
                }
            }
        }
        Ok((contacts, water))
    }
    fn explosion_contact(
        &self,
        source: &str,
        origin: Vec3,
        hit: &ShapeRayContact,
    ) -> Result<Option<Contact>, String> {
        let world = self
            .world
            .read()
            .map_err(|_| "explosion contact material world poisoned")?;
        //7b24f0/7b2e60 submit nothing when the actual witness resolves no
        //physics-material declaration; preset/null records are not materials.
        if hit.target.material_id == u16::MAX {
            return Ok(None);
        }
        let record = world
            .storage
            .materials
            .library
            .get(hit.target.material_id)
            .ok_or("explosion contact material ID is stale")?;
        if record.words[14] == 0 && record.words[15] == 0 {
            return Ok(None);
        }
        let target = world
            .storage
            .materials
            .material_for_id(hit.target.material_id)?
            .name
            .clone();
        let position = Vec3::from_slice(&hit.position[..3]);
        let delta = position - origin;
        let direction = delta * (1. / delta.length_squared().max(f32::MIN_POSITIVE).sqrt());
        Ok(Some(Contact {
            source: source.to_owned(),
            target,
            position,
            normal: Vec3::from_slice(&hit.normal[..3]),
            direction,
        }))
    }
}
