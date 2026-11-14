//! Frame-stamped, read-only query inputs from the canonical gameplay physics
//! world. Renderer FX use the same query implementation and shared geometry.
use bevy::prelude::*;
use std::sync::{Arc, RwLock};

pub(crate) struct PhysicsQueryPublication {
    pub camera: Entity,
    pub frame: u32,
    pub world: Arc<RwLock<crate::gameplay::world_point_query::level::LevelPointQueries>>,
}
#[derive(Resource, Default)]
pub(crate) struct PhysicsQueryFrame(pub Option<PhysicsQueryPublication>);
