//! 208bf0's paired world/local bases and 208d50's vector transform.
use crate::float::{add, multiply, subtract};

pub struct ParticleSpace {
    world_to_stage: [[f32; 3]; 3],
    local_to_stage: [[f32; 3]; 3],
}
impl ParticleSpace {
    /// Basis vectors are the emitter's three consecutive axis vectors. The
    /// local-stage path uses 10ee70's full inverse, including its unchanged
    /// matrix result when a small determinant rejects inversion.
    pub fn new(emitter_basis: [[f32; 3]; 3], stage_worldspace: bool) -> Self {
        let identity = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
        if stage_worldspace {
            Self {
                world_to_stage: identity,
                local_to_stage: emitter_basis,
            }
        } else {
            let mut inverse = emitter_basis;
            crate::gameplay::reference_frame::invert_basis(&mut inverse);
            Self {
                world_to_stage: inverse,
                local_to_stage: identity,
            }
        }
    }

    pub fn transform(&self, value: [f32; 3], worldspace: bool) -> [f32; 3] {
        let basis = if worldspace {
            &self.world_to_stage
        } else {
            &self.local_to_stage
        };
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

    /// 1dbce0 uses world Z and the installed 9.8 constant (141b05eb8), then
    /// multiplies by the separately sampled gravity parameter.
    pub fn gravity(&self, amount: f32) -> [f32; 3] {
        std::array::from_fn(|i| {
            multiply(
                subtract(
                    add(
                        multiply(self.world_to_stage[1][i], 0.),
                        multiply(self.world_to_stage[0][i], 0.),
                    ),
                    multiply(self.world_to_stage[2][i], f32::from_bits(0x411c_cccd)),
                ),
                amount,
            )
        })
    }
}
