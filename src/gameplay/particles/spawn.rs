//! Shared initial-velocity rotation and cone sampling (1f86c0/1dcc70/1dcf60).
use super::{orientation::normalize, parameters::VectorParameter, space::ParticleSpace};
use crate::{
    float::{add, multiply, subtract},
    gameplay::reference_frame::{axis_angle_basis, basis_product},
};

pub struct SpawnOrientation {
    pub angles: VectorParameter,
    pub worldspace: bool,
    pub order: usize,
}
impl Default for SpawnOrientation {
    /// 1f8940 initializes zero angles, local axes and order0.
    fn default() -> Self {
        Self {
            angles: VectorParameter::constant([0.; 3]),
            worldspace: false,
            order: 0,
        }
    }
}
impl SpawnOrientation {
    pub fn sample(
        &self,
        age: f32,
        space: &ParticleSpace,
        seed: &mut u32,
    ) -> Result<[[f32; 3]; 3], String> {
        let angles = self.angles.sample(age, seed)?;
        let basis = space.basis(self.worldspace);
        // The source stores the six overlapping triples in a thirteen-int table.
        let order = [
            [0, 1, 2],
            [2, 0, 1],
            [1, 2, 0],
            [0, 2, 1],
            [1, 0, 2],
            [2, 1, 0],
        ]
        .get(self.order)
        .copied()
        .ok_or("particle rotation order exceeds six modes")?;
        let rotation = |axis: usize| {
            axis_angle_basis(
                basis[axis],
                multiply(angles[axis], f32::from_bits(0xbc8e_fa35)),
            )
        };
        Ok(basis_product(
            basis_product(rotation(order[0]), rotation(order[1])),
            rotation(order[2]),
        ))
    }
}

fn cross(first: [f32; 3], second: [f32; 3]) -> [f32; 3] {
    [
        subtract(multiply(first[1], second[2]), multiply(first[2], second[1])),
        subtract(multiply(first[2], second[0]), multiply(first[0], second[2])),
        subtract(multiply(first[0], second[1]), multiply(first[1], second[0])),
    ]
}

/// 1dcc70 tries up to four random unit vectors to avoid a nearly parallel
/// direction. Its returned cross product is deliberately not normalized.
pub fn perpendicular(direction: [f32; 3], seed: &mut u32) -> [f32; 3] {
    let mut random = [0.; 3];
    for _ in 0..4 {
        random = std::array::from_fn(|_| subtract(multiply(crate::random::lcg_unit(seed), 2.), 1.));
        normalize(&mut random);
        let dot = add(
            add(
                multiply(random[1], direction[1]),
                multiply(random[0], direction[0]),
            ),
            multiply(random[2], direction[2]),
        );
        if dot.abs() < f32::from_bits(0x3f66_6666) {
            break;
        }
    }
    cross(direction, random)
}

/// 111590/0fb7b0's origin-zero vector rotation uses degrees and an already
/// prepared axis. It retains axis length instead of silently normalizing it.
pub fn rotate(value: [f32; 3], axis: [f32; 3], degrees: f32) -> [f32; 3] {
    let basis = axis_angle_basis(axis, multiply(f32::from_bits(0x3c8e_fa35), degrees));
    std::array::from_fn(|i| {
        let pair = if i == 0 {
            add(
                multiply(value[1], basis[1][i]),
                multiply(value[0], basis[0][i]),
            )
        } else {
            add(
                multiply(value[0], basis[0][i]),
                multiply(value[1], basis[1][i]),
            )
        };
        add(pair, multiply(value[2], basis[2][i]))
    })
}

/// 1dcf60 samples an azimuth even for a zero-width cone when velocity has
/// nonzero length. Those random draws affect the subsequent sprite state.
pub fn spray(velocity: [f32; 3], half_angle: f32, seed: &mut u32) -> [f32; 3] {
    let mut direction = velocity;
    if normalize(&mut direction) < f32::from_bits(0x3400_0000) {
        return velocity;
    }
    let side = perpendicular(direction, seed);
    let tangent = cross(side, direction);
    let tangent = rotate(
        tangent,
        direction,
        multiply(crate::random::lcg_unit(seed), 360.),
    );
    rotate(
        velocity,
        tangent,
        f32::from_bits(half_angle.to_bits() ^ 0x8000_0000),
    )
}
