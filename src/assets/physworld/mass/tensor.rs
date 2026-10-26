//! Inertia tensor frame changes used by compound mass recomputation.
use crate::float::{add, multiply};

/// ae0460: tensor * transposed rotation (ae1ba0), then rotation * intermediate
/// (ae17d0). The two kernels use different addition orders. Preserve W lanes.
pub fn rotate_inertia(tensor: &mut [[f32; 4]; 3], rotation: [[f32; 4]; 3]) {
    let intermediate: [[f32; 4]; 3] = std::array::from_fn(|column| {
        std::array::from_fn(|row| {
            add(
                add(
                    multiply(rotation[1][column], tensor[1][row]),
                    multiply(rotation[0][column], tensor[0][row]),
                ),
                multiply(rotation[2][column], tensor[2][row]),
            )
        })
    });
    *tensor = std::array::from_fn(|column| {
        std::array::from_fn(|row| {
            add(
                add(
                    multiply(intermediate[column][0], rotation[0][row]),
                    multiply(intermediate[column][1], rotation[1][row]),
                ),
                multiply(intermediate[column][2], rotation[2][row]),
            )
        })
    });
}

/// c2c1e0 subtracts a mass-weighted parallel-axis tensor from existing inertia.
/// Keep the four source mass lanes; callers normally broadcast a scalar.
pub fn subtract_parallel_axis(tensor: &mut [[f32; 4]; 3], offset: [f32; 4], mass: [f32; 4]) {
    let [x, y, z, _] = offset;
    let yy = multiply(y, y);
    let xy = multiply(multiply(y, x), mass[0]);
    let yz = multiply(multiply(z, y), mass[1]);
    let xz = multiply(multiply(x, z), mass[2]);
    let xx = multiply(x, x);
    let zz = multiply(z, z);
    let diagonal = [
        multiply(add(zz, yy), 0. - mass[0]),
        multiply(add(zz, xx), 0. - mass[1]),
        multiply(add(yy, xx), 0. - mass[2]),
    ];
    let terms = [
        [diagonal[0], xy, xz, 0.],
        [xy, diagonal[1], yz, 0.],
        [xz, yz, diagonal[2], 0.],
    ];
    for column in 0..3 {
        for row in 0..4 {
            tensor[column][row] = add(terms[column][row], tensor[column][row]);
        }
    }
}

/// c2c280 negates the mass through subtraction from zero and delegates to
/// c2c1e0, adding the parallel-axis contribution without clearing the tensor.
pub fn add_parallel_axis(tensor: &mut [[f32; 4]; 3], offset: [f32; 4], mass: [f32; 4]) {
    subtract_parallel_axis(tensor, offset, mass.map(|v| 0. - v));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inertia_operations_match_executed_routines() {
        let mut count = 0;
        for (case, line) in include_str!("../../../../tests/fixtures/inertia_operations.txt")
            .lines()
            .enumerate()
        {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let initial = std::array::from_fn(|i| vector(i * 4));
            let rotation = std::array::from_fn(|i| vector(12 + i * 4));
            let mut result = initial;
            rotate_inertia(&mut result, rotation);
            assert_eq!(
                result
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect::<Vec<_>>(),
                w[32..44],
                "rotation case {case}"
            );
            let mut result = initial;
            subtract_parallel_axis(&mut result, vector(24), vector(28));
            assert_eq!(
                result
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect::<Vec<_>>(),
                w[44..56],
                "subtract case {case}"
            );
            let mut result = initial;
            add_parallel_axis(&mut result, vector(24), vector(28));
            assert_eq!(
                result
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits)
                    .collect::<Vec<_>>(),
                w[56..],
                "add case {case}"
            );
            count += 1;
        }
        assert_eq!(count, 1024);
    }
}
