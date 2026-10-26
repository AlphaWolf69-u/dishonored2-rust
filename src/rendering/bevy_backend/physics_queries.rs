//! Frame-stamped, read-only query inputs from the canonical gameplay physics
//! world. Renderer FX use the same query implementation and shared geometry.
use bevy::{prelude::*, render::extract_resource::ExtractResource};
use std::sync::Arc;

pub struct PhysicsQueryPublication {
    pub camera: Entity,
    pub frame: u32,
    pub world: Arc<crate::gameplay::world_point_query::snapshot::PointQuerySnapshot>,
}
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct PhysicsQueryFrame(pub Option<Arc<PhysicsQueryPublication>>);
