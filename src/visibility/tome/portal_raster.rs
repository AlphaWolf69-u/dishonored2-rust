//! Packed pixel/edge evaluation for the portal rasterizer (FUN_140a8a080).
//! Each output bit still tests the sign bits of all four edge expressions.

#[cfg(target_arch = "x86_64")]
#[inline]
pub(super) fn coverage_4x4(
    origin: [f32; 4],
    edge_x: [f32; 4],
    edge_y: [f32; 4],
    x: i32,
    y: i32,
) -> u16 {
    use std::arch::x86_64::*;
    // SAFETY: x86_64 supplies SSE; all arrays and temporary vectors have four
    // initialized lanes. No floating-point reassociation or FMA is used.
    unsafe {
        let pixels = _mm_setr_ps(x as f32, (x + 1) as f32, (x + 2) as f32, (x + 3) as f32);
        let bases: [__m128; 4] = std::array::from_fn(|edge| {
            _mm_add_ps(
                _mm_set1_ps(origin[edge]),
                _mm_mul_ps(_mm_set1_ps(edge_x[edge]), pixels),
            )
        });
        let mut result = 0u16;
        for row in 0..4 {
            let py = _mm_set1_ps((y + row) as f32);
            let values: [__m128; 4] = std::array::from_fn(|edge| {
                _mm_add_ps(bases[edge], _mm_mul_ps(_mm_set1_ps(edge_y[edge]), py))
            });
            let combined = _mm_and_ps(
                _mm_and_ps(values[0], values[1]),
                _mm_and_ps(values[2], values[3]),
            );
            result |= (_mm_movemask_ps(combined) as u16) << (row * 4);
        }
        result
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub(super) use coverage_4x4_scalar as coverage_4x4;

#[cfg(any(test, not(target_arch = "x86_64")))]
pub(super) fn coverage_4x4_scalar(
    origin: [f32; 4],
    edge_x: [f32; 4],
    edge_y: [f32; 4],
    x: i32,
    y: i32,
) -> u16 {
    let mut result = 0u16;
    for py in 0..4 {
        for px in 0..4 {
            let mut inside = true;
            for edge in 0..4 {
                let value =
                    origin[edge] + edge_x[edge] * (x + px) as f32 + edge_y[edge] * (y + py) as f32;
                inside &= value.to_bits() & 0x8000_0000 != 0;
            }
            if inside {
                result |= 1 << (py * 4 + px);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_portal_pixels_preserve_scalar_bit_masks() {
        let mut seed = 0x243f6a88u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            match seed & 63 {
                0 => -0.,
                1 => 0.,
                2 => f32::MIN_POSITIVE,
                _ => (seed as i32 as f32) / 2147483648.0,
            }
        };
        for case in 0..8192 {
            let origin = std::array::from_fn(|_| next());
            let dx = std::array::from_fn(|_| next());
            let dy = std::array::from_fn(|_| next());
            let x = (case % 16) * 4;
            let y = ((case / 16) % 16) * 4;
            assert_eq!(
                coverage_4x4(origin, dx, dy, x, y),
                coverage_4x4_scalar(origin, dx, dy, x, y),
                "portal pixel block {case}"
            );
        }
    }
}
