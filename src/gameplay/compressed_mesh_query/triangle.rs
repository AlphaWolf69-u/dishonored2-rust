//! 1211510 / packed126c750 triangle ray, including source edge tolerances.
use crate::gameplay::{
    constraint_solver::reciprocal, convex_support::faces::reciprocal_sqrt_estimate,
};
#[derive(Clone, Copy, Debug)]
pub struct TriangleRayHit {
    pub fraction: f32,
    pub normal: [f32; 4],
    pub flags: u32,
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| a[i] + b[i])
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn length_estimate(v: [f32; 3]) -> f32 {
    let squared = dot(v, v);
    reciprocal_sqrt_estimate(squared) * squared
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let squared = dot(v, v);
    let r = reciprocal_sqrt_estimate(squared);
    let f = if squared <= 0. {
        0.
    } else {
        ((3. - squared * r * r) * r) * 0.5
    };
    v.map(|v| v * f)
}

/// `tolerance` is hkcdRayCastTriangle::g_tolerance (default .data value zero),
/// not the physics world's collision or contact tolerance.
pub fn cast(
    vertices: [[f32; 4]; 3],
    origin: [f32; 3],
    direction: [f32; 3],
    maximum_fraction: f32,
    query_flags: u32,
    tolerance: f32,
) -> Option<TriangleRayHit> {
    let [a, b, c] = vertices.map(|v| v[..3].try_into().unwrap());
    let edges = [sub(b, a), sub(c, b), sub(a, c)];
    let normal = cross(edges[0], edges[2]);
    let delta = sub(origin, a);
    let start = dot(delta, normal);
    let end = dot(
        std::array::from_fn(|i| delta[i] + direction[i] * maximum_fraction),
        normal,
    );
    if !(end * start < 0.) {
        return None;
    }
    let fraction = (reciprocal(start - end) * start) * maximum_fraction;
    let side = if start < 0. { 2_u32 } else { 1 };
    let edge_cross = edges.map(|v| cross(v, direction));
    let twice_origin = origin.map(|v| v + v);
    let sides = [
        dot(sub(twice_origin, add(a, b)), edge_cross[0]),
        dot(sub(twice_origin, add(b, c)), edge_cross[1]),
        dot(sub(twice_origin, add(a, c)), edge_cross[2]),
    ];
    let lengths = edges.map(length_estimate);
    let ray_length = length_estimate(direction);
    let epsilon = length_estimate(delta) * f32::from_bits(0x3400_0000) + tolerance;
    let thresholds = lengths.map(|v| ray_length * v * (epsilon + epsilon));
    let mut negative = 0;
    let mut positive = 0;
    for i in 0..3 {
        negative |= u32::from(sides[i] < -thresholds[i]) << i;
        positive |= u32::from(thresholds[i] < sides[i]) << i;
    }
    let mut flags = ((side >> negative) | (side >> positive)) & (query_flags ^ 3) & side;
    if query_flags & 8 != 0 {
        let longest = if lengths[2]
            > (if lengths[1] > lengths[0] {
                lengths[1]
            } else {
                lengths[0]
            }) {
            lengths[2]
        } else {
            if lengths[1] > lengths[0] {
                lengths[1]
            } else {
                lengths[0]
            }
        };
        if length_estimate(normal) * 0.25 * 0.25 <= longest * longest {
            let planes = edge_cross.map(normalize);
            let bisectors = [
                add(planes[2], planes[0]),
                add(planes[1], planes[0]),
                add(planes[2], planes[1]),
            ];
            let sign = start.to_bits() & 0x8000_0000;
            for (vertex, bisector) in [a, b, c].into_iter().zip(bisectors) {
                let delta = sub(origin, vertex);
                let value = f32::from_bits(dot(delta, bisector).to_bits() ^ sign);
                let limit = (length_estimate(delta) * 2. * f32::from_bits(0x3400_0000) + tolerance)
                    * length_estimate(bisector);
                if limit < value {
                    flags = 0;
                }
            }
        }
    }
    if flags == 0 {
        return None;
    }
    let sign = start.to_bits() & 0x8000_0000;
    let normal = normal.map(|v| f32::from_bits(v.to_bits() ^ sign));
    //126c750 packet assembly multiplies normal before the Newton correction.
    let squared = dot(normal, normal);
    let r = reciprocal_sqrt_estimate(squared);
    let correction = 3. - r * squared * r;
    let normal = normal.map(|v| ((v * correction) * r) * 0.5);
    Some(TriangleRayHit {
        fraction,
        normal: [normal[0], normal[1], normal[2], 0.],
        flags,
    })
}
