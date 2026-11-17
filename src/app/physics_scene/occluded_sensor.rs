//! Scene adapter for 095280/094f50/092630. The shared query owner retains
//! requests until the world's resolve phase; no sensor creates another world.
use super::*;
use crate::gameplay::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    collision_queries::{QueryHits, QueryId},
    collision_user_data::CollisionUserData,
    entity_lifetime::LiveEntities,
    occluded_sensor::{self as sensor, Hit, Request, Sensor, Target},
    shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
    world_convex_query::{WorldConvexQuery, WorldConvexRequest},
    world_ray_query::{WorldRayQuery, WorldRayRequest},
};

///096240/8af8a0 resolves the prior shared query batch before component result
///callbacks. New submissions made by those callbacks wait for the next batch.
///This owns no queries itself and does not advance a sensor or physics clock.
pub(in crate::app) fn resolve_pending(world: &mut bevy::prelude::World) -> Result<(), String> {
    if !world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>()
        .is_some_and(|clock| clock.advanced())
        || !world.get_resource::<PhysicsScene>().is_some_and(PhysicsScene::occluded_queries_pending)
    { return Ok(()); }
    world.resource_mut::<PhysicsScene>().execute_occluded_queries()
}

#[derive(Default)]
pub(super) struct SensorQueries {
    // The source shares its already-resident query pool with all query kinds.
    // Our typed host pool is acquired only when an actual sensor submits work.
    slots: Option<CollisionQueries<Request, Hit>>,
    convex: WorldConvexQuery,
    ray: WorldRayQuery,
    hits: Vec<ConvexContact>,
    initial: Vec<ConvexContact>,
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
///Source class ranges are captured once from the canonical registry; ordinary
///sensor frames do not walk class names or rebuild metadata dictionaries.
pub(in crate::app) struct GrappleClasses {
    ranges: [[i32; 2]; 8],
}
impl GrappleClasses {
    fn contains(&self, index: usize, id: i32) -> bool {
        self.ranges[index][0] <= id && id <= self.ranges[index][1]
    }
}
impl SensorQueries {
    fn convex_request(world: &LevelPointQueries, convex: &mut WorldConvexQuery,
        hits: &mut Vec<ConvexContact>, initial: &mut Vec<ConvexContact>,
        request: &sensor::ConvexRequest) -> Result<(), String> {
        world.convex_hits(convex, &WorldConvexRequest {
            cast: ConvexCast {
                geometry: ConvexQueryGeometry {shape_type: request.shape_type,
                    vertices: &request.vertices, radius: request.radius},
                transform: request.transform, displacement: request.displacement,
                tolerance: f32::from_bits(0x3a83_126f), start_tolerance: 0., hit: no_body(),
                filter: Some(CollisionQueryFilter {layers: request.layers, required_flags: request.required_flags}),
            }, maximum_fraction: 1.,
        }, hits, initial)
    }
    fn execute(&mut self, world: &LevelPointQueries) -> Result<(), String> {
        let Self {
            slots,
            convex,
            ray,
            hits,
            initial,
        } = self;
        let Some(slots) = slots else {
            return Ok(());
        };
        slots.try_resolve_pending(|request, output: &mut QueryHits<Hit>| {
            match request {
                Request::Convex(request) => {
                    Self::convex_request(world, convex, hits, initial, request)?;
                    for &hit in hits.iter() {
                        if let Some(body_id) = hit.target.body_id {
                            output.push(Hit {body_id, witness: Some(sensor::Witness::Convex(hit))});
                        }
                    }
                }
                Request::RayClosest {origin, direction, maximum_fraction, layers, required_flags} => {
                    let hit = world.ray_closest_hit(ray, &WorldRayRequest {
                        origin: *origin, direction: *direction,
                        maximum_fraction: *maximum_fraction,
                        hit: no_body(),
                        filter: Some(CollisionQueryFilter {layers: *layers, required_flags: *required_flags}),
                    })?;
                    if let Some(hit) = hit {
                        if let Some(body_id) = hit.target.body_id {
                            output.push(Hit {body_id, witness: Some(sensor::Witness::Ray(hit))});
                        }
                    }
                }
                Request::Sphere {
                    origin,
                    radius,
                    layers,
                } => {
                    let vertices = [[0.; 4]];
                    world.convex_hits(
                        convex,
                        &WorldConvexRequest {
                            cast: ConvexCast {
                                geometry: ConvexQueryGeometry {
                                    shape_type: 2,
                                    vertices: &vertices,
                                    radius: *radius,
                                },
                                transform: [
                                    [1., 0., 0., 0.],
                                    [0., 1., 0., 0.],
                                    [0., 0., 1., 0.],
                                    [origin[0], origin[1], origin[2], 1.],
                                ],
                                displacement: [0.; 3],
                                tolerance: f32::from_bits(0x3a83_126f),
                                start_tolerance: 0.,
                                hit: no_body(),
                                filter: Some(CollisionQueryFilter {
                                    layers: *layers,
                                    required_flags: 0,
                                }),
                            },
                            maximum_fraction: 1.,
                        },
                        hits,
                        initial,
                    )?;
                    // 096ac0/c89bc0 has only the main collector: initial
                    // geometry is also published there by cbd060. Never merge
                    // the independent start collector's duplicate witnesses.
                    for hit in hits.iter() {
                        if let Some(body_id) = hit.target.body_id {
                            output.push(Hit { body_id, witness: None });
                        }
                    }
                }
                Request::Ray {
                    origin,
                    direction,
                    layers,
                } => {
                    let hit = world.ray_any_hit(
                        ray,
                        &WorldRayRequest {
                            origin: *origin,
                            direction: *direction,
                            maximum_fraction: 1.,
                            hit: no_body(),
                            filter: Some(CollisionQueryFilter {
                                layers: *layers,
                                required_flags: 0,
                            }),
                        },
                        0,
                    )?;
                    // 094f50 collector3 is an any-hit predicate. The canonical
                    // executor owns shape filtering; no target/self exception
                    // or invented distance cutoff is applied here.
                    if let Some(hit) = hit {
                        output.push(Hit {
                            body_id: hit.target.body_id.unwrap_or(0x00ff_ffff),
                            witness: None,
                        });
                    }
                }
            }
            Ok(())
        })
    }
}
struct Host<'a> {
    scene: &'a PhysicsScene,
    live: &'a LiveEntities,
    queries: &'a mut SensorQueries,
}
impl sensor::Host for Host<'_> {
    fn submit(&mut self, request: Request) -> Result<QueryId, String> {
        self.queries
            .slots
            .get_or_insert_with(CollisionQueries::default)
            .submit(request, self.scene.registry.is_some())
            .map_err(|e| e.to_string())
    }
    fn take_completed(&mut self, id: &mut QueryId, hits: &mut Vec<Hit>) -> Result<bool, String> {
        if !id.is_valid() {
            return Ok(false);
        }
        self.queries
            .slots
            .as_mut()
            .ok_or("sensor query slot owner absent")?
            .take_completed(id, hits)
            .map_err(|e| e.to_string())
    }
    fn cancel(&mut self, id: &mut QueryId) -> Result<(), String> {
        if id.is_valid() {
            self.queries
                .slots
                .as_mut()
                .ok_or("sensor query slot owner absent")?
                .cancel(id)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    fn body_target(&self, body_id: u32) -> Result<Option<Target>, String> {
        let world = self
            .scene
            .world
            .read()
            .map_err(|_| "sensor canonical world poisoned")?;
        let Some(body) = world.storage.bodies.get(body_id) else {
            return Ok(None);
        };
        if body.flags & 3 == 0 {
            return Ok(None);
        }
        let index = CollisionUserData::decode(body.user_data)?
            .entity(|handle| self.scene.owner(handle).ok())?;
        let entity = self
            .scene
            .entities
            .get(index)
            .ok_or("sensor body owner absent")?;
        if entity.retired || self.live.get(entity.shared).is_none() {
            return Ok(None);
        }
        Ok(Some(Target {
            entity: Some(entity.shared),
            actor: None,
        }))
    }
    fn actor_targets(&self, _: [f32; 3], _: f32, _: &mut Vec<Target>) -> Result<(), String> {
        // 6c73f0 queries ONLY arkSwarmManager's three agent partitions;
        // 6c8910 requires a live agent weak generation. No swarm owner/agent
        // is admitted in this runtime yet. Ordinary scene actors and NPCs
        // are not members of this roster and must never be scanned here.
        Ok(())
    }
    fn position(&self, target: Target) -> Result<Option<[f32; 3]>, String> {
        let Some(entity) = target.entity else {
            if target.actor.is_some() {
                return Err("sensor swarm target has no admitted swarm owner".into());
            }
            return Ok(None);
        };
        if self.live.get(entity).is_none() {
            return Ok(None);
        }
        let Some(&index) = self.scene.shared_indices.get(&entity) else {
            return Ok(None);
        };
        let owner = self
            .scene
            .entities
            .get(index)
            .ok_or("sensor target header absent")?;
        if owner.retired || owner.shared != entity {
            return Ok(None);
        }
        // 878230 reads the actual reference actor when present, otherwise
        // the registered entity's own transform; it adds no Spawn-stage gate.
        if let Some(handle) = owner.components.reference_actor {
            let actor = self
                .scene
                .actors
                .actors
                .get(handle.slot)
                .ok_or("sensor reference actor absent")?;
            return Ok(Some(actor.frame.current.position));
        }
        Ok(Some(owner.frame.pose.position))
    }
}
impl PhysicsScene {
    pub(in crate::app) fn grapple_classes(&self) -> Result<GrappleClasses, String> {
        let registry = self.types.classes();
        let mut ranges = [[0; 2]; 8];
        for (slot, name) in ["arkMovable", "arkPickup", "arkGrenade", "arkNPC",
            "arkBreakable", "arkDoor", "arkBloodFlyNest", "arkClimbable"].into_iter().enumerate() {
            let class = registry.hierarchy().registration_order().iter().copied().find(|&index|
                registry.metadata(index).is_some_and(|metadata| registry.name_text(metadata.name) == Some(name.as_bytes())))
                .ok_or_else(|| format!("sensor27 class metadata missing: {name}"))?;
            ranges[slot] = registry.hierarchy().type_range(class)?;
        }
        Ok(GrappleClasses {ranges})
    }
    ///3b2810 and3b49c0 borrow the current canonical body/material/managed
    ///generation. A render instance or mere dynamic body is not a pull target.
    pub(in crate::app) fn choose_grapple_witness(
        &self, live: &LiveEntities, player: crate::gameplay::entity_lifetime::EntityId,
        hits: &[Hit], input: crate::gameplay::powers::far_reach::Input, radius: f32,
        classes: &GrappleClasses,
    ) -> Result<Option<crate::gameplay::powers::far_reach::Acquired>, String> {
        use crate::gameplay::{powers::far_reach::{Acquired, ObjectKind, ObjectTarget},
            player::{character_proxy::WorldHit, grapple::TargetHit}, collision_settings::CollisionBlock};
        use bevy_math::Vec3;
        let world = self.world.read().map_err(|_| "grapple candidate world poisoned")?;
        let mut nearest = None;
        let mut distance = input.length + 0.001;
        let mut targetable = false;
        let mut through_surface = false;
        for hit in hits {
            let Some(sensor::Witness::Convex(contact)) = hit.witness else { continue; };
            let Some(body) = world.storage.bodies.get(hit.body_id) else { continue; };
            if body.user_data != contact.target.user_data { continue; }
            let entity = self.collision_entity(live, &contact.target)?;
            if entity == Some(player) { continue; }
            let type_id = entity.map(|entity| self.types.classes().hierarchy().type_id(live.get(entity)
                .ok_or("grapple candidate entity retired")?.1)).transpose()?;
            let is = |slot| type_id.is_some_and(|id| classes.contains(slot, id));
            let movable = is(0);
            let npc = is(3);
            let mut kind = if is(2) {Some(ObjectKind::Grenade)} else if movable {Some(ObjectKind::Movable)}
                else if is(1) {Some(ObjectKind::Pickup)} else {None};
            if let Some(entity) = entity {
                let &index = self.shared_indices.get(&entity).ok_or("grapple candidate header absent")?;
                let dead = self.entities[index].entity_flags & 0x2800 != 0;
                //Native range1439cab10 is arkBreakable, NOT arkMovable;
                //1439c0350 is arkDoor. Both proved by registration1413c19f0/
                //1413b5290. Ordinary breakable entities are skipped here.
                if !dead && is(4) && !is(5) && !is(6) {continue;}
                if is(7) {
                    //1439caba0 is arkClimbable (1413c1aa0), not Projectile.
                    //Its+360 instigator cannot be replaced by Live.owner.
                    return Err("sensor27 requires the actual arkClimbable instigator owner".into());
                }
            }
            //NPC/ragdoll acquisition must use its actual owner, never the
            //ordinary prop release path. No such runtime actor is admitted.
            if npc { kind = None; }
            let material = world.storage.materials.material_for_id(contact.target.material_id)?;
            if material.can_grapple_through() { through_surface = true; continue; }
            let candidate = contact.fraction * input.length;
            if candidate >= distance { continue; }
            distance = candidate;
            targetable = material.collision.legacy_blocking(CollisionBlock::Vision);
            let position = Vec3::from_array(contact.position[..3].try_into().unwrap());
            let normal = Vec3::from_array(contact.normal[..3].try_into().unwrap());
            let level_body = body.user_data != 0 && CollisionUserData::decode(body.user_data)?.component.class_id == self.classes[14];
            let object = if input.pull_objects {
                if let (Some(entity), Some(kind)) = (entity, kind) {
                    let &index = self.shared_indices.get(&entity).ok_or("grapple candidate header absent")?;
                    let owner = &self.entities[index];
                    if owner.entity_flags & 0x8000 != 0 {
                        let origin = Vec3::from_array(owner.frame.pose.position);
                        let toward = (input.origin - origin).with_z(0.);
                        Some(ObjectTarget {entity, kind, movable, origin,
                            normal: toward / toward.length_squared().max(f32::MIN_POSITIVE).sqrt()})
                    } else { None }
                } else { None }
            } else { None };
            nearest = Some(Acquired {surface: TargetHit {
                hit: WorldHit {body_flags: body.flags, user_data: body.user_data,
                    material_id: Some(contact.target.material_id), position, normal,
                    value: contact.fraction, body: if level_body {0} else {hit.body_id}, key: contact.target.shape_key},
                position: object.map_or(position + normal * radius, |target| target.origin),
                distance: candidate, through_surface,
            }, object});
        }
        if let Some(nearest) = nearest.as_mut() { nearest.surface.through_surface = through_surface; }
        Ok(if targetable {nearest} else {None})
    }
    ///3b4610 directly refines a completed sensor27 sample through the same
    ///world and retained executor, not a synchronous replacement for its FIFO.
    pub(in crate::app) fn refine_sensor_query(&mut self, request: &sensor::ConvexRequest,
        output: &mut Vec<Hit>) -> Result<(), String> {
        let queries = self.sensor_queries.as_mut().ok_or("sensor query owner reentered")?;
        let world = self.world.read().map_err(|_| "sensor query world poisoned")?;
        SensorQueries::convex_request(&world, &mut queries.convex, &mut queries.hits, &mut queries.initial, request)?;
        output.clear();
        output.extend(queries.hits.iter().filter_map(|&hit| hit.target.body_id.map(|body_id|
            Hit {body_id, witness: Some(sensor::Witness::Convex(hit))})));
        Ok(())
    }
    ///095280/095030 share the same source slot/FIFO owner as ordinary sensors.
    ///The request keeps its actual shape lease until completion or cancellation.
    pub(in crate::app) fn submit_sensor_query(&mut self, request: Request) -> Result<QueryId, String> {
        self.sensor_queries.as_mut().ok_or("sensor query owner reentered")?
            .slots.get_or_insert_with(CollisionQueries::default)
            .submit(request, self.registry.is_some()).map_err(|error| error.to_string())
    }
    pub(in crate::app) fn take_sensor_query(&mut self, id: &mut QueryId, output: &mut Vec<Hit>) -> Result<bool, String> {
        if !id.is_valid() { return Ok(false); }
        self.sensor_queries.as_mut().ok_or("sensor query owner reentered")?
            .slots.as_mut().ok_or("sensor query slot owner absent")?
            .take_completed(id, output).map_err(|error| error.to_string())
    }
    pub(in crate::app) fn cancel_sensor_query(&mut self, id: &mut QueryId) -> Result<(), String> {
        if !id.is_valid() { return Ok(()); }
        self.sensor_queries.as_mut().ok_or("sensor query owner reentered")?
            .slots.as_mut().ok_or("sensor query slot owner absent")?
            .cancel(id).map_err(|error| error.to_string())
    }
    pub(in crate::app) fn occluded_queries_pending(&self) -> bool {
        self.sensor_queries
            .as_ref()
            .and_then(|queries| queries.slots.as_ref())
            .is_some_and(CollisionQueries::has_pending)
    }
    pub(in crate::app) fn retire_occluded_sensor_queries(
        &mut self,
        sensor: &mut Sensor,
    ) -> Result<(), String> {
        let queries = self
            .sensor_queries
            .as_mut()
            .ok_or("sensor query owner reentered")?;
        sensor.retire_with_cancel(|id| {
            if id.is_valid() {
                queries
                    .slots
                    .as_mut()
                    .ok_or("sensor query slot owner absent")?
                    .cancel(id)
                    .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
    }
    /// 8af8a0/096240 resolves submitted queries before component result phase4.
    /// Call once at that boundary, not after each sensor submits new work.
    pub(in crate::app) fn execute_occluded_queries(&mut self) -> Result<(), String> {
        let mut queries = self
            .sensor_queries
            .take()
            .ok_or("sensor query owner reentered")?;
        let result = (|| {
            let world = self
                .world
                .read()
                .map_err(|_| "sensor canonical world poisoned")?;
            queries.execute(&world)
        })();
        self.sensor_queries = Some(queries);
        result
    }
    fn with_occluded_sensor(
        &mut self,
        live: &LiveEntities,
        sensor: &mut Sensor,
        action: impl FnOnce(&mut Sensor, &mut Host<'_>) -> Result<(), String>,
    ) -> Result<(), String> {
        let mut queries = self
            .sensor_queries
            .take()
            .ok_or("sensor query owner reentered")?;
        let result = action(
            sensor,
            &mut Host {
                scene: self,
                live,
                queries: &mut queries,
            },
        );
        self.sensor_queries = Some(queries);
        result
    }
    pub(in crate::app) fn gather_occluded_sensor(
        &mut self,
        live: &LiveEntities,
        sensor: &mut Sensor,
    ) -> Result<(), String> {
        self.with_occluded_sensor(live, sensor, |sensor, host| sensor.gather(host))
    }
    pub(in crate::app) fn submit_occluded_sensor(
        &mut self,
        live: &LiveEntities,
        sensor: &mut Sensor,
    ) -> Result<(), String> {
        self.with_occluded_sensor(live, sensor, |sensor, host| sensor.submit(host))
    }
    pub(in crate::app) fn reset_occluded_sensor(
        &mut self,
        live: &LiveEntities,
        sensor: &mut Sensor,
    ) -> Result<(), String> {
        self.with_occluded_sensor(live, sensor, |sensor, host| sensor.reset(host))
    }
    pub(in crate::app) fn retire_occluded_sensor(
        &mut self,
        live: &LiveEntities,
        sensor: &mut Sensor,
    ) -> Result<(), String> {
        self.with_occluded_sensor(live, sensor, |sensor, host| sensor.retire(host))
    }
}
