//! Rotation construction used by penetration-query retries.
use super::ShapeMatrix;

/// a9fce0: packed range reduction and polynomial sine/cosine. This preserves
/// the physics library's rounding, quadrant masks and nonfinite handling.
#[cfg(target_arch = "x86_64")]
pub fn sin_cos(angles: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    use std::arch::x86_64::*;
    unsafe {
        let constant = |bits| _mm_set1_ps(f32::from_bits(bits));
        let sign = _mm_castsi128_ps(_mm_set1_epi32(i32::MIN));
        let input = _mm_loadu_ps(angles.as_ptr());
        let absolute = _mm_andnot_ps(sign, input);
        let signs = _mm_and_ps(input, sign);
        let one_i = _mm_set1_epi32(1);
        let quadrant = _mm_andnot_si128(
            one_i,
            _mm_add_epi32(
                _mm_cvttps_epi32(_mm_mul_ps(constant(0x3fa2_f983), absolute)),
                one_i,
            ),
        );
        let q = _mm_cvtepi32_ps(quadrant);
        let reduced = _mm_add_ps(
            _mm_add_ps(
                _mm_add_ps(_mm_mul_ps(constant(0xbf49_0000), q), absolute),
                _mm_mul_ps(constant(0xb97d_a000), q),
            ),
            _mm_mul_ps(constant(0xb322_2169), q),
        );
        let squared = _mm_mul_ps(reduced, reduced);
        let sine = _mm_add_ps(
            _mm_mul_ps(
                _mm_mul_ps(
                    _mm_add_ps(
                        _mm_mul_ps(
                            _mm_add_ps(
                                _mm_mul_ps(squared, constant(0xb94c_a1f9)),
                                constant(0x3c08_839e),
                            ),
                            squared,
                        ),
                        constant(0xbe2a_aaa3),
                    ),
                    squared,
                ),
                reduced,
            ),
            reduced,
        );
        let cosine = _mm_add_ps(
            _mm_sub_ps(
                _mm_mul_ps(
                    _mm_mul_ps(
                        _mm_add_ps(
                            _mm_mul_ps(
                                _mm_add_ps(
                                    _mm_mul_ps(squared, constant(0x37cc_f5ce)),
                                    constant(0xbab6_061a),
                                ),
                                squared,
                            ),
                            constant(0x3d2a_aaa5),
                        ),
                        squared,
                    ),
                    squared,
                ),
                _mm_mul_ps(squared, _mm_set1_ps(0.5)),
            ),
            _mm_set1_ps(1.),
        );
        let even = _mm_castsi128_ps(_mm_cmpeq_epi32(
            _mm_and_si128(quadrant, _mm_set1_epi32(2)),
            _mm_setzero_si128(),
        ));
        let selected_sine = _mm_and_ps(even, sine);
        let selected_cosine = _mm_andnot_ps(even, cosine);
        let other = _mm_add_ps(
            _mm_sub_ps(sine, selected_sine),
            _mm_sub_ps(cosine, selected_cosine),
        );
        let selected = _mm_add_ps(selected_cosine, selected_sine);
        let sine_sign = _mm_xor_ps(
            signs,
            _mm_castsi128_ps(_mm_slli_epi32::<29>(_mm_and_si128(
                quadrant,
                _mm_set1_epi32(4),
            ))),
        );
        let cosine_sign = _mm_castsi128_ps(_mm_slli_epi32::<29>(_mm_andnot_si128(
            _mm_sub_epi32(quadrant, _mm_set1_epi32(2)),
            _mm_set1_epi32(4),
        )));
        let mut result = ([0.; 4], [0.; 4]);
        _mm_storeu_ps(
            result.0.as_mut_ptr(),
            _mm_xor_ps(sine_sign, _mm_min_ps(selected, _mm_set1_ps(1.))),
        );
        _mm_storeu_ps(
            result.1.as_mut_ptr(),
            _mm_xor_ps(cosine_sign, _mm_min_ps(other, _mm_set1_ps(1.))),
        );
        result
    }
}
#[cfg(not(target_arch = "x86_64"))]
pub fn sin_cos(_angles: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    unimplemented!("physics angle reduction requires x86-64 SSE")
}

/// aea310/aea340: the authored axis is already normalized by the caller.
pub fn axis_angle_quaternion(axis: [f32; 4], angle: f32) -> [f32; 4] {
    let (sine, cosine) = sin_cos([angle * 0.5; 4]);
    [
        axis[0] * sine[0],
        axis[1] * sine[1],
        axis[2] * sine[2],
        cosine[3],
    ]
}

/// aa0590 reuses aa0390's verified quaternion-to-basis conversion.
pub fn axis_angle_basis(axis: [f32; 4], angle: f32) -> [[f32; 4]; 3] {
    crate::assets::physworld::quaternion_basis(axis_angle_quaternion(axis, angle))
}

/// ae1230/ae17d0: multiply the basis in place; translation is untouched.
pub fn compose_basis(matrix: &mut ShapeMatrix, basis: [[f32; 4]; 3]) {
    use crate::float::{add, multiply};
    let before = *matrix;
    for column in 0..3 {
        matrix[column] = std::array::from_fn(|i| {
            add(
                add(
                    multiply(basis[column][0], before[0][i]),
                    multiply(basis[column][1], before[1][i]),
                ),
                multiply(basis[column][2], before[2][i]),
            )
        });
    }
}

/// a9fe60's packed arcsine, including its small-angle and half-range branches.
/// The machine instructions define this polynomial; Ghidra's scalar return
/// type omits most of the vector computation.
#[cfg(target_arch = "x86_64")]
pub fn asin(values: [f32; 4]) -> [f32; 4] {
    use std::arch::x86_64::*;
    unsafe {
        let c = |bits| _mm_set1_ps(f32::from_bits(bits));
        let sign = _mm_castsi128_ps(_mm_set1_epi32(i32::MIN));
        let input = _mm_loadu_ps(values.as_ptr());
        let absolute = _mm_min_ps(_mm_set1_ps(1.), _mm_andnot_ps(sign, input));
        let small = _mm_cmplt_ps(absolute, c(0x38d1b717));
        let large = _mm_cmplt_ps(_mm_set1_ps(0.5), absolute);
        let half_distance = _mm_mul_ps(_mm_sub_ps(_mm_set1_ps(1.), absolute), _mm_set1_ps(0.5));
        let root = _mm_or_ps(
            _mm_and_ps(large, _mm_sqrt_ps(half_distance)),
            _mm_andnot_ps(large, absolute),
        );
        let square = _mm_or_ps(
            _mm_and_ps(large, half_distance),
            _mm_andnot_ps(large, _mm_mul_ps(absolute, absolute)),
        );
        let mut polynomial = c(0x3d2cb352);
        for coefficient in [0x3cc617e3, 0x3d3a3ec7, 0x3d9980f6, 0x3e2aaae4] {
            polynomial = _mm_add_ps(_mm_mul_ps(polynomial, square), c(coefficient));
        }
        polynomial = _mm_add_ps(_mm_mul_ps(_mm_mul_ps(polynomial, square), root), root);
        let complement = _mm_sub_ps(c(0x3fc90fdb), _mm_add_ps(polynomial, polynomial));
        let angle = _mm_or_ps(
            _mm_and_ps(large, complement),
            _mm_andnot_ps(large, polynomial),
        );
        let selected = _mm_or_ps(_mm_and_ps(small, absolute), _mm_andnot_ps(small, angle));
        let mut result = [0.; 4];
        _mm_storeu_ps(
            result.as_mut_ptr(),
            _mm_xor_ps(selected, _mm_and_ps(input, sign)),
        );
        result
    }
}
#[cfg(not(target_arch = "x86_64"))]
pub fn asin(_values: [f32; 4]) -> [f32; 4] {
    unimplemented!("physics arcsine requires x86-64 SSE")
}

#[cfg(test)]
#[path = "rotation_tests.rs"]
mod tests;
