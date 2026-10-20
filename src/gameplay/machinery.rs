//! Constraint motor commands, independent of the renderer and physics solver.
//! FUN_140712bd0 consumes the preceding measured motion, not a frame counter.

use crate::entities::machinery::AngularMotorSettings;
pub use crate::entities::machinery::MotorMode;

#[derive(Clone, Copy, Debug)]
pub struct ConstraintMotor {
    pub mode: MotorMode,
    pub target_velocity: f32,
    pub target_acceleration: f32,
    pub force: f32,
    pub previous_inverse_step: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct MotorStep {
    pub seconds: f32,
    pub inverse_seconds: f32,
    pub time_scale: f32,
    pub measured_velocity: f32,
    pub position_error: f32,
    pub previous_position_error: f32,
    pub previous_solver_value: f32,
}

/// Relative-pose history updated by the constraint component before solving.
#[derive(Clone, Copy, Debug)]
pub struct AngularMotorHistory {
    pub relative_orientation: [f32; 4],
}

impl Default for AngularMotorHistory {
    fn default() -> Self {
        Self {
            relative_orientation: [0., 0., 0., 1.],
        }
    }
}

fn asin(value: f32) -> f32 {
    let x = value.abs().min(1.);
    if x < f32::from_bits(0x38d1b717) {
        return x.copysign(value);
    }
    let high = x > 0.5;
    let t = if high { (1. - x) * 0.5 } else { x * x };
    let s = if high { t.sqrt() } else { x };
    let c = |bits| f32::from_bits(bits);
    let p = ((((c(0x3d2cb352) * t + c(0x3cc617e3)) * t + c(0x3d3a3ec7)) * t + c(0x3d9980f6)) * t
        + c(0x3e2aaae4))
        * t
        * s
        + s;
    let result = if high {
        f32::from_bits(0x3fc90fdb) - (p + p)
    } else {
        p
    };
    result.copysign(value)
}

impl AngularMotorHistory {
    /// Angular half of 140715f80. The rate comes from preceding relative poses,
    /// with quaternion sign folding; it is not the motor's requested velocity.
    pub fn measure(
        &mut self,
        a: [[f32; 4]; 3],
        b: [[f32; 4]; 3],
        local_axis: [f32; 3],
        seconds: f32,
        time_scale: f32,
    ) -> f32 {
        let relative = std::array::from_fn(|column| {
            std::array::from_fn(|row| {
                if row == 3 {
                    0.
                } else {
                    (b[column][1] * a[row][1] + b[column][0] * a[row][0]) + b[column][2] * a[row][2]
                }
            })
        });
        let current = crate::physworld::rotation_quaternion(relative);
        let [px, py, pz, pw] = self.relative_orientation;
        let [x, y, z, w] = current;
        let delta = crate::physworld::normalize_body_quaternion([
            ((z * py - pz * y) - w * px) + pw * x,
            ((x * pz - px * z) - w * py) + pw * y,
            ((y * px - py * x) - w * pz) + pw * z,
            (x * px + z * pz) + (y * py + w * pw),
        ]);
        let length = (delta[1] * delta[1] + delta[0] * delta[0]) + delta[2] * delta[2];
        let measured = if length <= f32::from_bits(0x28800000) {
            0.
        } else {
            #[cfg(target_arch = "x86_64")]
            let reciprocal_length = unsafe {
                use std::arch::x86_64::*;
                let estimate = _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(length)));
                (3. - estimate * length * estimate) * (estimate * 0.5)
            };
            #[cfg(not(target_arch = "x86_64"))]
            let reciprocal_length = 1. / length.sqrt();
            let axis: [f32; 3] = std::array::from_fn(|i| {
                let value = reciprocal_length * delta[i];
                if delta[3] < 0. {
                    -value
                } else {
                    value
                }
            });
            let angle = f32::from_bits(0x3fc90fdb) - asin(delta[3].abs());
            let angle = angle + angle;
            -(((axis[1] * local_axis[1] + axis[0] * local_axis[0]) + axis[2] * local_axis[2])
                * angle
                * (time_scale / seconds))
        };
        self.relative_orientation = current;
        measured
    }
}

impl ConstraintMotor {
    /// 140712510's history starts at zero; declaration settings are applied
    /// by 140713850 before the first solver callback.
    pub fn from_settings(settings: AngularMotorSettings) -> Self {
        Self {
            mode: settings.mode,
            target_velocity: settings.target_velocity,
            target_acceleration: settings.target_acceleration,
            force: settings.force,
            previous_inverse_step: 0.,
        }
    }
    /// Six Havok callback outputs, in the game's order. Position motors use
    /// the same acceleration limiter but clamp the error-derived velocity.
    pub fn command(&mut self, step: MotorStep) -> [f32; 6] {
        let acceleration = step.time_scale * self.target_acceleration * step.time_scale;
        let (target, limit) = match self.mode {
            MotorMode::Velocity => (self.target_velocity, acceleration),
            MotorMode::Position => {
                let desired =
                    (step.previous_position_error + step.position_error) * step.inverse_seconds;
                if desired < -self.target_velocity {
                    (-self.target_velocity, acceleration)
                } else if self.target_velocity < desired {
                    (self.target_velocity, acceleration)
                } else {
                    (desired, f32::from_bits(0x7f7f_ffee))
                }
            }
        };
        let mut velocity = target * step.time_scale;
        let change = (velocity - step.measured_velocity) * step.inverse_seconds;
        // A zero target bypasses the limiter in 140712c72..140712cb3.
        if velocity != 0.0 {
            if change > limit {
                velocity = step.seconds * limit + step.measured_velocity;
            } else if change < -limit {
                velocity = step.measured_velocity - step.seconds * limit;
            }
        }
        let impulse = step.previous_solver_value * (step.seconds * self.previous_inverse_step);
        self.previous_inverse_step = step.inverse_seconds;
        [impulse, velocity, self.force, -self.force, 1.0, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_motor_motion_matches_executable() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/motor_measurement_cases.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let basis = |at| std::array::from_fn(|c| std::array::from_fn(|r| v(at + c * 4 + r)));
            let mut history = AngularMotorHistory {
                relative_orientation: std::array::from_fn(|i| v(24 + i)),
            };
            let measured = history.measure(
                basis(0),
                basis(12),
                std::array::from_fn(|i| v(28 + i)),
                v(31),
                v(32),
            );
            let actual: Vec<u32> = [measured]
                .into_iter()
                .chain(history.relative_orientation)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, bits[33..], "measurement case {count}");
            count += 1;
        }
        assert_eq!(count, 512);
    }

    #[test]
    fn motor_commands_match_executable_callback() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/constraint_motor_cases.txt")
            .lines()
            .filter(|row| !row.starts_with('#') && !row.is_empty())
        {
            let mut values = row.split_whitespace();
            let mode = if values.next() == Some("1") {
                MotorMode::Velocity
            } else {
                MotorMode::Position
            };
            let bits: Vec<_> = values
                .map(|value| u32::from_str_radix(value, 16).unwrap())
                .collect();
            let v = |index| f32::from_bits(bits[index]);
            let mut motor = ConstraintMotor {
                mode,
                target_velocity: v(0),
                target_acceleration: v(1),
                force: v(2),
                previous_inverse_step: v(3),
            };
            let actual = motor.command(MotorStep {
                seconds: v(4),
                inverse_seconds: v(10),
                time_scale: v(5),
                measured_velocity: v(6),
                position_error: v(7),
                previous_position_error: v(8),
                previous_solver_value: v(9),
            });
            assert_eq!(
                actual.map(f32::to_bits),
                bits[11..17],
                "callback case {count}"
            );
            assert_eq!(motor.previous_inverse_step.to_bits(), bits[10]);
            count += 1;
        }
        assert_eq!(count, 1024);
    }
}
