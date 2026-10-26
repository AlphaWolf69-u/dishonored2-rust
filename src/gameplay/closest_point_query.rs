//! Closest-point query preparation in cada50, before geometry dispatch.
use super::shape_transform::{relative_rigid, ShapeMatrix};

/// ca6550's query sphere. The vertex W is an encoded support value, always
/// overwritten with 0.5; it is not the point's homogeneous input coordinate.
pub struct PointSphere {
    pub vertex: [f32; 4],
    pub radius: f32,
}
impl PointSphere {
    pub fn new(mut point: [f32; 4], radius: f32) -> Self {
        point[3] = 0.5;
        Self {
            vertex: point,
            radius,
        }
    }

    /// ce2de0's point-to-target-AABB gate, before closest-point dispatch.
    /// `bounds` must come from the target shape's actual world calcAabb.
    pub fn for_query(point: [f32; 4], distance: f32, bounds: [[f32; 4]; 2]) -> Option<Self> {
        // MAXSS selects its second operand on equality and unordered inputs.
        let radius = if 0. > distance { 0. } else { distance };
        let delta: [f32; 3] = std::array::from_fn(|i| {
            let upper = if point[i] < bounds[1][i] {
                point[i]
            } else {
                bounds[1][i]
            };
            let clamped = if upper > bounds[0][i] {
                upper
            } else {
                bounds[0][i]
            };
            clamped - point[i]
        });
        let squared = (delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2];
        // UCOMISS/JA rejects only ordered greater, admitting unordered inputs.
        if !(squared > radius * radius) {
            Some(Self::new(point, 0.))
        } else {
            None
        }
    }
}
impl super::compound_query::prepare::CompoundQueryGeometry for PointSphere {
    fn calculate_bounds(&mut self, matrix: ShapeMatrix) -> Result<[[f32; 4]; 2], String> {
        Ok(super::convex_support::convex_bounds(
            &[self.vertex],
            matrix,
            self.radius,
        ))
    }
    fn convex_vertices(&self) -> Option<(&[[f32; 4]], f32)> {
        Some((std::slice::from_ref(&self.vertex), self.radius))
    }
}

pub fn prepare_transforms(
    query_world: ShapeMatrix,
    target_world: ShapeMatrix,
    distance: f32,
    collector_distance: &mut [f32; 4],
) -> ShapeMatrix {
    let relative = relative_rigid(query_world, target_world);
    // MINPS selects its second operand on equality and unordered comparisons.
    *collector_distance = collector_distance.map(|current| {
        if current < distance {
            current
        } else {
            distance
        }
    });
    relative
}

#[cfg(test)]
#[path = "closest_point_query/tests.rs"]
mod tests;
