//! Frame-stamped, read-only query inputs from the canonical gameplay physics
//! world. Renderer FX use the same query implementation and shared geometry.
use bevy::prelude::*;
use std::sync::{Arc, RwLock};

pub(crate) struct PhysicsQueryPublication {
    pub camera: Entity,
    pub frame: u32,
    pub world: Arc<RwLock<crate::gameplay::world_point_query::level::LevelPointQueries>>,
    /// The level collision map and its active owners, for the Havok
    /// particle collisions (bevy_backend: the original queries the hknp
    /// world's compressed meshes).
    pub collision: Option<(Arc<crate::gameplay::navigation::CollisionMap>, Arc<[bool]>)>,
    /// The dynamic machinery bodies, for their moving-body collisions.
    pub moving: Option<Arc<crate::gameplay::havok_fx::dynamic_collide::MovingWorld>>,
}
#[derive(Resource, Default)]
pub(crate) struct PhysicsQueryFrame(pub Option<PhysicsQueryPublication>);
