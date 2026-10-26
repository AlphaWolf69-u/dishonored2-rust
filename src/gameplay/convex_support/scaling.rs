//! 140ce2190: convex scale clamping and radius-preserving vertex adjustment.
use crate::gameplay::constraint_solver::reciprocal;

fn min(a: f32, b: f32) -> f32 {
    if a < b {
        a
    } else {
        b
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}
fn abs(value: f32) -> f32 {
    f32::from_bits(value.to_bits() & 0x7fff_ffff)
}
fn signed(value: f32, sign: f32) -> f32 {
    f32::from_bits(value.to_bits() ^ (sign.to_bits() & 0x8000_0000))
}

/// Updates XYZ scale and radius, returns the accompanying four-lane offset.
/// The source preserves scale W even when the offset calculation uses that lane.
pub fn adjust_scale(
    shape_type: u8,
    vertices: &[[f32; 4]],
    keep_radius: bool,
    scale: &mut [f32; 4],
    radius: &mut f32,
) -> Result<[f32; 4], String> {
    let floor = f32::from_bits(0x3a83_126f);
    let magnitude = scale.map(|v| max(abs(v), floor));
    let clamped: [f32; 4] = std::array::from_fn(|i| signed(magnitude[i], scale[i]));
    let mut adjusted = clamped;
    let mut offset = [0.; 4];
    if matches!(shape_type, 2 | 3) {
        *radius = magnitude[0] * *radius;
        adjusted[1] = adjusted[0];
        adjusted[2] = adjusted[0];
    } else if !keep_radius
        && abs(*radius) > f32::from_bits(0x3727_c5ac)
        && !(0..3).all(|i| abs(clamped[i] - 1.) <= f32::from_bits(0x3400_0000))
    {
        let first = *vertices
            .first()
            .ok_or("convex radius adjustment requires vertices")?;
        let mut lower = first;
        let mut upper = first;
        for vertex in &vertices[1..] {
            for lane in 0..4 {
                lower[lane] = min(lower[lane], vertex[lane]);
                upper[lane] = max(upper[lane], vertex[lane]);
            }
        }
        let center: [f32; 4] = std::array::from_fn(|i| (upper[i] + lower[i]) * 0.5);
        let half: [f32; 4] = std::array::from_fn(|i| (upper[i] - lower[i]) * 0.5);
        let outer: [f32; 4] = std::array::from_fn(|i| (*radius + half[i]) * magnitude[i]);
        let reduced = if (0..3).all(|i| *radius < outer[i]) {
            *radius
        } else {
            min(outer[2], min(outer[1], outer[0]))
        };
        *radius = reduced;
        for lane in 0..4 {
            let value = if half[lane] == 0. {
                0.
            } else {
                reciprocal(half[lane]) * (outer[lane] - reduced)
            };
            adjusted[lane] = signed(max(value, floor), clamped[lane]);
            offset[lane] = (clamped[lane] - adjusted[lane]) * center[lane];
        }
    }
    scale[..3].copy_from_slice(&adjusted[..3]);
    Ok(offset)
}
