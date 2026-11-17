//! Shape buildGeometry virtuals ca5f50/cb3820 and their ba06a0 sample map.
//! These samples exist only while08b830 builds its shared shape property.
use crate::gameplay::{
    constraint_solver::{inverse_length, reciprocal},
    havok_fx::convex_hull::Hull as ConvexHull,
    shape_transform::rotation::sin_cos,
};
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// ba06a0: equal-area square-to-sphere map, including the four source folds.
fn direction(u: f32, v: f32) -> [f32; 3] {
    let x = u * 2. - 1.;
    let y = v * 2. - 1.;
    let sum = y + x;
    let difference = y - x;
    let (r, angle, sign) = if y < 0. {
        if x >= 0. {
            let inside = -1. <= difference;
            let r = if inside { -difference } else { difference + 2. };
            let n = if inside { x } else { y + 1. };
            (r, n * reciprocal(r) + 3., if inside { 1. } else { -1. })
        } else {
            let inside = -1. <= sum;
            let r = if inside { -sum } else { sum + 2. };
            let n = if inside { y } else { x + 1. };
            let a = n * reciprocal(r);
            (
                r,
                if inside { 2. - a } else { a + 2. },
                if inside { 1. } else { -1. },
            )
        }
    } else if x < 0. {
        let inside = difference <= 1.;
        let r = if inside { difference } else { 2. - difference };
        let n = if inside { x } else { 1. - y };
        let a = n * reciprocal(r);
        (
            r,
            if inside { 1. - a } else { a + 1. },
            if inside { 1. } else { -1. },
        )
    } else {
        let inside = sum <= 1.;
        let r = if inside { sum } else { 2. - sum };
        let n = if inside { y } else { 1. - x };
        (r, n * reciprocal(r), if inside { 1. } else { -1. })
    };
    let angle = if r == 0. {
        0.
    } else {
        angle * f32::from_bits(0x3fc90fdb)
    };
    let radial = 2. - r * r;
    let radial = inverse_length(radial) * radial * r;
    let (s, c) = sin_cos([angle; 4]);
    [c[0] * radial, s[0] * radial, (1. - r * r) * sign]
}
/// cb3820 shortest-arc quaternion, rotating +Z to normalized endpointA-B.
fn capsule_rotation(axis: [f32; 3]) -> [f32; 4] {
    let cosine = axis[2];
    if cosine > f32::from_bits(0x3f7fff58) {
        return [0., 0., 0., 1.];
    }
    if cosine < -f32::from_bits(0x3f7fff58) {
        return [-1., 0., 0., 0.];
    }
    let d = 0.5 * cosine + 0.5;
    let mut factor = inverse_length(d) * 0.5;
    let perpendicular = [-axis[1], axis[0], 0.];
    if cosine < f32::from_bits(0xbf7fbe77) {
        let one = d - cosine;
        let square = dot(perpendicular, perpendicular);
        factor = (inverse_length(one) * one) * inverse_length(square);
    }
    [
        perpendicular[0] * factor,
        perpendicular[1] * factor,
        perpendicular[2] * factor,
        inverse_length(d) * d,
    ]
}
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let vector = [q[0], q[1], q[2]];
    let projection = dot(vector, v);
    let crossed = cross(vector, v);
    std::array::from_fn(|i| {
        let half = (projection * vector[i] + (q[3] * q[3] - 0.5) * v[i]) + crossed[i] * q[3];
        half + half
    })
}
/// b88380 uses the already-ported b88330/default b88010 hull builder;
/// b928a0(param2=0) exports original positions in native triangle-list order.
pub(super) fn hull(samples: &[[f32; 4]]) -> Result<Vec<[[f32; 3]; 3]>, String> {
    let hull = ConvexHull::build(samples)?;
    Ok(hull
        .triangle_order()
        .into_iter()
        .map(|triangle| {
            hull.triangles[triangle].vertices.map(|vertex| {
                let p = hull.vertices[vertex].point.position;
                [p[0], p[1], p[2]]
            })
        })
        .collect())
}
/// ce1e30 mode1 uses the shared hull owner's complete b8c950 expansion.
pub(super) fn convex(vertices: &[[f32; 4]], radius: f32) -> Result<Vec<[[f32; 3]; 3]>, String> {
    Ok(ConvexHull::build_geometry(vertices, radius)?.geometry_triangles())
}
pub(super) fn sphere(center: [f32; 4], radius: f32) -> Result<Vec<[[f32; 3]; 3]>, String> {
    let mut samples = Vec::with_capacity(256);
    for u in 0..16 {
        for v in 0..16 {
            let d = direction(
                u as f32 * f32::from_bits(0x3d888889),
                v as f32 * f32::from_bits(0x3d888889),
            );
            samples.push([
                d[0] * radius + center[0],
                d[1] * radius + center[1],
                d[2] * radius + center[2],
                0.,
            ]);
        }
    }
    hull(&samples)
}
pub(super) fn capsule(a: [f32; 4], b: [f32; 4], radius: f32) -> Result<Vec<[[f32; 3]; 3]>, String> {
    let delta = std::array::from_fn(|i| a[i] - b[i]);
    let inverse = inverse_length(dot(delta, delta));
    let axis = delta.map(|value| value * inverse);
    let rotation = capsule_rotation(axis);
    let radius = radius - radius * f32::from_bits(0x3c23d70a); //cb4830 removes one percent.
    let mut samples = Vec::with_capacity(256);
    for u in 0..16 {
        for v in 0..16 {
            let d = direction(
                u as f32 * f32::from_bits(0x3d888889),
                v as f32 * f32::from_bits(0x3d888889),
            );
            let p = rotate(rotation, d).map(|value| value * radius);
            let endpoint = if dot(p, axis) < 0. { b } else { a };
            samples.push([
                endpoint[0] + p[0],
                endpoint[1] + p[1],
                endpoint[2] + p[2],
                0.,
            ]);
        }
    }
    hull(&samples)
}
