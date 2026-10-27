//! CPU vector-field preparation and ordered particle sampling (214d20..216310).
use super::space::ParticleSpace;
use crate::{
    assets::fx::particle_field::{FieldTexture, ParticleField},
    float::{add, divide, multiply, subtract},
    gameplay::reference_frame::{basis_product, invert_basis, ReferenceFrame},
};

impl FieldTexture {
    /// 214ab0 reads two bytes at truncated coordinates, without interpolation.
    pub fn sample(&self, uv: [f32; 2]) -> Result<[f32; 2], String> {
        let Some(_) = self.source else {
            return Ok([0.; 2]);
        };
        let pixels = self
            .pixels
            .as_ref()
            .ok_or("particle field image is not loaded")?;
        let dimensions = [pixels.width, pixels.height];
        let xy: [i32; 2] = std::array::from_fn(|i| {
            let coordinate = crate::float::truncate_i32(multiply(dimensions[i] as f32, uv[i]));
            if self.tiling {
                coordinate.rem_euclid(dimensions[i])
            } else {
                coordinate.clamp(0, dimensions[i] - 1)
            }
        });
        let index = (xy[1] as usize * pixels.width as usize + xy[0] as usize) * 2;
        let mut value = std::array::from_fn(|i| {
            add(
                self.low[i],
                multiply(
                    pixels.rg[index + i] as f32,
                    multiply(
                        subtract(self.high[i], self.low[i]),
                        f32::from_bits(0x3b80_8081),
                    ),
                ),
            )
        });
        if self.fading {
            let epsilon = f32::from_bits(0x3727_c5ac);
            for i in 0..2 {
                // 214bf0 prepares both reciprocals with the same 1e-5 term.
                let high_scale = divide(1., add(subtract(1., self.fade_center[i]), epsilon));
                let low_scale = divide(1., add(self.fade_center[i], epsilon));
                let high = multiply(subtract(uv[i], self.fade_center[i]), high_scale)
                    .clamp(0., 1.)
                    .powf(self.fade_power[i]);
                let low = multiply(subtract(self.fade_center[i], uv[i]), low_scale)
                    .clamp(0., 1.)
                    .powf(self.fade_power[i]);
                value[i] = multiply(
                    subtract(1., high),
                    crate::gameplay::effects::normalize_small(value[i]),
                );
                value[i] = multiply(
                    subtract(1., low),
                    crate::gameplay::effects::normalize_small(value[i]),
                );
            }
        }
        Ok(value)
    }
}

pub struct FieldState {
    rotation: [f32; 3],
    seed: u32,
    inverse: [[f32; 3]; 3],
    offset: [f32; 3],
    output: [[f32; 3]; 3],
    constant: [f32; 3],
    unbounded: bool,
}
impl FieldState {
    pub fn new(definition: &ParticleField, seed: &mut u32) -> Self {
        let mix = std::array::from_fn(|_| crate::random::lcg_unit(seed));
        Self {
            rotation: crate::gameplay::effects::parameters::position_range(
                definition.initial_rotation,
                mix,
            ),
            seed: crate::random::lcg_step(seed),
            inverse: ReferenceFrame::default().basis,
            offset: [0.; 3],
            output: ReferenceFrame::default().basis,
            constant: [0.; 3],
            unbounded: definition.textures.iter().any(|texture| texture.tiling),
        }
    }
    pub fn reset(&mut self, definition: &ParticleField, seed: &mut u32) {
        if definition.cycle_reset {
            let mix = std::array::from_fn(|_| crate::random::lcg_unit(seed));
            self.rotation = crate::gameplay::effects::parameters::position_range(
                definition.initial_rotation,
                mix,
            );
        }
    }
    pub fn prepare(
        &mut self,
        definition: &ParticleField,
        elapsed: f32,
        fraction: f32,
        delta: f32,
        origin: [f32; 3],
        space: &ParticleSpace,
        cycle_seed: &mut u32,
    ) -> Result<(), String> {
        let mut seed = self.seed;
        let rate = definition.rotation_rate.sample(elapsed, &mut seed)?;
        let cycle_rate = definition
            .rotation_rate_cycle
            .sample(fraction, cycle_seed)?;
        self.rotation = std::array::from_fn(|i| {
            add(
                multiply(add(rate[i], cycle_rate[i]), delta),
                self.rotation[i],
            )
        });
        let basis = space.basis(definition.worldspace);
        let rotation = basis_product(
            euler_basis(self.rotation, definition.rotation_order)?,
            basis,
        );
        let constant = definition.constant_strength.sample(elapsed, &mut seed)?;
        let constant_cycle = definition
            .constant_strength_cycle
            .sample(fraction, cycle_seed)?;
        self.constant = transform(
            std::array::from_fn(|i| multiply(constant[i], constant_cycle[i])),
            rotation,
        );
        let scale = definition.scale.sample(elapsed, &mut seed)?;
        let scale_cycle = definition.scale_cycle.sample(fraction, cycle_seed)?;
        let scaled = std::array::from_fn(|i| {
            rotation[i].map(|v| multiply(multiply(scale[i], scale_cycle[i]), v))
        });
        let position = definition.position.sample(elapsed, &mut seed)?;
        let position_cycle = definition.position_cycle.sample(fraction, cycle_seed)?;
        let translation = transform(
            std::array::from_fn(|i| add(position[i], position_cycle[i])),
            basis,
        );
        let position = std::array::from_fn(|i| add(translation[i], origin[i]));
        self.inverse = scaled;
        invert_basis(&mut self.inverse);
        let translation = transform(position, self.inverse);
        self.offset = translation.map(|v| add(-v, 0.5));
        let strength = definition.strength.sample(elapsed, &mut seed)?;
        let strength_cycle = definition.strength_cycle.sample(fraction, cycle_seed)?;
        self.output = std::array::from_fn(|i| {
            rotation[i].map(|v| multiply(multiply(strength[i], strength_cycle[i]), v))
        });
        Ok(())
    }
    pub fn sample(
        &self,
        definition: &ParticleField,
        position: [f32; 3],
        age: f32,
        seed: &mut u32,
    ) -> Result<Option<[f32; 3]>, String> {
        if !definition.enabled {
            return Ok(None);
        }
        let point = transform(position, self.inverse);
        let point: [f32; 3] = std::array::from_fn(|i| add(point[i], self.offset[i]));
        if !self.unbounded && !point.iter().all(|&v| 0. <= v && v <= 1.) {
            return Ok(None);
        }
        let xy = definition.textures[0].sample([point[0], point[1]])?;
        let xz = definition.textures[1].sample([point[0], point[2]])?;
        let yz = definition.textures[2].sample([point[1], point[2]])?;
        let value = [
            add(add(0., xy[0]), xz[0]),
            add(add(0., xy[1]), yz[0]),
            add(add(0., xz[1]), yz[1]),
        ];
        let value = transform(value, self.output);
        let strength = definition.strength_life.sample(age, seed)?;
        let mut value = std::array::from_fn(|i| multiply(value[i], strength[i]));
        let constant = definition.constant_strength_life.sample(age, seed)?;
        for i in 0..3 {
            value[i] = add(value[i], multiply(constant[i], self.constant[i]));
            value[i] = match definition.modes[i] {
                1 => value[i].abs(),
                2 => -value[i].abs(),
                _ => value[i],
            };
        }
        Ok(Some(value))
    }
}

pub fn sample(
    states: &[FieldState],
    definitions: &[ParticleField],
    position: [f32; 3],
    age: f32,
    seed: &mut u32,
) -> Result<Option<[f32; 3]>, String> {
    let mut result = None;
    for (state, definition) in states.iter().zip(definitions) {
        if let Some(value) = state.sample(definition, position, age, seed)? {
            let previous = result.get_or_insert([0.; 3]);
            *previous = std::array::from_fn(|i| add(value[i], previous[i]));
        }
    }
    Ok(result)
}

fn transform(value: [f32; 3], basis: [[f32; 3]; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        add(
            add(
                multiply(value[1], basis[1][i]),
                multiply(value[0], basis[0][i]),
            ),
            multiply(value[2], basis[2][i]),
        )
    })
}

/// 213d10 composes three coordinate rotations, using 10e6a0's direct sin/cos
/// matrices rather than the arbitrary-axis quaternion rotation helper.
fn euler_basis(degrees: [f32; 3], order: usize) -> Result<[[f32; 3]; 3], String> {
    let axes = [
        [0, 1, 2],
        [2, 0, 1],
        [1, 2, 0],
        [0, 2, 1],
        [1, 0, 2],
        [2, 1, 0],
    ];
    let axes = axes
        .get(order)
        .ok_or("field rotation order exceeds six modes")?;
    let mut result = ReferenceFrame::default().basis;
    for &axis in axes {
        let angle = multiply(degrees[axis], f32::from_bits(0x3c8e_fa35));
        let s = angle.sin();
        let c = angle.cos();
        let rotation = match axis {
            0 => [[1., 0., 0.], [0., c, s], [0., -s, c]],
            1 => [[c, 0., -s], [0., 1., 0.], [s, 0., c]],
            _ => [[c, s, 0.], [-s, c, 0.], [0., 0., 1.]],
        };
        result = basis_product(result, rotation);
    }
    Ok(result)
}
