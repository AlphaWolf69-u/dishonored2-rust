//! 637160/636b40 retained melee query submissions and result transfer.
//! The weapon owner supplies its actual632820 shape and634990/634ce0 sweep;
//! this adapter neither invents a blade ray nor constructs a second world.
use super::*;
use crate::gameplay::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    collision_queries::{CollisionQueries, QueryHits, QueryId},
    collision_user_data::CollisionUserData,
    entity_lifetime::{EntityId, LiveEntities},
    shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
    shape_ray_query::ShapeRayContact,
    shape_transform::ShapeMatrix,
    world_convex_query::{WorldConvexQuery, WorldConvexRequest},
    world_ray_query::{WorldRayQuery, WorldRayRequest},
};

/// Retained support data from the actual weapon shape owner. Query requests
/// borrow the same immutable allocation through their completion/cancel ack.
pub(in crate::app) struct Shape {
    pub shape_type: u8,
    pub vertices: Arc<[[f32; 4]]>,
    pub radius: f32,
}
///0932d0 creates the shared[0,1] box withca15b0's .01 shrink/radius.
///632890/cb83b0 wraps it andcb4d60 resizes it. This host query backend
///publishes scaled support vertices after the prior query references drain;
///the allocation is retained instead of constructing another shape cache.
pub(in crate::app) fn prepare_shape(
    retained: &mut Option<Arc<Shape>>,
    dimensions: bevy_math::Vec3,
) -> Result<Arc<Shape>, String> {
    let original_radius = f32::from_bits(0x3c23_d70a);
    let high = 1. - original_radius;
    let base: [[f32; 4]; 8] = std::array::from_fn(|index| {
        [
            if index & 1 == 0 {
                high
            } else {
                original_radius
            },
            if index & 2 == 0 {
                high
            } else {
                original_radius
            },
            if index & 4 == 0 {
                high
            } else {
                original_radius
            },
            f32::from_bits(0x3f00_0000 | index as u32),
        ]
    });
    let mut scale = [dimensions.x, dimensions.y, dimensions.z, 1.];
    let mut radius = original_radius;
    let offset = crate::gameplay::convex_support::scaling::adjust_scale(
        5,
        &base,
        false,
        &mut scale,
        &mut radius,
    )?;
    let shape = retained.get_or_insert_with(|| {
        Arc::new(Shape {
            shape_type: 5,
            vertices: Arc::from(base),
            radius,
        })
    });
    let mutable = Arc::get_mut(shape)
        .ok_or("melee shape resize preceded its outstanding query acknowledgment")?;
    let vertices = Arc::get_mut(&mut mutable.vertices)
        .ok_or("melee support vertices still have an outstanding owner")?;
    for (index, vertex) in vertices.iter_mut().enumerate() {
        for axis in 0..3 {
            vertex[axis] = base[index][axis] * scale[axis] + offset[axis];
        }
        vertex[3] = base[index][3];
    }
    mutable.radius = radius;
    Ok(shape.clone())
}
pub(in crate::app) struct Sweep {
    pub shape: Arc<Shape>,
    pub transform: ShapeMatrix,
    /// Native query direction is normalized; its W stores this distance.
    pub direction: [f32; 3],
    pub maximum_distance: f32,
    pub layers: u32,
}
#[derive(Clone, Copy)]
pub(in crate::app) struct Ray {
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub maximum_fraction: f32,
    pub layers: u32,
}
enum Request {
    Shape(Sweep),
    Ray(Ray),
}
#[derive(Clone, Copy)]
pub(in crate::app) enum Hit {
    Shape(ConvexContact),
    Ray(ShapeRayContact),
}
impl Hit {
    pub fn target(self) -> ShapeHitInfo {
        match self {
            Self::Shape(hit) => hit.target,
            Self::Ray(hit) => hit.target,
        }
    }
    pub fn distance_parameter(self) -> f32 {
        match self {
            Self::Shape(hit) if hit.flags == 4 => hit.distance,
            Self::Shape(hit) => hit.fraction,
            Self::Ray(hit) => hit.fraction,
        }
    }
}
#[derive(Clone, Copy)]
pub(in crate::app) struct Contact {
    pub hit: Hit,
    pub entity: Option<EntityId>,
}
/// One manager-owned pool shared by all admitted weapons. Individual weapon
/// instances retain only their four actual query IDs and source frame state.
#[derive(Default)]
pub(in crate::app) struct MeleeQueries {
    slots: Option<CollisionQueries<Request, Hit>>,
    convex: WorldConvexQuery,
    ray: WorldRayQuery,
    rays: Vec<ShapeRayContact>,
    transferred: Vec<Hit>,
    contacts: Vec<Contact>,
}
fn no_body() -> ShapeHitInfo {
    ShapeHitInfo {
        body_id: None,
        material_id: u16::MAX,
        shape_key: u32::MAX,
        filter_info: 0,
        user_data: 0,
    }
}
impl MeleeQueries {
    pub fn has_pending(&self) -> bool {
        self.slots
            .as_ref()
            .is_some_and(CollisionQueries::has_pending)
    }
    pub fn submit_shape(
        &mut self,
        request: Sweep,
        world_available: bool,
    ) -> Result<QueryId, String> {
        self.slots
            .get_or_insert_with(CollisionQueries::default)
            .submit(Request::Shape(request), world_available)
            .map_err(|e| e.to_string())
    }
    pub fn submit_ray(&mut self, request: Ray, world_available: bool) -> Result<QueryId, String> {
        self.slots
            .get_or_insert_with(CollisionQueries::default)
            .submit(Request::Ray(request), world_available)
            .map_err(|e| e.to_string())
    }
    pub fn cancel(&mut self, id: &mut QueryId) -> Result<(), String> {
        if id.is_valid() {
            self.slots
                .as_mut()
                .ok_or("melee query owner absent")?
                .cancel(id)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}
impl PhysicsScene {
    ///635140 chooses damage virtual138 only for actual target+d7 bit2.
    ///A non-damageable entity receives the raw contact virtual110 instead.
    pub(in crate::app) fn melee_damageable(&self, live: &LiveEntities,
        target: Option<EntityId>) -> Result<bool, String> {
        let Some(target) = target else { return Ok(false); };
        if live.get(target).is_none() { return Ok(false); }
        let Some(&index) = self.shared_indices.get(&target) else { return Ok(false); };
        let entity = self.entities.get(index).ok_or("melee damage target header absent")?;
        Ok(!entity.retired && entity.shared == target && entity.entity_flags_d7 & 2 != 0)
    }

    ///631020 compares actual arkNPC type interval1439ce2c0/4.
    ///Registration1413c5f60 uses DAT1439ce260 (chunk988:2148).
    pub(in crate::app) fn melee_npc_range(&self) -> Result<[i32; 2], String> {
        let classes = self.types.classes();
        let class = classes.hierarchy().registration_order().iter().copied().find(|&class|
            classes.metadata(class).is_some_and(|metadata|
                classes.name_text(metadata.name) == Some(b"arkNPC")))
            .ok_or("melee NPC class metadata absent")?;
        classes.hierarchy().type_range(class)
    }

    ///631020's entity-list split: only real NPCs and mode2 Grenade owners
    ///bypass the additional WORLD focus pass. Health does not select this.
    pub(in crate::app) fn melee_entity_needs_world_focus(&self, live: &LiveEntities,
        target: Option<EntityId>, npc_range: [i32; 2]) -> Result<bool, String> {
        let Some(target) = target else { return Ok(true); };
        let Some((_, class)) = live.get(target) else { return Ok(true); };
        let class = self.types.classes().hierarchy().type_id(class)?;
        if (npc_range[0]..=npc_range[1]).contains(&class) { return Ok(false); }
        let Some(&index) = self.shared_indices.get(&target) else { return Ok(true); };
        let entity = self.entities.get(index).ok_or("melee entity focus header absent")?;
        if entity.retired || entity.shared != target { return Ok(true); }
        Ok(self.melee_grenade_component(index)?.is_none())
    }

    fn melee_grenade_component(&self, index: usize) -> Result<Option<ComponentHandle>, String> {
        let grenade_range = self.types.classes().hierarchy()
            .type_range(self.types.factory(self.factories[19])?.class)?;
        self.entities[index].components.find_component(grenade_range, [0x28, 0], |handle| {
            Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                flags: self.owner(handle)?.flags,
                start_phases: self.declarations.get(&handle.packed()).map(|decl| decl.scheduling.start_phases),
            })
        })
    }

    ///635140 uses the raw packet's velocity for ordinary bodies. A real
    ///Started Grenade instead uses its OWN authored melee impulse and the
    ///direction from the attacker to hit+[0,0,.2] (141ab3818).
    pub(in crate::app) fn melee_point_impulse(&self, live: &LiveEntities,
        target: Option<EntityId>, position: [f32; 4], source: [f32; 3],
        velocity: [f32; 3], force: f32) -> Result<[f32; 3], String> {
        let mut vector = velocity;
        let mut magnitude = force;
        let mut grenade = false;
        if let Some(target) = target.filter(|target| live.get(*target).is_some()) {
            if let Some(&index) = self.shared_indices.get(&target) {
                if let Some(handle) = self.melee_grenade_component(index)? {
                    if handle.class_id != self.classes[19] {
                        return Err("melee Grenade subclass has no admitted payload owner".into());
                    }
                    let component = self.grenades.get(handle.slot).ok_or("melee Grenade payload absent")?;
                    magnitude = component.definition.melee_impulse;
                    vector = [position[0] - source[0], position[1] - source[1],
                        (position[2] + f32::from_bits(0x3e4c_cccd)) - source[2]];
                    grenade = true;
                }
            }
        }
        let square = vector[0] * vector[0] + vector[1] * vector[1] + vector[2] * vector[2];
        let inverse = 1. / square.max(f32::from_bits(0x0080_0000)).sqrt();
        let mut impulse = vector.map(|value| value * inverse * magnitude);
        if !grenade { impulse[2] = impulse[2].abs(); }
        Ok(impulse)
    }

    ///630890's final condition is the real PhysicsBase family, not just
    ///RigidBody. Resolve this immutable class identity at weapon admission.
    pub(in crate::app) fn melee_physics_base_range(&self) -> Result<[i32; 2], String> {
        let classes = self.types.classes();
        let class = classes.hierarchy().registration_order().iter().copied().find(|&class|
            classes.metadata(class).is_some_and(|metadata|
                classes.name_text(metadata.name) == Some(b"arkCpntPhysicsBase")))
            .ok_or("melee PhysicsBase class metadata absent")?;
        classes.hierarchy().type_range(class)
    }

    ///630890: the FIRST non-owner hit can replace the focused contact only
    ///for damageable, special-damage, or actually Started PhysicsBase targets.
    pub(in crate::app) fn melee_forward_candidate(&self, live: &LiveEntities,
        target: Option<EntityId>, physics_base: [i32; 2]) -> Result<bool, String> {
        let Some(target) = target else { return Ok(false); };
        if live.get(target).is_none() { return Ok(false); }
        let Some(&index) = self.shared_indices.get(&target) else { return Ok(false); };
        let entity = self.entities.get(index).ok_or("melee focused target header absent")?;
        if entity.retired || entity.shared != target { return Ok(false); }
        if entity.entity_flags_d7 & (2 | 8) != 0 { return Ok(true); }
        Ok(entity.components.find_component(physics_base, [0x28, 0], |handle| {
            Ok(crate::gameplay::component_state::entity_lookup::ComponentLookupState {
                flags: self.owner(handle)?.flags,
                start_phases: self.declarations.get(&handle.packed()).map(|decl| decl.scheduling.start_phases),
            })
        })?.is_some())
    }

    ///636b40 keeps world/entity result lists separate. Its closest-ray bank
    ///goes to world only for null/idWorldspawn, independently of query layers.
    pub(in crate::app) fn melee_contact_world(
        &self, live: &LiveEntities, contact: &Contact, bank: usize,
    ) -> Result<bool, String> {
        match bank {
            0 => Ok(true),
            2 => Ok(false),
            //The same registered idWorldspawn range1439b8470 is used by
            //6123e0's existing typed check. classes[14] is a component class
            //(CpntPhysicsWorld), so it cannot replace this entity type check.
            3 => Ok(self.heart_context_target(live, contact.entity)?.is_none()),
            1 => {
                let target = contact.entity.ok_or("melee enemy collector target identity absent")?;
                //636b40 merges +2298 into +1188 only for entity+d1 bit3.
                //The remaining live-enemy list needs actual NPC callbacks;
                //do not silently treat that unhosted owner as a plain prop.
                if self.heart_context_target_alive(live, Some(target))? {
                    return Err("melee live-enemy collector requires its actual NPC callback owner".into());
                }
                Ok(false)
            }
            _ => Err("melee collector bank is outside the source four tickets".into()),
        }
    }
    ///6364c0 routes the two actual collectors separately.633e50(initial4)
    ///rejects every same-owner target;6340f0(main2) retains the explicit
    ///arkProjectile/arkGrenade exception. Ownership is entity+ac, not binding.
    pub(in crate::app) fn melee_contact_allowed(
        &self, live: &LiveEntities, equipment: EntityId, contact: &Contact,
    ) -> Result<bool, String> {
        let flags = match contact.hit {
            //636b40 sends its separate closest-ray result directly to633a70.
            Hit::Ray(_) => return Ok(true),
            Hit::Shape(hit) => hit.flags,
        };
        if !matches!(flags, 2 | 4) { return Ok(false); }
        let Some(source) = live.owner(equipment)? else { return Ok(true); };
        let Some(target) = contact.entity else { return Ok(true); };
        if target == source { return Ok(false); }
        if live.owner(target)? != Some(source) { return Ok(true); }
        if flags == 4 { return Ok(false); }
        //TypeInfo registrations988/990: ranges1439cbec0/1439d46b0 are
        //arkGrenade/arkProjectile. Walk the existing real class hierarchy.
        let Some((_, mut class)) = live.get(target) else { return Ok(false); };
        let classes = self.types.classes();
        loop {
            let metadata = classes.metadata(class).ok_or("melee target class metadata absent")?;
            if matches!(classes.name_text(metadata.name), Some(b"arkProjectile" | b"arkGrenade")) {
                return Ok(true);
            }
            let Some(base) = classes.hierarchy().node(class).and_then(|node|node.base) else { return Ok(false); };
            class = base;
        }
    }
    pub(in crate::app) fn melee_body_dynamic(&self, hit: ShapeHitInfo) -> Result<bool, String> {
        let Some(body) = hit.body_id else {
            return Ok(false);
        };
        let world = self
            .world
            .read()
            .map_err(|_| "melee impulse world poisoned")?;
        Ok(world
            .storage
            .bodies
            .get(body)
            .ok_or("melee impulse body absent")?
            .flags
            & 5
            == 0)
    }

    ///635140 validates the full body generation before its accepted callback.
    ///An object retired while an earlier FX post was pending is skipped.
    pub(in crate::app) fn melee_contact_current(&self, hit: ShapeHitInfo) -> Result<bool, String> {
        let Some(id) = hit.body_id else { return Ok(false); };
        let world = self.world.read().map_err(|_| "melee contact world poisoned")?;
        Ok(world.storage.bodies.get(id).is_some_and(|body| body.user_data == hit.user_data))
    }
    ///635b00/39eed0: static world is blocking; a real damageable entity
    ///is not, except a HealthDoor which the actual attacker cannot break.
    pub(in crate::app) fn melee_world_blocks(
        &self,
        live: &LiveEntities,
        hit: ShapeHitInfo,
        target: Option<EntityId>,
        attacker: EntityId,
        damage: Option<&crate::assets::damage::Declaration>,
    ) -> Result<bool, String> {
        if hit.user_data != 0
            && CollisionUserData::decode(hit.user_data)?.component.class_id == self.classes[14]
        {
            return Ok(true);
        }
        let flags = if let Some(target) = target {
            let &index = self
                .shared_indices
                .get(&target)
                .ok_or("melee target header missing")?;
            self.entities[index].entity_flags_d7
        } else {
            0
        };
        let dynamic = if let Some(body) = hit.body_id {
            let world = self
                .world
                .read()
                .map_err(|_| "melee target world poisoned")?;
            let body = world
                .storage
                .bodies
                .get(body)
                .ok_or("melee target body missing")?;
            body.flags & 5 == 0
        } else {
            false
        };
        if flags & 2 == 0 {
            return Ok(!dynamic);
        }
        if let Some(target) = target {
            if self
                .health_component(live, target)?
                .is_some_and(|handle| handle.class_id == self.classes[16])
            {
                let player = self
                    .damage_player
                    .ok_or("melee HealthDoor requires the actual Player damage context")?;
                let can_break = player.id == attacker
                    && (player.greater_strength_modified
                        || damage.is_some_and(|damage| damage.damage_type.0 & 0x40000000000 != 0));
                return Ok(!can_break);
            }
        }
        Ok(false)
    }
    pub(in crate::app) fn melee_hit_region(&self, hit: ShapeHitInfo) -> Result<u32, String> {
        self.collision_hit_region(hit)
    }
    pub(in crate::app) fn melee_entity_flags(&self, id: EntityId) -> Result<u8, String> {
        let &index = self
            .shared_indices
            .get(&id)
            .ok_or("melee entity header absent")?;
        let owner = self.entities.get(index).ok_or("melee entity slot absent")?;
        if owner.retired || owner.shared != id {
            return Err("melee entity generation is stale".into());
        }
        self.entity_flags_d4
            .get(index)
            .copied()
            .ok_or_else(|| "melee entity runtime flags absent".into())
    }
    /// 636630/630890 perform their obstruction/focus rays synchronously,
    /// using the actual all-hit collector sorted by source hit fraction.
    pub(in crate::app) fn melee_ray_contacts<'a>(
        &self,
        live: &LiveEntities,
        queries: &'a mut MeleeQueries,
        request: &Ray,
    ) -> Result<&'a [Contact], String> {
        {
            let world = self.world.read().map_err(|_| "melee ray world poisoned")?;
            world.ray_hits(
                &mut queries.ray,
                &WorldRayRequest {
                    origin: request.origin,
                    direction: request.direction,
                    maximum_fraction: request.maximum_fraction,
                    hit: no_body(),
                    filter: Some(CollisionQueryFilter {
                        layers: request.layers,
                        required_flags: 0x10_0000,
                    }),
                },
                0,
                &mut queries.rays,
            )?;
        }
        queries
            .rays
            .sort_by(|a, b| a.fraction.total_cmp(&b.fraction));
        queries.transferred.clear();
        queries
            .transferred
            .extend(queries.rays.iter().copied().map(Hit::Ray));
        self.prepare_melee_contacts(live, queries)?;
        Ok(&queries.contacts)
    }
    /// Source096240 worker boundary, before636b40 consumes previous requests.
    pub(in crate::app) fn resolve_melee_queries(
        &self,
        queries: &mut MeleeQueries,
    ) -> Result<(), String> {
        let Some(slots) = queries.slots.as_mut() else {
            return Ok(());
        };
        let world = self
            .world
            .read()
            .map_err(|_| "melee canonical world poisoned")?;
        slots.try_resolve_pending(|request, output: &mut QueryHits<Hit>| {
            match request {
                Request::Shape(request) => {
                    world.convex_hits_each(
                        &mut queries.convex,
                        &WorldConvexRequest {
                            cast: ConvexCast {
                                geometry: ConvexQueryGeometry {
                                    shape_type: request.shape.shape_type,
                                    vertices: &request.shape.vertices,
                                    radius: request.shape.radius,
                                },
                                transform: request.transform,
                                displacement: request.direction,
                                tolerance: f32::from_bits(0x3a83_126f),
                                start_tolerance: 0.,
                                hit: no_body(),
                                filter: Some(CollisionQueryFilter {
                                    layers: request.layers,
                                    required_flags: 0x10_0000,
                                }),
                            },
                            maximum_fraction: request.maximum_distance,
                        },
                        &mut |hit, _start| {
                            //096ac0 connects both collectors to one FIFO.
                            //Keep each leaf's initial4/main2 callback order.
                            output.push(Hit::Shape(hit));
                        },
                    )?;
                    //095370's two collector kinds are both meaningful to
                    //6364c0: flags2 enters6340f0, flags4 enters633e50.
                }
                Request::Ray(request) => {
                    let hit = world.ray_closest_hit(
                        &mut queries.ray,
                        &WorldRayRequest {
                            origin: request.origin,
                            direction: request.direction,
                            maximum_fraction: request.maximum_fraction,
                            hit: no_body(),
                            filter: Some(CollisionQueryFilter {
                                layers: request.layers,
                                required_flags: 0x10_0000,
                            }),
                        },
                    )?;
                    if let Some(hit) = hit {
                        output.push(Hit::Ray(hit));
                    }
                }
            }
            Ok(())
        })
    }
    /// Borrowed result scratch survives calls, while actual query completion
    /// transfers its owned hit allocation through092630 and invalidates the ID.
    pub(in crate::app) fn poll_melee_query<'a>(
        &self,
        live: &LiveEntities,
        queries: &'a mut MeleeQueries,
        id: &mut QueryId,
    ) -> Result<Option<&'a [Contact]>, String> {
        if !id.is_valid() {
            return Ok(None);
        }
        if !queries
            .slots
            .as_mut()
            .ok_or("melee query owner absent")?
            .take_completed(id, &mut queries.transferred)
            .map_err(|e| e.to_string())?
        {
            return Ok(None);
        }
        self.prepare_melee_contacts(live, queries)?;
        Ok(Some(&queries.contacts))
    }
    fn prepare_melee_contacts(
        &self,
        live: &LiveEntities,
        queries: &mut MeleeQueries,
    ) -> Result<(), String> {
        let world = self
            .world
            .read()
            .map_err(|_| "melee result world poisoned")?;
        queries.contacts.clear();
        for &hit in &queries.transferred {
            let target = hit.target();
            if let Some(body) = target.body_id {
                let Some(body) = world.storage.bodies.get(body) else {
                    continue;
                };
                if body.user_data != target.user_data {
                    continue;
                }
            }
            let entity = if target.user_data != 0 {
                let index = CollisionUserData::decode(target.user_data)?
                    .entity(|handle| self.owner(handle).ok())?;
                let entity = self
                    .entities
                    .get(index)
                    .ok_or("melee target header absent")?;
                (!entity.retired && live.get(entity.shared).is_some()).then_some(entity.shared)
            } else {
                None
            };
            queries.contacts.push(Contact { hit, entity });
        }
        Ok(())
    }
}
