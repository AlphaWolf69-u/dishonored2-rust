//! Scalar lanes of the stream contact solver (140c54d90).
//!
//! These are the ordinary rigid contact rows, not particle collision response.
//! Their body velocities and inverse masses are borrowed from the world's
//! existing solver motion array. A manifold owner retains the row/state packet.
use super::ball_socket::SolverVelocities;
pub mod build;
pub mod correspondence;
pub(crate) mod material;
pub mod restitution;
pub use build::{build, ContactBuild, ContactCache, ContactPoint};

#[derive(Clone, Copy, Debug, Default)]
pub struct NormalRow {
    pub angular_a: [f32; 3],
    pub angular_b: [f32; 3],
    pub effective_mass: f32,
    pub rhs: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct ImpulseLimit {
    pub maximum: f32,
    /// Stream header +1c, decoded with 1423ccdc0 (1/255).
    pub clipped_fraction: u8,
}

#[derive(Clone, Copy, Debug)]
pub enum FrictionLimit {
    /// Header friction type4 uses this iteration's accepted normal impulses.
    NormalImpulse(f32),
    /// Type3 carries the builder's converted previous normal force.
    Fixed(f32),
}

#[derive(Clone, Copy, Debug)]
pub struct FrictionBlock {
    pub tangent: [[f32; 3]; 2],
    pub angular_a: [[f32; 3]; 2],
    pub angular_b: [[f32; 3]; 2],
    pub rhs: [f32; 2],
    /// Symmetric inverse tangent matrix: diagonal0, diagonal1, off-diagonal.
    pub effective_mass: [f32; 3],
    pub spin_a: [f32; 3],
    pub spin_b: [f32; 3],
    pub spin_rhs: f32,
    pub spin_effective_mass: f32,
    pub radius: f32,
    pub limit: FrictionLimit,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ContactState {
    pub normal_impulses: [f32; 4],
    /// Actual stream result word0, used by the next frame's friction builder.
    pub friction_factor: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SolveResult {
    pub normal_delta: f32,
    pub normal_unclipped_delta: f32,
    pub clipped: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ContactRows {
    pub normal: [f32; 3],
    pub rows: [NormalRow; 4],
    pub count: u8,
    /// Stream header bit4 at +4. Ordinary rows permit undoing old impulses.
    pub retract_normal_impulses: bool,
    pub impulse_limit: Option<ImpulseLimit>,
    pub friction: Option<FrictionBlock>,
}

impl ContactRows {
    /// The source visits rows in manifold order and updates the same velocities
    /// immediately, before evaluating the following row or friction block.
    pub fn solve(
        &self,
        state: &mut ContactState,
        a: &mut SolverVelocities,
        b: &mut SolverVelocities,
        final_iteration: bool,
    ) -> SolveResult {
        let mut result = SolveResult::default();
        for index in 0..usize::from(self.count).min(4) {
            let row = self.rows[index];
            let velocity: [f32; 3] = std::array::from_fn(|i| {
                row.angular_a[i] * a.angular[i]
                    + b.angular[i] * row.angular_b[i]
                    + (a.linear[i] - b.linear[i]) * self.normal[i]
            });
            let residual = row.rhs - ((velocity[1] + velocity[0]) + velocity[2]);
            let previous = state.normal_impulses[index];
            if !(0.0 < max(previous, residual)) {
                continue;
            }
            let floor = if self.retract_normal_impulses {
                -previous
            } else {
                0.0
            };
            let unclipped = max(floor, row.effective_mass * residual);
            result.normal_unclipped_delta += unclipped;
            let delta = match self.impulse_limit {
                Some(limit) if limit.maximum < unclipped => {
                    result.clipped = true;
                    f32::from(limit.clipped_fraction) * f32::from_bits(0x3b80_8081) * limit.maximum
                }
                _ => unclipped,
            };
            result.normal_delta += delta;
            state.normal_impulses[index] = delta + previous;
            let linear_a = delta * a.inverse_mass[3];
            let linear_b = delta * b.inverse_mass[3];
            for i in 0..3 {
                a.linear[i] += linear_a * self.normal[i];
                b.linear[i] -= linear_b * self.normal[i];
                a.angular[i] += row.angular_a[i] * a.inverse_mass[i] * delta;
                b.angular[i] += row.angular_b[i] * b.inverse_mass[i] * delta;
            }
        }
        if let Some(friction) = self.friction {
            friction.solve(state, a, b, result.normal_delta, final_iteration);
        }
        result
    }
}

impl FrictionBlock {
    fn solve(
        self,
        state: &mut ContactState,
        a: &mut SolverVelocities,
        b: &mut SolverVelocities,
        normal_delta: f32,
        final_iteration: bool,
    ) {
        let residual: [f32; 2] = std::array::from_fn(|axis| {
            let velocity: [f32; 3] = std::array::from_fn(|i| {
                self.angular_a[axis][i] * a.angular[i]
                    + b.angular[i] * self.angular_b[axis][i]
                    + self.tangent[axis][i] * (a.linear[i] - b.linear[i])
            });
            self.rhs[axis] - ((velocity[1] + velocity[0]) + velocity[2])
        });
        let spin_velocity: [f32; 3] =
            std::array::from_fn(|i| self.spin_a[i] * a.angular[i] + b.angular[i] * self.spin_b[i]);
        let spin_residual =
            self.spin_rhs - ((spin_velocity[1] + spin_velocity[0]) + spin_velocity[2]);
        let mut tangent = [
            self.effective_mass[0] * residual[0] + self.effective_mass[2] * residual[1],
            self.effective_mass[1] * residual[1] + self.effective_mass[2] * residual[0],
        ];
        let mut spin = self.spin_effective_mass * spin_residual;
        let limit = match self.limit {
            FrictionLimit::NormalImpulse(coefficient) => coefficient * normal_delta,
            FrictionLimit::Fixed(value) => value,
        };
        let tangent_squared = tangent[1] * tangent[1] + tangent[0] * tangent[0];
        if spin * spin + tangent_squared <= limit * limit {
            state.friction_factor = 1.0;
        } else {
            if limit * limit == 0.0 {
                state.friction_factor = 0.0;
                return;
            }
            // c55703..c557d1 / c57d00: an anisotropic tangent/spin partition.
            // These particular reciprocals/square roots have NO Newton step.
            let spin_squared = ((self.radius * spin_residual) * self.radius) * spin_residual;
            let ratio = if spin_squared == 0.0 {
                f32::from_bits(0x5f7f_fff0)
            } else {
                reciprocal_estimate(spin_squared)
                    * ((residual[1] * residual[1] + residual[0] * residual[0]) * 0.5)
            };
            let partition = max(min(ratio, 1.0), f32::from_bits(0x3d88_8889));
            let spin_partition = max(1.0 - partition, f32::from_bits(0x3d88_8889));
            let tangent_scale = min(
                1.0,
                partition * limit * inverse_length_estimate(tangent_squared),
            );
            let spin_scale = min(
                1.0,
                if spin.abs() == 0.0 {
                    f32::from_bits(0x5f7f_fff0)
                } else {
                    reciprocal_estimate(spin.abs()) * (spin_partition * limit)
                },
            );
            tangent[0] *= tangent_scale;
            tangent[1] *= tangent_scale;
            spin *= spin_scale;
            if final_iteration {
                state.friction_factor = min(
                    min(tangent_scale, spin_scale) * 15.0,
                    max(tangent_scale, spin_scale),
                );
            }
        }
        for i in 0..3 {
            b.linear[i] = (b.linear[i] - tangent[0] * b.inverse_mass[3] * self.tangent[0][i])
                - tangent[1] * b.inverse_mass[3] * self.tangent[1][i];
            a.linear[i] = a.linear[i]
                + a.inverse_mass[3] * self.tangent[0][i] * tangent[0]
                + a.inverse_mass[3] * self.tangent[1][i] * tangent[1];
            b.angular[i] = b.angular[i]
                + tangent[0] * self.angular_b[0][i] * b.inverse_mass[i]
                + tangent[1] * self.angular_b[1][i] * b.inverse_mass[i]
                + spin * self.radius * b.inverse_mass[i] * self.spin_b[i];
            a.angular[i] = a.angular[i]
                + tangent[0] * a.inverse_mass[i] * self.angular_a[0][i]
                + tangent[1] * a.inverse_mass[i] * self.angular_a[1][i]
                + a.inverse_mass[i] * self.spin_a[i] * spin * self.radius;
        }
    }
}

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

fn reciprocal_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
        _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(value)))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1.0 / value
    }
}
fn inverse_length_estimate(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(value)))
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1.0 / value.sqrt()
    }
}
