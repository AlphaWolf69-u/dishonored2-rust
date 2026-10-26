//! cbd7d0's world-space closest-point result assembly. Geometry selection and
//! collector ordering remain with the query owner.
use super::{
    convex_support::distance::ConvexClosestPoint,
    shape_transform::{rotate_vector, transform_point, ShapeMatrix},
};

#[derive(Clone, Copy, Debug)]
pub struct ShapeHitInfo {
    pub body_id: Option<u32>,
    pub material_id: u16,
    pub shape_key: u32,
    pub filter_info: u32,
    pub user_data: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct HitBodyInfo {
    pub body_id: u32,
    pub material_id: u16,
    pub shape_key: u32,
    pub filter_info: u32,
    pub flags: u32,
    pub user_data: u64,
}
impl HitBodyInfo {
    fn from_shape(shape: ShapeHitInfo, flags: u32) -> Self {
        Self {
            body_id: shape.body_id.unwrap_or(0x00ff_ffff),
            material_id: shape.material_id,
            shape_key: shape.shape_key,
            filter_info: shape.filter_info,
            flags,
            user_data: shape.user_data,
        }
    }
}

/// The source's112-byte packet has unspecified padding. Typed fields preserve
/// its defined values without giving meaning to those uninitialized bytes.
#[derive(Clone, Copy, Debug)]
pub struct ClosestPointHit {
    pub position: [f32; 4],
    pub normal: [f32; 4],
    pub distance: f32,
    pub query: HitBodyInfo,
    pub target: HitBodyInfo,
    pub query_type: u64,
}

pub fn publish_closest_point(
    geometry: Option<ConvexClosestPoint>,
    target_world: ShapeMatrix,
    query: ShapeHitInfo,
    target: ShapeHitInfo,
    target_flags: u32,
    submit: impl FnOnce(ClosestPointHit),
) -> bool {
    let Some(geometry) = geometry else {
        return false;
    };
    submit(ClosestPointHit {
        position: transform_point(target_world, geometry.point),
        normal: rotate_vector(target_world, geometry.normal),
        distance: geometry.distance[0],
        query: HitBodyInfo::from_shape(query, u32::MAX),
        target: HitBodyInfo::from_shape(target, target_flags),
        query_type: 4,
    });
    true
}

#[cfg(test)]
#[path = "collision_hit/tests.rs"]
mod tests;
