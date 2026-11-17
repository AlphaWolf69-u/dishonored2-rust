//! Windblast impulses use actual component/body identities and canonical
//! WorldStorage publication. No render-placement body is manufactured here.
use super::*;
use crate::gameplay::entity_lifetime::{EntityId, LiveEntities};
use crate::gameplay::{
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    powers::blast::{Capsule, CapsuleQuery},
    ray_query::RayQuantization,
    world_ray_query::{WorldRayQuery, WorldRayRequest},
};

#[derive(Default)]
pub(in crate::app) struct TargetsQuery {
    capsule: CapsuleQuery,
    broadphase: crate::gameplay::broadphase::OverlapWorkspace,
    bodies: Vec<u32>,
}

#[derive(Clone, Copy)]
pub(in crate::app) struct Target {
    pub entity: EntityId,
    pub body: u32,
    pub center: [f32; 3],
    pub bounds: [[f32; 4]; 2],
    pub movable: bool,
}

impl PhysicsScene {
    ///1404e10d0 resolves the current entity center at UpdateResults, rather
    ///than treating a previous query packet's pose as current world state.
    pub(in crate::app) fn windblast_refresh_target(
        &self,
        live: &LiveEntities,
        target: Target,
    ) -> Result<Option<Target>, String> {
        if live.get(target.entity).is_none() {
            return Ok(None);
        }
        let Some(&index) = self.shared_indices.get(&target.entity) else {
            return Ok(None);
        };
        let entity = self
            .entities
            .get(index)
            .ok_or("windblast refreshed entity is absent")?;
        let Some(handle) = entity
            .components
            .entries
            .values()
            .iter()
            .find_map(|packed| {
                ComponentHandle::from_packed(*packed)
                    .ok()
                    .filter(|handle| self.is_rigid_class(handle.class_id))
            })
        else {
            return Ok(None);
        };
        let component = self.rigid_component(handle)?;
        if component.owner.flags & 0xc != 8 {
            return Ok(None);
        }
        let Some(body_id) = component.body() else {
            return Ok(None);
        };
        let world = self
            .world
            .read()
            .map_err(|_| "windblast canonical world is poisoned")?;
        let body = world
            .storage
            .bodies
            .get(body_id)
            .ok_or("windblast refreshed body is stale")?;
        let geometry = world
            .geometry
            .shapes
            .get(body.shape)
            .and_then(Option::as_ref)
            .ok_or("windblast refreshed shape is absent")?;
        let bounds = geometry
            .query_shapes
            .bounds(geometry.query_shapes.root(), body.transform)?;
        Ok(Some(Target {
            entity: target.entity,
            body: body_id,
            bounds,
            movable: component.start_behavior == Some(2),
            center: std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5),
        }))
    }
    pub(in crate::app) fn windblast_rigid_targets(
        &self,
        live: &LiveEntities,
        capsule: Capsule,
        layers: u32,
        query: &mut TargetsQuery,
        out: &mut Vec<Target>,
    ) -> Result<(), String> {
        out.clear();
        let world = self
            .world
            .read()
            .map_err(|_| "windblast canonical world is poisoned")?;
        let bounds = std::array::from_fn(|corner| {
            std::array::from_fn(|axis| {
                if axis < 3 {
                    if corner == 0 {
                        capsule.ends[0][axis].min(capsule.ends[1][axis]) - capsule.radius
                    } else {
                        capsule.ends[0][axis].max(capsule.ends[1][axis]) + capsule.radius
                    }
                } else {
                    0.
                }
            })
        });
        let packed = world
            .storage
            .quantization
            .packed
            .pack_bounds(bounds, [0.; 4]);
        query.bodies.clear();
        world.storage.broadphase.filtered_overlaps_with_workspace(
            packed,
            &world.storage.quantization.packed,
            &mut query.broadphase,
            |_| true,
            |_, id| {
                world
                    .storage
                    .bodies
                    .get_by_index(id & 0x00ff_ffff)
                    .is_some_and(|body| {
                        (CollisionQueryFilter {
                            layers,
                            required_flags: 0,
                        })
                        .accepts(body.filter_info)
                    })
            },
            &mut query.bodies,
        )?;
        for &body_index in &query.bodies {
            let body = world
                .storage
                .bodies
                .get_by_index(body_index & 0x00ff_ffff)
                .ok_or("windblast query body is absent")?;
            if body.user_data == 0 {
                continue;
            }
            let handle =
                crate::gameplay::collision_user_data::CollisionUserData::decode(body.user_data)?
                    .component;
            if !self.is_rigid_class(handle.class_id) {
                continue;
            }
            let component = self.rigid_component(handle)?;
            let entity = self
                .entities
                .get(
                    component
                        .owner
                        .entity
                        .ok_or("windblast rigid owner is absent")?,
                )
                .ok_or("windblast rigid entity is absent")?;
            if live.get(entity.shared).is_none() {
                continue;
            }
            if component.owner.flags & 0xc != 8 {
                continue;
            }
            let Some(body) = component.body() else {
                continue;
            };
            let body_id = body;
            let body = world
                .storage
                .bodies
                .get(body_id)
                .ok_or("windblast body generation is stale")?;
            if body.broadphase_id == u32::MAX
                || !(CollisionQueryFilter {
                    layers,
                    required_flags: 0,
                })
                .accepts(body.filter_info)
            {
                continue;
            }
            let geometry = world
                .geometry
                .shapes
                .get(body.shape)
                .and_then(Option::as_ref)
                .ok_or("windblast target has no retained shape")?;
            if !query
                .capsule
                .intersects(capsule, &geometry.query_shapes, body.transform)?
            {
                continue;
            }
            let bounds = geometry
                .query_shapes
                .bounds(geometry.query_shapes.root(), body.transform)?;
            out.push(Target {
                entity: entity.shared,
                body: body_id,
                bounds,
                //RigidBody vtable141c745c8 +178 ->140808e10: +fe==2.
                movable: component.start_behavior == Some(2),
                center: std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5),
            });
        }
        Ok(())
    }
    /// 1404e10d0 admits any of three source rays that reaches the target,
    /// misses, or first hits an authored wind-permeable material.
    pub(in crate::app) fn windblast_visible(
        &self,
        query: &mut WorldRayQuery,
        origin: [f32; 3],
        target: Target,
        samples: &[[f32; 3]; 7],
        layers: u32,
    ) -> Result<bool, String> {
        let world = self
            .world
            .read()
            .map_err(|_| "windblast canonical world is poisoned")?;
        for point in &samples[..3] {
            let request = WorldRayRequest {
                origin,
                direction: std::array::from_fn(|i| point[i] - origin[i]),
                maximum_fraction: 1.,
                hit: ShapeHitInfo {
                    body_id: None,
                    material_id: u16::MAX,
                    shape_key: u32::MAX,
                    filter_info: 0,
                    user_data: 0,
                },
                filter: Some(CollisionQueryFilter {
                    layers,
                    required_flags: 0,
                }),
            };
            let hit = query.closest_hit(
                &world.storage.bodies,
                &world.storage.broadphase.query_trees(),
                RayQuantization {
                    offset: world.storage.quantization.offset[..3].try_into().unwrap(),
                    scale: world.storage.quantization.scale[..3].try_into().unwrap(),
                },
                &request,
                |index| {
                    let shape = world
                        .geometry
                        .shapes
                        .get(index)
                        .and_then(Option::as_ref)
                        .ok_or("windblast occluder shape absent")?;
                    Ok(WorldQueryShape {
                        shapes: &shape.query_shapes,
                        materials: None,
                    })
                },
            )?;
            match hit {
                None => return Ok(true),
                Some(hit) if hit.target.body_id == Some(target.body) => return Ok(true),
                Some(hit) => {
                    if world
                        .storage
                        .materials
                        .material_for_id(hit.target.material_id)?
                        .can_windblast_through()
                    {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
    pub(in crate::app) fn windblast_impulse(
        &mut self,
        live: &LiveEntities,
        entity: EntityId,
        impulse: [f32; 3],
        position: [f32; 3],
    ) -> Result<bool, String> {
        if live.get(entity).is_none() {
            return Err("windblast target generation is stale".into());
        }
        let index = *self
            .shared_indices
            .get(&entity)
            .ok_or("windblast physics entity is absent")?;
        let target = self
            .entities
            .get(index)
            .ok_or("windblast physics entity index is invalid")?;
        let Some(handle) = target
            .components
            .entries
            .values()
            .iter()
            .find_map(|packed| {
                ComponentHandle::from_packed(*packed)
                    .ok()
                    .filter(|handle| self.is_rigid_class(handle.class_id))
            })
        else {
            return Ok(false);
        };
        let component = self.rigid_component(handle)?;
        if component.owner.flags & 0xc != 8 {
            return Ok(false);
        }
        let body = component.body().ok_or("windblast rigid body is absent")?;
        let mut registry = self
            .registry
            .take()
            .ok_or("windblast component registry is absent")?;
        let result = (|| {
            scene_publication_events!(self, &mut registry, events);
            let mut world = self
                .world
                .write()
                .map_err(|_| "windblast canonical world is poisoned")?;
            let world = &mut *world;
            let impulse = [impulse[0], impulse[1], impulse[2], 0.0];
            //809c90's all-zero application point dispatches the center
            //impulse overload; preserve that special source representation.
            let point =
                (position != [0.0; 3]).then_some([position[0], position[1], position[2], 0.0]);
            world
                .storage
                .apply_body_impulse(&mut world.geometry, &mut events, body, impulse, point)
        })();
        self.registry = Some(registry);
        result
    }
}
