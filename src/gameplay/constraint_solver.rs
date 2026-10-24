//! Constraint row construction and angular motor solver arithmetic.
use super::machinery::{ConstraintMotor, MotorStep};
pub mod angular;
pub mod ball_socket;
pub mod linear;
pub mod motion;
pub mod atom_stack;
pub mod step;

pub fn inverse_length(squared: f32) -> f32 {
    if squared <= 0. {
        return 0.;
    }
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        let estimate = unsafe { _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(squared))) };
        (3. - estimate * squared * estimate) * (estimate * 0.5)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / squared.sqrt()
    }
}

/// RCPPS followed by the game's single Newton refinement.
pub fn reciprocal(value: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rcp_ss, _mm_set_ss};
        let estimate = unsafe { _mm_cvtss_f32(_mm_rcp_ss(_mm_set_ss(value))) };
        (2. - estimate * value) * estimate
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        1. / value
    }
}

/// 140a9f2b0/140a9f380; the original packed polynomial rather than libm atan2.
pub fn hinge_angle(y: f32, x: f32) -> f32 {
    let half_pi = f32::from_bits(0x3fc9_0fdb);
    let sign = y.to_bits() & 0x8000_0000;
    let correction = if x == 0. {
        if y == 0. {
            0.
        } else {
            f32::from_bits(half_pi.to_bits() ^ sign)
        }
    } else {
        f32::from_bits(
            (if x < 0. {
                std::f32::consts::PI.to_bits()
            } else {
                0
            }) ^ sign,
        )
    };
    if x == 0. || y == 0. {
        return correction;
    }
    let ratio = y / x;
    let invert = ratio.abs() > 1.;
    let v = if invert { -reciprocal(ratio) } else { ratio };
    let square = v * v;
    let c = |bits| f32::from_bits(bits);
    let polynomial = square
        * v
        * (c(0x3b3b_d74a) * square + c(0xbbeb_7f9e))
        * ((c(0x3fda_2291) + square) * square + c(0x400f_22d2))
        * ((c(0xbf80_9ff7) + square) * square + c(0x4071_2d99))
        * ((c(0xc075_3eb3) + square) * square + c(0x40b0_19ca));
    correction + ((polynomial + if invert { half_pi.copysign(ratio) } else { 0. }) + v)
}

pub type Basis = [[f32; 4]; 3];

pub(super) fn schema_axis(value: f32, kind: u8) -> f32 {
    f32::from_bits((value + f32::from_bits(0x34000000)).to_bits() & !0xff | u32::from(kind))
}

#[derive(Clone, Copy, Debug)]
pub struct ConstraintBody {
    pub principal: Basis,
    pub center: [f32; 3],
    pub inverse_inertia: [f32; 3],
    pub inverse_mass: f32,
}

pub(super) fn project(basis: Basis, axis: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (axis[1] * basis[1][i] + axis[0] * basis[0][i]) + axis[2] * basis[2][i])
}

fn angular_inverse_mass(
    a: ConstraintBody,
    b: ConstraintBody,
    axis_a: [f32; 3],
    axis_b: [f32; 3],
) -> [f32; 3] {
    std::array::from_fn(|i| {
        axis_b[i] * axis_b[i] * b.inverse_inertia[i] + axis_a[i] * axis_a[i] * a.inverse_inertia[i]
    })
}

#[derive(Clone, Copy, Debug)]
pub struct AngularLockRow {
    pub axis_a: [f32; 3],
    pub axis_b: [f32; 3],
    pub effective_mass: f32,
    pub correction: f32,
}

/// Complete 140d3d720; the two perpendicular rows in a hinge use this builder.
pub fn angular_lock_row(
    a: ConstraintBody,
    b: ConstraintBody,
    reference: [f32; 3],
    axis: [f32; 3],
    perpendicular: [f32; 3],
    inverse_step_tau: f32,
    damping: f32,
) -> AngularLockRow {
    let mut axis_a = project(a.principal, axis);
    let axis_b = project(b.principal, axis.map(|v| -v));
    let inverse = angular_inverse_mass(a, b, axis_a, axis_b);
    let effective_mass =
        damping * reciprocal((inverse[1] + inverse[0]) + inverse[2] + f32::from_bits(0x34000000));
    let error = (perpendicular[1] * reference[1] + perpendicular[0] * reference[0])
        + perpendicular[2] * reference[2];
    axis_a[0] = schema_axis(axis_a[0], 12);
    AngularLockRow {
        axis_a,
        axis_b,
        effective_mass,
        correction: (0. - error) * inverse_step_tau,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LinearLockRow {
    pub axis: [f32; 3],
    pub angular_a: [f32; 3],
    pub angular_b: [f32; 3],
    pub effective_mass: f32,
    pub correction: f32,
    pub inverse_effective_mass: f32,
}

/// Complete 140d3d070; a bilateral linear axis through the body's constraint pivot.
pub fn linear_lock_row(
    a: ConstraintBody,
    b: ConstraintBody,
    pivot_a: [f32; 3],
    pivot_b: [f32; 3],
    axis: [f32; 3],
    inverse_step_tau: f32,
    damping: f32,
) -> LinearLockRow {
    let ra: [f32; 3] = std::array::from_fn(|i| pivot_a[i] - a.center[i]);
    let rb: [f32; 3] = std::array::from_fn(|i| pivot_a[i] - b.center[i]);
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let angular_a = project(a.principal, cross(ra, axis));
    let angular_b = project(b.principal, cross(axis, rb));
    let inverse = angular_inverse_mass(a, b, angular_a, angular_b);
    let mass = (b.inverse_mass + a.inverse_mass).max(f32::from_bits(0x28800000));
    let denominator = (inverse[0] + inverse[2]) + (inverse[1] + mass);
    let effective_mass = damping * reciprocal(denominator);
    let gap: [f32; 3] = std::array::from_fn(|i| pivot_b[i] - pivot_a[i]);
    let correction = ((gap[1] * axis[1] + gap[0] * axis[0]) + gap[2] * axis[2]) * inverse_step_tau;
    let mut packed_axis = axis;
    packed_axis[0] = schema_axis(axis[0], 5);
    LinearLockRow {
        axis: packed_axis,
        angular_a,
        angular_b,
        effective_mass,
        correction,
        inverse_effective_mass: denominator,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AngularMotorRow {
    pub axis_a: [f32; 3],
    pub axis_b: [f32; 3],
    pub effective_mass: f32,
    pub initial_impulse: f32,
    pub maximum_impulse: f32,
    pub minimum_impulse: f32,
    pub target_velocity: f32,
    pub tau: f32,
    pub damping: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct HingeHistory {
    pub initialized: bool,
    pub previous_angle: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct AngularVelocityAccumulator {
    pub current: [f32; 3],
    pub initial: [f32; 3],
    pub inverse_inertia: [f32; 3],
}

impl AngularMotorRow {
    /// Schema 15 in 14129af90. The solver keeps its initial velocity separate
    /// from accumulated corrections; dropping that term changes motor speeds.
    /// Returns the applied impulse and the row's saturation scale.
    pub fn solve(
        self,
        a: &mut AngularVelocityAccumulator,
        b: &mut AngularVelocityAccumulator,
        global_tau: f32,
        step_fraction: f32,
    ) -> [f32; 2] {
        let velocity: [f32; 3] = std::array::from_fn(|i| {
            ((a.current[i] - a.initial[i]) * self.axis_a[i]
                + (b.current[i] - b.initial[i]) * self.axis_b[i])
                * self.damping
                + (a.initial[i] * self.axis_a[i] + b.initial[i] * self.axis_b[i])
                    * global_tau
                    * self.tau
        });
        let requested = (((self.target_velocity * step_fraction + self.initial_impulse)
            * self.tau)
            - ((velocity[1] + velocity[0]) + velocity[2]))
            * self.effective_mass;
        let (impulse, saturation) = if requested > self.maximum_impulse {
            (
                self.maximum_impulse,
                reciprocal(requested) * self.maximum_impulse,
            )
        } else if requested < self.minimum_impulse {
            (
                self.minimum_impulse,
                reciprocal(requested) * self.minimum_impulse,
            )
        } else {
            (requested, 1.)
        };
        for i in 0..3 {
            a.current[i] += impulse * a.inverse_inertia[i] * self.axis_a[i];
            b.current[i] += impulse * b.inverse_inertia[i] * self.axis_b[i];
        }
        [impulse, saturation]
    }
}

pub struct HingeRowInput {
    pub axis: usize,
    pub frame_a: Basis,
    pub frame_b: Basis,
    pub principal_a: Basis,
    pub principal_b: Basis,
    pub inverse_inertia_a: [f32; 3],
    pub inverse_inertia_b: [f32; 3],
    pub target_angle: f32,
    pub step: MotorStep,
    pub warm_start_scale: f32,
    pub impulse_seconds: f32,
}

fn dot(a: [f32; 4], b: [f32; 4]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}

/// 140c4cf70's plain hinge path. Multi-turn limit atoms have a separate
/// runtime layout and are not passed to this constructor.
pub fn hinge_motor_row(
    motor: &mut ConstraintMotor,
    history: &mut HingeHistory,
    input: HingeRowInput,
) -> AngularMotorRow {
    let a = input.frame_a[(input.axis + 1) % 3];
    let b = input.frame_b[(input.axis + 2) % 3];
    let axis_b = input.frame_b[input.axis];
    let cross = [
        axis_b[2] * b[1] - b[2] * axis_b[1],
        axis_b[0] * b[2] - b[0] * axis_b[2],
        axis_b[1] * b[0] - b[1] * axis_b[0],
        0.,
    ];
    let angle = hinge_angle(dot(b, a), dot(cross, a));
    let previous = if history.initialized {
        history.previous_angle
    } else {
        angle
    };
    let world_axis = input.frame_a[input.axis];
    let project = |basis: Basis, sign: f32| {
        std::array::from_fn::<_, 3, _>(|i| {
            ((sign * world_axis[1]) * basis[1][i] + (sign * world_axis[0]) * basis[0][i])
                + (sign * world_axis[2]) * basis[2][i]
        })
    };
    let mut axis_a = project(input.principal_a, 1.);
    let axis_b = project(input.principal_b, -1.);
    let inverse: [f32; 3] = std::array::from_fn(|i| {
        axis_b[i] * axis_b[i] * input.inverse_inertia_b[i]
            + axis_a[i] * axis_a[i] * input.inverse_inertia_a[i]
    });
    let effective_mass =
        reciprocal((inverse[1] + inverse[0]) + inverse[2] + f32::from_bits(0x3400_0000));
    let command = motor.command(MotorStep {
        position_error: input.target_angle - previous,
        previous_position_error: previous - angle,
        ..input.step
    });
    history.initialized = true;
    history.previous_angle = input.target_angle;
    axis_a[0] = schema_axis(axis_a[0], 15);
    AngularMotorRow {
        axis_a,
        axis_b,
        effective_mass,
        initial_impulse: command[0] * input.warm_start_scale,
        maximum_impulse: command[2] * input.impulse_seconds,
        minimum_impulse: command[3] * input.impulse_seconds,
        target_velocity: command[1],
        tau: command[4],
        damping: command[5],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gameplay::machinery::MotorMode;

    #[test]
    fn lock_rows_match_executable_construction() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/constraint_lock_rows.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let mut fields = line.split_whitespace();
            let angular = fields.next() == Some("1");
            let bits: Vec<u32> = fields
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            let body = |at, center, inverse| ConstraintBody {
                principal: std::array::from_fn(|c| std::array::from_fn(|r| v(at + c * 4 + r))),
                center: vector(center),
                inverse_inertia: vector(inverse),
                inverse_mass: v(inverse + 3),
            };
            let a = body(9, 33, 39);
            let b = body(21, 36, 43);
            let actual = if angular {
                let row = angular_lock_row(a, b, vector(0), vector(3), vector(6), v(47), v(48));
                vec![
                    row.axis_a[0],
                    row.axis_a[1],
                    row.axis_a[2],
                    row.effective_mass,
                    row.axis_b[0],
                    row.axis_b[1],
                    row.axis_b[2],
                    row.correction,
                ]
            } else {
                let row = linear_lock_row(a, b, vector(0), vector(3), vector(6), v(47), v(48));
                vec![
                    row.axis[0],
                    row.axis[1],
                    row.axis[2],
                    row.correction,
                    row.angular_a[0],
                    row.angular_a[1],
                    row.angular_a[2],
                    row.effective_mass,
                    row.angular_b[0],
                    row.angular_b[1],
                    row.angular_b[2],
                    row.inverse_effective_mass,
                ]
            };
            for (i, value) in actual.into_iter().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    bits[49 + i],
                    "case {count} field {i}: {value} vs {}",
                    v(49 + i)
                );
            }
            count += 1;
        }
        assert_eq!(count, 512);
    }

    #[test]
    fn angular_motor_solves_match_executable() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/angular_motor_solve.txt")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            let mut a = AngularVelocityAccumulator {
                current: vector(0),
                initial: vector(6),
                inverse_inertia: vector(12),
            };
            let mut b = AngularVelocityAccumulator {
                current: vector(3),
                initial: vector(9),
                inverse_inertia: vector(15),
            };
            let row = AngularMotorRow {
                axis_a: vector(20),
                effective_mass: v(23),
                axis_b: vector(24),
                initial_impulse: v(27),
                maximum_impulse: v(28),
                minimum_impulse: v(29),
                target_velocity: v(30),
                tau: v(31),
                damping: v(32),
            };
            let solved = row.solve(&mut a, &mut b, v(18), v(19));
            let actual: Vec<_> = a
                .current
                .into_iter()
                .chain(b.current)
                .chain(solved)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, bits[33..], "solve case {count}");
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn hinge_rows_match_executable_construction() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/hinge_motor_rows.txt")
            .lines()
            .filter(|line| !line.starts_with('#') && !line.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let basis = |at| std::array::from_fn(|c| std::array::from_fn(|r| v(at + c * 4 + r)));
            let mut motor = ConstraintMotor {
                mode: if v(1) == 1. {
                    MotorMode::Velocity
                } else {
                    MotorMode::Position
                },
                target_velocity: v(3),
                target_acceleration: v(4),
                force: v(5),
                previous_inverse_step: v(8),
            };
            let mut history = HingeHistory {
                initialized: v(2) != 0.,
                previous_angle: v(11),
            };
            let row = hinge_motor_row(
                &mut motor,
                &mut history,
                HingeRowInput {
                    axis: v(0) as usize,
                    frame_a: basis(13),
                    frame_b: basis(25),
                    principal_a: basis(37),
                    principal_b: basis(49),
                    inverse_inertia_a: std::array::from_fn(|i| v(61 + i)),
                    inverse_inertia_b: std::array::from_fn(|i| v(64 + i)),
                    target_angle: v(12),
                    step: MotorStep {
                        seconds: v(9),
                        inverse_seconds: 1. / v(9),
                        time_scale: v(7),
                        measured_velocity: v(6),
                        position_error: 0.,
                        previous_position_error: 0.,
                        previous_solver_value: v(10),
                    },
                    warm_start_scale: 0.25,
                    impulse_seconds: v(9),
                },
            );
            let actual = [
                row.axis_a[0],
                row.axis_a[1],
                row.axis_a[2],
                row.effective_mass,
                row.axis_b[0],
                row.axis_b[1],
                row.axis_b[2],
                row.initial_impulse,
                row.maximum_impulse,
                row.minimum_impulse,
                row.target_velocity,
                row.tau,
                row.damping,
                history.previous_angle,
            ];
            for (i, value) in actual.into_iter().enumerate() {
                assert_eq!(
                    value.to_bits(),
                    bits[67 + i],
                    "case {count}, field {i}: {value} vs {}",
                    v(67 + i)
                );
            }
            count += 1;
        }
        assert_eq!(count, 256);
    }
}
