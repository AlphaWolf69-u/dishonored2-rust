//! Packed operations from FUN_140a71b90/140a76f30 and 140a82de7..140a82e58.
//! These replace scalar arithmetic, preserving operation and operand order.
use std::arch::x86_64::*;

/// FUN_140a82710 at 0x140a8336f/0x140a83378. CVTT returns INT_MIN for
/// unordered/out-of-range inputs; Rust's saturating float cast differs there.
#[inline(always)]
pub(super) fn truncate_i32(value: f32) -> i32 {
    // SAFETY: x86_64 guarantees SSE; the operation has no memory operands.
    unsafe { _mm_cvttss_si32(_mm_set_ss(value)) }
}

/// FUN_140a85bd0 at 0x140a86749 converts all four portal edges together.
#[inline(always)]
pub(super) fn truncate_i32x4(values: [f32; 4]) -> [i32; 4] {
    let mut result = [0; 4];
    // SAFETY: both unaligned accesses address complete four-element arrays.
    unsafe {
        _mm_storeu_si128(
            result.as_mut_ptr().cast(),
            _mm_cvttps_epi32(_mm_loadu_ps(values.as_ptr())),
        );
    }
    result
}

#[inline]
unsafe fn sum_yxzw(value: __m128) -> __m128 {
    let first = _mm_add_ss(_mm_shuffle_ps::<0x55>(value, value), value);
    let second = _mm_add_ss(first, _mm_shuffle_ps::<0xaa>(value, value));
    let total = _mm_add_ss(second, _mm_shuffle_ps::<0xff>(value, value));
    _mm_shuffle_ps::<0>(total, total)
}

pub(super) fn object_distance_rejection_mask(
    record: [[f32; 4]; 2],
    point: [f32; 3],
    scale_squared: [f32; 4],
) -> u32 {
    // SAFETY: each unaligned load addresses a complete four-float array.
    // x86_64 guarantees the SSE operations used by the original predicate.
    unsafe {
        let p = _mm_set_ps(0., point[2], point[1], point[0]);
        let lower = _mm_loadu_ps(record[0].as_ptr());
        let upper = _mm_loadu_ps(record[1].as_ptr());
        let nearest = _mm_min_ps(upper, _mm_max_ps(lower, p));
        let xyz = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));
        let delta = _mm_and_ps(_mm_sub_ps(nearest, p), xyz);
        let distance = _mm_mul_ps(
            sum_yxzw(_mm_mul_ps(delta, delta)),
            _mm_loadu_ps(scale_squared.as_ptr()),
        );
        let far = _mm_shuffle_ps::<0xff>(upper, upper);
        let near = _mm_shuffle_ps::<0xff>(lower, lower);
        _mm_movemask_ps(_mm_or_ps(
            _mm_cmple_ps(far, distance),
            _mm_cmplt_ps(distance, near),
        )) as u32
    }
}

/// Caller validates all active indices before entering this loop.
pub(super) unsafe fn plane_minimum(
    planes: &[[f32; 4]],
    input: [[f32; 4]; 2],
    active: &[usize],
    maximum_corner: bool,
    minimum: f32,
) -> f32 {
    let lower = _mm_loadu_ps(input[0].as_ptr());
    let upper = _mm_loadu_ps(input[1].as_ptr());
    let zero = _mm_setzero_ps();
    let all = _mm_castsi128_ps(_mm_set1_epi32(-1));
    let distance = |index: usize| {
        let plane = _mm_loadu_ps(planes.as_ptr().add(index).cast());
        let mut mask = _mm_cmpgt_ps(plane, zero);
        if !maximum_corner {
            mask = _mm_xor_ps(mask, all);
        }
        let corner = _mm_or_ps(_mm_and_ps(mask, upper), _mm_andnot_ps(mask, lower));
        sum_yxzw(_mm_mul_ps(corner, plane))
    };
    let mut result = _mm_set1_ps(minimum);
    let mut groups = active.chunks_exact(4);
    for group in &mut groups {
        let first = _mm_min_ps(distance(group[0]), distance(group[1]));
        let second = _mm_min_ps(distance(group[2]), distance(group[3]));
        result = _mm_min_ps(result, _mm_min_ps(first, second));
    }
    for &index in groups.remainder() {
        result = _mm_min_ps(result, distance(index));
    }
    _mm_cvtss_f32(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_preserves_indefinite_results_and_integer_boundaries() {
        let reference = |value: f32| {
            if value >= -2147483648.0 && value < 2147483648.0 {
                value.trunc() as i32
            } else {
                i32::MIN
            }
        };
        let boundaries = [
            0, 0x80000000, 1, 0x80000001, 0x3f7fffff, 0xbf7fffff, 0x3f800001, 0xbf800001,
            0x4effffff, 0x4f000000, 0xcf000000, 0xcf000001, 0x7f800000, 0xff800000, 0x7fc00001,
            0x7f800001,
        ];
        let mut seed = 0x243f6a88u32;
        for iteration in 0..32768 {
            let values = std::array::from_fn(|lane| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                f32::from_bits(if iteration < boundaries.len() {
                    boundaries[(iteration + lane) % boundaries.len()]
                } else {
                    seed
                })
            });
            let expected = values.map(reference);
            assert_eq!(values.map(truncate_i32), expected);
            assert_eq!(truncate_i32x4(values), expected);
        }
    }

    #[test]
    fn packed_predicates_preserve_scalar_nan_zero_and_plane_order() {
        let mut seed = 0x6a09e667u32;
        let mut next = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            match seed & 31 {
                0 => -0.,
                1 => f32::NAN,
                2 => f32::INFINITY,
                3 => f32::NEG_INFINITY,
                _ => (seed as i32 as f32) * 0.000001,
            }
        };
        for iteration in 0..4096 {
            let input = std::array::from_fn(|_| std::array::from_fn(|_| next()));
            let planes: [[f32; 4]; 8] = std::array::from_fn(|_| std::array::from_fn(|_| next()));
            let active: Vec<_> = (0..iteration % 9)
                .map(|i| (i * 3 + iteration) % 8)
                .collect();
            for maximum in [false, true] {
                let start = if maximum { 0. } else { 1. };
                let reference = super::super::query_plane_minimum_scalar(
                    &planes, input, &active, maximum, start,
                );
                let actual = unsafe { plane_minimum(&planes, input, &active, maximum, start) };
                assert!(
                    actual.to_bits() == reference.to_bits()
                        || (actual.is_nan() && reference.is_nan()),
                    "plane reduction {iteration} maximum={maximum}: {actual:?} != {reference:?}"
                );
            }
            let point = std::array::from_fn(|_| next());
            let scale = std::array::from_fn(|_| next());
            assert_eq!(
                object_distance_rejection_mask(input, point, scale),
                super::super::object_distance_rejection_mask_scalar(input, point, scale),
                "distance reduction {iteration}"
            );
        }
    }
}
