//! Compose the verified static navigation queries with actual rigid-family bodies.
//! No navigation owner index is interpreted as a generational physics body.
use super::*;
use crate::gameplay::{
    body_storage::BodyStorage,
    collision_body::CollisionBody,
    collision_filter::CollisionQueryFilter,
    collision_hit::ShapeHitInfo,
    ray_query::RayQuantization,
    shape_convex_query::{ConvexCast, ConvexContact, ConvexQueryGeometry},
    shape_ray_query::ShapeRayContact,
    world_convex_query::{WorldConvexQuery, WorldConvexRequest},
    world_point_query::{PointQueryBodies, WorldQueryShape, level::LevelPointQueries},
    world_ray_query::{WorldRayQuery, WorldRayRequest},
};
use std::cell::{Cell, RefCell};
#[derive(Default)]
pub(in crate::app) struct Scratch {
    convex: WorldConvexQuery,
    ray: WorldRayQuery,
    vertices: Vec<[f32; 4]>,
    casts: Vec<ConvexContact>,
    starts: Vec<ConvexContact>,
    rays: Vec<ShapeRayContact>,
    error: Option<String>,
}
pub(in crate::app) struct Collision<'a> {
    navigation: GameCollision<'a>,
    classes: [i32; 2],
    world: &'a LevelPointQueries,
    scratch: &'a RefCell<Scratch>,
}
struct RigidBodies<'a> {
    bodies: &'a BodyStorage,
    classes: [i32; 2],
    invalid: &'a Cell<bool>,
}
impl PointQueryBodies for RigidBodies<'_> {
    fn body(&self, index: u32) -> Option<&CollisionBody> {
        self.bodies.get_by_index(index)
    }
    fn accepts_body(&self, body: &CollisionBody) -> bool {
        if body.user_data == 0 {
            return false;
        }
        match crate::gameplay::collision_user_data::CollisionUserData::decode(body.user_data) {
            Ok(owner) => self.classes.contains(&owner.component.class_id),
            Err(_) => {
                self.invalid.set(true);
                false
            }
        }
    }
}
/// 099610's reciprocal body predicate, expressed in the existing query filter.
fn query_filter(filter: QueryFilter) -> CollisionQueryFilter {
    match filter {
        QueryFilter::Layers(filter) => filter,
        QueryFilter::World(0) => CollisionQueryFilter {
            layers: u32::MAX,
            required_flags: 0,
        },
        QueryFilter::World(word) => CollisionQueryFilter {
            layers: word >> 5,
            required_flags: 1 << (word & 31),
        },
    }
}
fn query_identity(filter: QueryFilter) -> ShapeHitInfo {
    ShapeHitInfo {
        body_id: None,
        material_id: u16::MAX,
        shape_key: u32::MAX,
        filter_info: match filter {
            QueryFilter::World(word) => word,
            _ => 0,
        },
        user_data: 0,
    }
}
impl<'a> Collision<'a> {
    pub(in crate::app) fn new(
        navigation: &'a CollisionWorld,
        scene: &'a crate::app::physics_scene::PhysicsScene,
        world: &'a LevelPointQueries,
        scratch: &'a RefCell<Scratch>,
    ) -> Self {
        let mut state = scratch.borrow_mut();
        state.error = None;
        drop(state);
        Self {
            navigation: GameCollision(navigation),
            classes: scene.player_rigid_classes(),
            world,
            scratch,
        }
    }
    pub(in crate::app) fn take_error(&self) -> Option<String> {
        self.scratch.borrow_mut().error.take()
    }
    fn error(&self, error: String) {
        self.scratch.borrow_mut().error.get_or_insert(error);
    }
    fn accepts_navigation_owner(&self, owner: u32) -> bool {
        !self.navigation.0.physics_owns_placed_owner(owner)
    }
    fn shape(&self, index: usize) -> Result<WorldQueryShape<'_>, String> {
        let shape = self
            .world
            .geometry
            .shapes
            .get(index)
            .and_then(Option::as_ref)
            .ok_or("player query body has no retained geometry")?;
        Ok(WorldQueryShape {
            shapes: &shape.query_shapes,
            materials: Some(self.world),
        })
    }
    fn quantization(&self) -> RayQuantization {
        RayQuantization {
            offset: self.world.storage.quantization.offset[..3]
                .try_into()
                .unwrap(),
            scale: self.world.storage.quantization.scale[..3]
                .try_into()
                .unwrap(),
        }
    }
}
impl PlayerWorld for Collision<'_> {
    fn surface_velocity(&self, hit: &WorldHit, previous_for_dynamic: bool) -> Option<Vec3> {
        // Native c8aae0 validates the full body generation, flags&3 and
        // broadphase membership before reading its actual motion. Static
        // navigation hits never reach this lookup (their IDs are not Havok IDs).
        let body = self.world.storage.bodies.get(hit.body)?;
        if body.flags & 3 == 0 || body.broadphase_id == u32::MAX { return None; }
        let Some(motion) = self.world.storage.motions.get(body.motion_id) else {
            self.error("Player moving contact motion is absent".into());
            return None;
        };
        let xyz = |value: [f32; 4]| Vec3::new(value[0], value[1], value[2]);
        let (linear, angular) = if previous_for_dynamic && body.flags & 5 == 0 {
            // c8aae0 directly reads the completed integration's world-space
            // velocities. These angular lanes are already in world space.
            (motion.previous_step_linear_velocity, motion.previous_step_angular_velocity)
        } else {
            // ca7240 rotates principal-axis angular velocity through the
            // actual motion orientation before taking omega cross radius.
            (motion.linear_velocity, motion.world_angular_velocity())
        };
        Some(xyz(linear) + xyz(angular).cross(hit.position - xyz(motion.pose.center)))
    }
    /// 1b5940 validates the full ID and flags&3, then reads body+78. This
    /// means broadphase publication, not the sleeping/active-list bit8.
    fn body_published(&self, body: u32) -> Option<bool> {
        self.world
            .storage
            .bodies
            .get(body)
            .filter(|body| body.flags & 3 != 0)
            .map(|body| body.broadphase_id != u32::MAX)
    }
    fn cast_shape(
        &self,
        filter: QueryFilter,
        shape: &[Vec3],
        rotation: Mat3,
        from: Vec3,
        displacement: Vec3,
        tolerance: f32,
        cast: &mut Vec<WorldHit>,
        start: Option<&mut Vec<WorldHit>>,
    ) {
        self.cast_shape_filtered(
            filter,
            shape,
            rotation,
            from,
            displacement,
            tolerance,
            cast,
            start,
            &mut |_, _| true,
        );
    }
    fn cast_shape_filtered(
        &self,
        filter: QueryFilter,
        shape: &[Vec3],
        rotation: Mat3,
        from: Vec3,
        displacement: Vec3,
        tolerance: f32,
        cast: &mut Vec<WorldHit>,
        mut start: Option<&mut Vec<WorldHit>>,
        accepts_body: &mut dyn FnMut(u32, bool) -> bool,
    ) {
        self.navigation.cast_shape_where(
            filter,
            shape,
            rotation,
            from,
            displacement,
            tolerance,
            cast,
            start.as_deref_mut(),
            |owner| self.accepts_navigation_owner(owner),
        );
        let invalid = Cell::new(false);
        let mut state = self.scratch.borrow_mut();
        let Scratch {
            convex,
            vertices,
            casts,
            starts,
            error,
            ..
        } = &mut *state;
        vertices.clear();
        vertices.extend(shape.iter().map(|point| point.extend(0.).to_array()));
        let transform = [
            rotation.x_axis.extend(0.).to_array(),
            rotation.y_axis.extend(0.).to_array(),
            rotation.z_axis.extend(0.).to_array(),
            from.extend(1.).to_array(),
        ];
        // The player interface supplies its already constructed convex hull
        // (1b3d00), the same points consumed by the static triangle adapter.
        let request = WorldConvexRequest {
            cast: ConvexCast {
                geometry: ConvexQueryGeometry {
                    shape_type: 5,
                    vertices,
                    radius: 0.,
                },
                transform,
                displacement: displacement.to_array(),
                tolerance: 0.001,
                start_tolerance: tolerance,
                hit: query_identity(filter),
                filter: Some(query_filter(filter)),
            },
            maximum_fraction: 1.,
        };
        let result = convex.all_hits(
            &RigidBodies {
                bodies: &self.world.storage.bodies,
                classes: self.classes,
                invalid: &invalid,
            },
            &self.world.storage.broadphase.query_trees(),
            self.quantization(),
            &request,
            |index| self.shape(index),
            casts,
            starts,
        );
        if let Err(failure) = result {
            error.get_or_insert(failure);
            return;
        }
        if invalid.get() {
            error.get_or_insert_with(|| "canonical player body has invalid userdata".into());
            return;
        }
        for (hits, initial) in [(&*casts, false), (&*starts, true)] {
            if initial && start.is_none() {
                continue;
            }
            for hit in hits {
                let Some(body) = hit.target.body_id else {
                    error.get_or_insert_with(|| "player query returned no actual body ID".into());
                    continue;
                };
                if !accepts_body(body, initial) {
                    continue;
                }
                let Some(record) = self.world.storage.bodies.get(body) else {
                    error.get_or_insert_with(|| "player query returned a stale body generation".into());
                    continue;
                };
                let value = if initial { hit.distance } else { hit.fraction };
                let result = WorldHit {
                    user_data: hit.target.user_data,
                    body_flags: record.flags as u8,
                    material_id: Some(hit.target.material_id),
                    position: Vec3::from_array(hit.position[..3].try_into().unwrap()),
                    normal: Vec3::from_array(hit.normal[..3].try_into().unwrap()),
                    value,
                    body,
                    key: hit.target.shape_key,
                };
                if initial {
                    if let Some(start) = start.as_deref_mut() {
                        start.push(result);
                    }
                } else {
                    cast.push(result);
                }
            }
        }
    }
    fn cast_ray(
        &self,
        filter: QueryFilter,
        from: Vec3,
        direction: Vec3,
        length: f32,
    ) -> Option<RayHit> {
        let navigation = self
            .navigation
            .cast_ray_where(filter, from, direction, length, |owner| {
                self.accepts_navigation_owner(owner)
            });
        let request = WorldRayRequest {
            origin: from.to_array(),
            direction: (direction * length).to_array(),
            maximum_fraction: 1.,
            hit: query_identity(filter),
            filter: Some(query_filter(filter)),
        };
        let invalid = Cell::new(false);
        let mut state = self.scratch.borrow_mut();
        let Scratch {
            ray, rays, error, ..
        } = &mut *state;
        let result = ray.all_hits(
            &RigidBodies {
                bodies: &self.world.storage.bodies,
                classes: self.classes,
                invalid: &invalid,
            },
            &self.world.storage.broadphase.query_trees(),
            self.quantization(),
            &request,
            |index| self.shape(index),
            rays,
        );
        if let Err(failure) = result {
            error.get_or_insert(failure);
            return navigation;
        }
        if invalid.get() {
            error.get_or_insert_with(|| "canonical player body has invalid userdata".into());
            return navigation;
        }
        let rigid = rays
            .iter()
            .min_by(|a, b| a.fraction.total_cmp(&b.fraction))
            .copied()
            .and_then(|hit| {
                let body = hit.target.body_id?;
                Some(RayHit {
                    user_data: hit.target.user_data,
                    body_flags: self.world.storage.bodies.get(body)?.flags as u8,
                    material_id: Some(hit.target.material_id),
                    distance: hit.fraction * length,
                    position: Vec3::from_array(hit.position[..3].try_into().unwrap()),
                    normal: Vec3::from_array(hit.normal[..3].try_into().unwrap()),
                    body,
                    key: hit.target.shape_key,
                })
            });
        match (navigation, rigid) {
            (Some(a), Some(b)) => Some(if a.distance <= b.distance { a } else { b }),
            (a, b) => a.or(b),
        }
    }
    fn physics_material(
        &self,
        hit: &WorldHit,
    ) -> Option<&crate::assets::entities::physics_material::PhysicsMaterial> {
        if let Some(material) = hit.material_id {
            match self.world.storage.materials.material_for_id(material) {
                Ok(material) => Some(material),
                Err(error) => {
                    self.error(error);
                    None
                }
            }
        } else {
            self.navigation.physics_material(hit)
        }
    }
    fn material(&self, hit: &WorldHit) -> Option<&str> {
        self.physics_material(hit)
            .map(|material| material.name.as_str())
    }
    fn surface(&self, hit: &WorldHit) -> crate::gameplay::player::character_proxy::SurfaceFlags {
        let material = self.physics_material(hit);
        crate::gameplay::player::character_proxy::SurfaceFlags {
            allow_mantle: material.is_none_or(|m| m.allow_mantle()),
            allow_vault: material.is_none_or(|m| m.allow_vault()),
        }
    }
}
