//! Radius adjustment and witness publication in140d10960 after the GSK solve.
use crate::gameplay::shape_transform::{rotate_vector, transform_point, ShapeMatrix};

pub struct ConvexDistanceSolution {
    pub status: i32,
    pub distance: [f32; 4],
    pub normal: [f32; 4],
    pub witness: [f32; 4],
}
pub struct ConvexClosestPoint {
    pub point: [f32; 4],
    pub normal: [f32; 4],
    pub distance: [f32; 4],
}

/// d10960's complete point path, including the cold GSK solve and publication.
pub fn query_point(
    point: [f32; 4],
    target: &[[f32; 4]],
    relative: ShapeMatrix,
    query_radius: f32,
    target_radius: f32,
    threshold: f32,
) -> Result<Option<ConvexClosestPoint>, String> {
    let mut solution = ConvexDistanceSolution {
        status: 0,
        distance: [0.; 4],
        normal: [0.; 4],
        witness: [0.; 4],
    };
    super::point_distance::solve_point_distance(point, target, relative, &mut solution)?;
    Ok(finish_distance(
        solution,
        query_radius,
        target_radius,
        relative,
        threshold,
    ))
}

/// Consumes an actual solver result, never an AABB/plane approximation.
pub fn finish_distance(
    solution: ConvexDistanceSolution,
    query_radius: f32,
    target_radius: f32,
    relative: ShapeMatrix,
    threshold: f32,
) -> Option<ConvexClosestPoint> {
    let radius = query_radius + target_radius;
    let distance = solution.distance.map(|d| d - radius);
    // UCOMISS/JNC rejects ordered >= and admits unordered comparisons.
    if solution.status > 4 || distance[0] >= threshold {
        return None;
    }
    let point = std::array::from_fn(|i| {
        solution.witness[i] - (solution.distance[i] - target_radius) * solution.normal[i]
    });
    Some(ConvexClosestPoint {
        point: transform_point(relative, point),
        normal: rotate_vector(relative, solution.normal),
        distance,
    })
}
