//! ca2490/ca2350/ca22b0 supporting-face virtuals for vector convex shapes.
//! Packed face IDs retain the source's four byte indices and winding; this
//! selects support features directly, without constructing a replacement hull.
use super::{
    contact_faces::SelectedFace, faces::reciprocal_sqrt_estimate, pair_distance::PairCache,
};
use crate::gameplay::havok_fx::convex_hull::invert_transposed;
fn sub(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|i| a[i] - b[i])
}
fn dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn cross(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
        a[3] * b[3] - b[3] * a[3],
    ]
}
fn normalize(value: [f32; 4]) -> [f32; 4] {
    let squared = dot(value, value);
    let estimate = reciprocal_sqrt_estimate(squared);
    if 0. < squared {
        let factor = if squared <= 0. {
            0.
        } else {
            ((3. - estimate * squared * estimate) * estimate) * 0.5
        };
        value.map(|v| v * factor)
    } else {
        value
    }
}
fn vertex(vertices: &[[f32; 4]], id: u8) -> Result<[f32; 4], String> {
    vertices
        .get(usize::from(id))
        .copied()
        .ok_or_else(|| format!("support face vertex {id} absent"))
}
/// ca2350: its diagonal-cross virtual also works for the repeated fourth
/// vertex of a triangle; minimum half angle is exactly zero.
pub(crate) fn face(vertices: &[[f32; 4]], code: u32) -> Result<SelectedFace, String> {
    let ids = code.to_be_bytes();
    let a = vertex(vertices, ids[0])?;
    let b = vertex(vertices, ids[1])?;
    let c = vertex(vertices, ids[2])?;
    let d = vertex(vertices, ids[3])?;
    let mut plane = normalize(cross(sub(c, a), sub(d, b)));
    plane[3] = 0. - dot(plane, a);
    Ok(SelectedFace {
        index: code,
        plane,
        minimum_half_angle: 0,
    })
}
/// ca22b0 publishes exactly four vertices in byte24,16,8,0 order.
pub(crate) fn vertices(
    input: &[[f32; 4]],
    code: u32,
    output: &mut [[f32; 4]; 16],
) -> Result<usize, String> {
    for (out, id) in output.iter_mut().zip(code.to_be_bytes()) {
        *out = vertex(input, id)?;
    }
    Ok(4)
}
/// ae12e0 is already ported by the common convex-hull owner. Its input is
/// three column vectors here; adapt that layout to its transposed entry.
fn inside_triangle(origin: [f32; 4], a: [f32; 4], b: [f32; 4], point: [f32; 4]) -> bool {
    let a = sub(a, origin);
    let b = sub(b, origin);
    let n = cross(a, b);
    let matrix = invert_transposed(
        [a[0], b[0], n[0], 0.],
        [a[1], b[1], n[1], 0.],
        [a[2], b[2], n[2], 0.],
    );
    // On a singular matrix ae12e0 leaves its source/destination unchanged.
    let matrix = matrix.unwrap_or([
        a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3], n[0], n[1], n[2], n[3],
    ]);
    let relative = sub(point, origin);
    let x = (relative[1] * matrix[4] + relative[0] * matrix[0]) + relative[2] * matrix[8];
    let y = (relative[1] * matrix[5] + relative[0] * matrix[1]) + relative[2] * matrix[9];
    let lower = f32::from_bits(0xba83_126f); //141ae87bc
    let upper = f32::from_bits(0x3f80_20c5); //141c44820
    lower <= x && lower <= y && y + x <= upper
}
pub(crate) fn select(
    input: &[[f32; 4]],
    direction: [f32; 3],
    witness: [f32; 4],
    cache: PairCache,
    target: bool,
    previous: Option<u32>,
) -> Result<SelectedFace, String> {
    let direction = [direction[0], direction[1], direction[2], 0.];
    if let Some(code) = previous {
        let selected = face(input, code)?;
        if f32::from_bits(0x3f7d_70a4) < dot(selected.plane, direction.map(|v| -v)) {
            let ids = code.to_be_bytes();
            let [a, b, c, d] = [
                vertex(input, ids[0])?,
                vertex(input, ids[1])?,
                vertex(input, ids[2])?,
                vertex(input, ids[3])?,
            ];
            if inside_triangle(a, b, d, witness) || inside_triangle(c, b, d, witness) {
                return Ok(selected);
            }
        }
    }
    if input.is_empty() || input.len() > usize::from(u8::MAX) {
        return Err("support face vertex count exceeds source byte IDs".into());
    }
    let query = usize::from(cache.0[4] >> 6);
    let count = if target {
        usize::from(cache.0[4] & 3)
    } else {
        query
    };
    let offset = if target { query } else { 0 };
    if !(1..=3).contains(&count) || offset + count > 4 {
        return Err("support face GSK feature invalid".into());
    }
    let a = cache.0[offset];
    let mut b = if count > 1 {
        cache.0[offset + 1]
    } else {
        u8::MAX
    };
    let mut c = if count > 2 {
        cache.0[offset + 2]
    } else {
        u8::MAX
    };
    let bias = f32::from_bits(0x3c23_d70a); //141ab3810
    let maximum = f32::from_bits(0x7f7f_ffee); //1423cce10
    if count == 1 {
        b = u8::MAX;
        let mut score = maximum;
        let mut squared = 1.;
        let origin = vertex(input, a)?;
        for (index, &point) in input.iter().enumerate() {
            let delta = std::array::from_fn(|i| (point[i] - origin[i]) + bias * direction[i]);
            let projection = dot(delta, direction);
            let signed = f32::from_bits(
                (projection.to_bits() & 0x8000_0000) ^ (projection * projection).to_bits(),
            );
            let distance = dot(delta, delta);
            if signed * squared < distance * score {
                b = index as u8;
                score = signed;
                squared = distance;
            }
        }
    }
    if count <= 2 {
        let origin = vertex(input, a)?;
        let axis = sub(vertex(input, b)?, origin);
        let estimate = reciprocal_sqrt_estimate(dot(axis, axis));
        let axis = axis.map(|v| v * estimate); //ca2490 intentionally uses raw RSQRTPS.
        let projection = dot(axis, direction);
        let remainder = std::array::from_fn(|i| direction[i] - projection * axis[i]);
        let normal = cross(remainder, axis);
        let plane = 0. - dot(normal, origin);
        let side = (0. - dot(remainder, origin)) + bias;
        let mut best = -maximum;
        let mut squared = 1.;
        c = u8::MAX;
        for (index, &point) in input.iter().enumerate() {
            if index == usize::from(a) || index == usize::from(b) {
                continue;
            }
            let distance = (side + point[1] * remainder[1] + point[2] * remainder[2])
                + point[0] * remainder[0];
            let distance = distance * distance;
            let perpendicular =
                (point[2] * normal[2] + point[0] * normal[0]) + (plane + point[1] * normal[1]);
            let perpendicular = perpendicular * perpendicular;
            if distance * best < perpendicular * squared {
                best = perpendicular;
                squared = distance;
                c = index as u8;
            }
        }
    }
    let origin = vertex(input, a)?;
    let mut second = vertex(input, b)?;
    let mut third = vertex(input, c)?;
    let mut normal = normalize(cross(sub(third, second), sub(second, origin)));
    if 0. <= dot(normal, direction) {
        std::mem::swap(&mut b, &mut c);
        std::mem::swap(&mut second, &mut third);
        normal = normal.map(|v| -v);
    }
    let edges = [
        normalize(sub(second, origin)),
        normalize(sub(third, second)),
        normalize(sub(origin, third)),
    ];
    let origins = [origin, second, third];
    let mut best = -maximum;
    let mut insertion = 0;
    let mut fourth = u8::MAX;
    for (index, &point) in input.iter().enumerate() {
        for edge in 0..3 {
            let delta =
                std::array::from_fn(|i| (point[i] - origins[edge][i]) + bias * direction[i]);
            let alignment = dot(normalize(cross(edges[edge], delta)), normal);
            if best < alignment && dot(delta, normal).abs() < f32::from_bits(0x3d88_8889) {
                best = alignment;
                insertion = edge + 1;
                fourth = index as u8;
            }
        }
    }
    let mut ids = [a, b, c, c];
    if f32::from_bits(0x3f66_6666) < best {
        //141ac95dc
        for index in (insertion + 1..4).rev() {
            ids[index] = ids[index - 1];
        }
        ids[insertion] = fourth;
    }
    let mut minimum = 0;
    for index in 1..4 {
        if ids[index] < ids[minimum] {
            minimum = index;
        }
    }
    ids.rotate_left(minimum);
    for index in 0..4 {
        if ids[index] == u8::MAX {
            for next in 1..4 {
                let value = ids[(index + next) & 3];
                if value != u8::MAX {
                    ids[index] = value;
                    break;
                }
            }
        }
    }
    face(input, u32::from_le_bytes(ids))
}
