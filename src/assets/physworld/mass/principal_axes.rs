//! Jacobi mass-frame diagonalization and the source's axis reconstruction.
use crate::float::{divide, subtract};
use crate::gameplay::{
    constraint_solver::inverse_length, shape_transform::rotation::compose_basis,
};

const IDENTITY: [[f32; 4]; 3] = [[1., 0., 0., 0.], [0., 1., 0., 0.], [0., 0., 1., 0.]];
fn product(left: [[f32; 4]; 3], right: [[f32; 4]; 3]) -> [[f32; 4]; 3] {
    let mut matrix = [left[0], left[1], left[2], [0.; 4]];
    compose_basis(&mut matrix, right);
    [matrix[0], matrix[1], matrix[2]]
}

/// ae08a0. Largest off-diagonal selection retains its original tie order;
/// convergence tolerance is relative to the initial squared matrix norm.
pub fn diagonalize_inertia(
    mut tensor: [[f32; 4]; 3],
    iterations: i32,
) -> ([[f32; 4]; 3], [f32; 4]) {
    let mut axes = IDENTITY;
    let norm_columns = tensor.map(|v| (v[1] * v[1] + v[0] * v[0]) + v[2] * v[2]);
    let norm = (norm_columns[0] + norm_columns[1]) + norm_columns[2];
    let threshold = norm * f32::from_bits(0x2880_0000);
    for _ in 0..iterations {
        let mut p = 0;
        let mut q = 1;
        let mut maximum = tensor[1][0].abs();
        if maximum < tensor[2][0].abs() {
            q = 2;
            maximum = tensor[2][0].abs();
        }
        if maximum < tensor[2][1].abs() {
            p = 1;
            q = 2;
            maximum = tensor[2][1].abs();
        }
        // COMISS/JC also exits on an unordered comparison.
        if !(maximum * maximum >= threshold) {
            break;
        }
        let tau = divide(subtract(tensor[q][q], tensor[p][p]), tensor[q][p] * 2.);
        let root = (tau * tau + 1.).sqrt();
        let denominator = if !(tau >= 0.) {
            subtract(tau, root)
        } else {
            root + tau
        };
        let t = divide(1., denominator);
        let cosine = divide(1., (t * t + 1.).sqrt());
        let sine = cosine * t;
        let mut rotation = IDENTITY;
        let mut transpose = IDENTITY;
        rotation[p][p] = cosine;
        rotation[q][q] = cosine;
        rotation[q][p] = sine;
        rotation[p][q] = f32::from_bits(sine.to_bits() ^ 0x8000_0000);
        transpose[p][p] = cosine;
        transpose[q][q] = cosine;
        transpose[p][q] = sine;
        transpose[q][p] = f32::from_bits(sine.to_bits() ^ 0x8000_0000);
        tensor = product(tensor, rotation);
        tensor = product(transpose, tensor);
        axes = product(axes, rotation);
    }
    (
        axes,
        [tensor[0][0], tensor[1][1], tensor[2][2], tensor[2][2]],
    )
}
fn normalize(axis: [f32; 4]) -> [f32; 4] {
    let squared = (axis[1] * axis[1] + axis[0] * axis[0]) + axis[2] * axis[2];
    let inverse = inverse_length(squared);
    axis.map(|v| inverse * v)
}
fn cross(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        subtract(b[2] * a[1], a[2] * b[1]),
        subtract(b[0] * a[2], a[0] * b[2]),
        subtract(b[1] * a[0], a[1] * b[0]),
        subtract(b[3] * a[3], a[3] * b[3]),
    ]
}

/// c2c040 always uses ten iterations, clamps principal values, and rebuilds
/// the last two axes by normalized cross products. No sorting or fallback axes.
pub fn principal_inertia_axes(tensor: &mut [[f32; 4]; 3]) -> [[f32; 4]; 3] {
    let (mut axes, values) = diagonalize_inertia(*tensor, 10);
    let minimum = f32::from_bits(0x3400_0000);
    let values = values.map(|v| if v > minimum { v } else { minimum });
    *tensor = std::array::from_fn(|column| {
        std::array::from_fn(|row| values[row] * IDENTITY[column][row])
    });
    axes[0] = normalize(axes[0]);
    axes[1] = normalize(cross(axes[2], axes[0]));
    axes[2] = normalize(cross(axes[0], axes[1]));
    axes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::physworld::mass::MassProperties;
    #[test]
    fn principal_axes_match_executed_routines() {
        let mut count = 0;
        for (case, line) in include_str!("../../../../tests/fixtures/principal_axes.txt")
            .lines()
            .enumerate()
        {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let properties = MassProperties {
                volume: f32::from_bits(w[0]),
                mass: f32::from_bits(w[1]),
                center: vector(2),
                inertia: std::array::from_fn(|i| vector(6 + i * 4)),
            };
            let (axes, values) = diagonalize_inertia(properties.inertia, w[18] as i32);
            let actual: Vec<u32> = axes
                .into_iter()
                .flatten()
                .chain(values)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, w[19..35], "diagonalization case {case}");
            let mut tensor = properties.inertia;
            let axes = principal_inertia_axes(&mut tensor);
            let actual: Vec<u32> = tensor
                .into_iter()
                .flatten()
                .chain(axes.into_iter().flatten())
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, w[35..59], "principal axes case {case}");
            let p = properties.to_principal();
            let actual: Vec<u32> = [p.volume, p.mass]
                .into_iter()
                .chain(p.center)
                .chain(p.inertia)
                .chain(p.orientation)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, w[59..], "full conversion case {case}");
            count += 1;
        }
        assert_eq!(count, 1024);
    }
}
