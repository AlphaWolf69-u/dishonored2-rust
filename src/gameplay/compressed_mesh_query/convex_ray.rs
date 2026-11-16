//! Sphere/capsule ray specializations embedded in14126c750.
use crate::gameplay::{
    constraint_solver::{inverse_length, reciprocal},
    convex_support::{faces::reciprocal_sqrt_estimate, ray::ConvexRayHit},
};
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
/// Packed sphere primitive branch, including queryflag4's exit mask141e846a0.
pub fn sphere(
    center: [f32; 4],
    origin: [f32; 3],
    direction: [f32; 3],
    radius: f32,
    maximum_fraction: f32,
    query_flags: u32,
) -> Option<ConvexRayHit> {
    let delta = std::array::from_fn(|i| origin[i] - center[i]);
    let projection = 0. - dot(direction, delta);
    let length_squared = dot(direction, direction);
    let projected = if length_squared == 0. {
        0.
    } else {
        reciprocal(length_squared) * projection
    };
    let closest = std::array::from_fn(|i| projected * direction[i] + delta[i]);
    let discriminant = (dot(closest, closest) - radius * radius) * (0. - length_squared);
    if !(0. <= discriminant) {
        return None;
    }
    let estimate = reciprocal_sqrt_estimate(discriminant);
    let root = if discriminant <= 0. {
        0.
    } else {
        ((3. - discriminant * estimate * estimate) * (estimate * 0.5)) * discriminant
    };
    let entry = (0. - root) + projection;
    let exit = entry < 0. && query_flags & 4 != 0;
    let numerator = if exit { root + projection } else { entry };
    if !(numerator < maximum_fraction * length_squared && 0. <= numerator) {
        return None;
    }
    let fraction = numerator * reciprocal(length_squared);
    let normal: [f32; 3] =
        std::array::from_fn(|i| (fraction * direction[i] + origin[i]) - center[i]);
    let inverse = inverse_length(dot(normal, normal));
    Some(ConvexRayHit {
        fraction,
        normal: [
            normal[0] * inverse,
            normal[1] * inverse,
            normal[2] * inverse,
            0.,
        ],
        flags: if exit { 5 } else { 1 },
    })
}
