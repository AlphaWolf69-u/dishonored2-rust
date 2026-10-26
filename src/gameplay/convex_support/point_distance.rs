//! Cold-cache point branch of141212780, as configured by d10960.
//! A single query vertex cannot improve its support projection, so only the
//! target simplex grows. General convex/convex queries require their own path.
use super::{
    distance::ConvexDistanceSolution,
    feature_query::{query_penetration, ConvexFeatures},
    penetration::PenetrationSupport,
    seed::{dot, inverse_length, subtract},
    vertices::support_pair,
};
use crate::gameplay::shape_transform::{inverse_rotate_vector, ShapeMatrix};
#[path = "point_distance/simplex.rs"]
mod simplex;

/// Keeps the source's partial-output behavior for status5 (distance limit).
pub fn solve_point_distance(
    point: [f32; 4],
    target: &[[f32; 4]],
    relative: ShapeMatrix,
    output: &mut ConvexDistanceSolution,
) -> Result<(), String> {
    let first = *target
        .first()
        .ok_or("point distance requires target vertices")?;
    let mut mapped = inverse_rotate_vector(relative, subtract(first, relative[3]));
    mapped[3] = first[3];
    let mut features = ConvexFeatures {
        query: [point; 4],
        target: [mapped; 4],
        query_count: 1,
        target_count: 1,
    };
    let mut epsilon = f32::from_bits(0x3727_c5ac);
    let support = PenetrationSupport {
        query: &[point],
        target,
        relative,
    };
    loop {
        epsilon *= f32::from_bits(0x3fb3_3333);
        let (direction, force) =
            simplex::reduce(point, &mut features.target, &mut features.target_count)?;
        let squared = dot(direction, direction);
        let factor = inverse_length(squared);
        let normal = direction.map(|v| v * factor);
        let separation = dot(subtract(point, features.target[0]), direction) * factor;
        if force || separation < epsilon || squared < ((epsilon * epsilon) * epsilon) * epsilon {
            *output = query_penetration(&mut features, &support)?;
            return Ok(());
        }
        let pair = support_pair(&[point], target, normal.map(|v| -v), relative)?;
        let mut next = pair.target_in_query;
        next[3] = pair.target[3];
        let distance = dot(subtract(point, next), normal);
        if f32::from_bits(0x7f7f_ffee) < distance {
            output.status = 5;
            output.distance = [distance; 4];
            output.normal = normal;
            return Ok(());
        }
        let improvement = dot(subtract(next, features.target[0]), normal);
        let duplicate = (1..features.target_count)
            .any(|i| dot(subtract(next, features.target[i]), normal) < epsilon);
        if epsilon < improvement && !duplicate {
            if features.target_count >= 4 {
                return Err("point simplex failed to reduce before support insertion".into());
            }
            features.target[features.target_count] = next;
            features.target_count += 1;
            continue;
        }
        let distance = dot(subtract(point, features.target[0]), normal);
        output.status = 0;
        output.distance = [distance; 4];
        output.normal = normal;
        output.witness = point;
        return Ok(());
    }
}

#[cfg(test)]
#[path = "point_distance/tests.rs"]
mod tests;
