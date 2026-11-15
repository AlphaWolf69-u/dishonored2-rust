//! Frame-stamped, read-only query inputs from the canonical gameplay physics
//! world. Renderer FX use the same query implementation and shared geometry.
use bevy::{prelude::*, render::extract_resource::ExtractResource};
use std::sync::Arc;

pub struct PhysicsQueryPublication {
    pub camera: Entity,
    pub frame: u32,
    pub world: Arc<crate::gameplay::world_point_query::snapshot::PointQuerySnapshot>,
    /// The level collision map and its active owners, for the Havok
    /// particle collisions (bevy_backend: the original queries the hknp
    /// world's compressed meshes).
    pub collision: Option<(Arc<crate::gameplay::navigation::CollisionMap>, Arc<[bool]>)>,
    /// The dynamic machinery bodies, for their moving-body collisions.
    pub moving: Option<Arc<crate::gameplay::havok_fx::dynamic_collide::MovingWorld>>,
}
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct PhysicsQueryFrame(pub Option<Arc<PhysicsQueryPublication>>);
