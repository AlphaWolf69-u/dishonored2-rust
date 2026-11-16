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
impl SensorQueries {
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
            match *request {
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
                                    radius,
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
                                    layers,
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
                            output.push(Hit { body_id });
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
                            origin,
                            direction,
                            maximum_fraction: 1.,
                            hit: no_body(),
                            filter: Some(CollisionQueryFilter {
                                layers,
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
