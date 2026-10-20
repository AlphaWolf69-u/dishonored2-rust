//! Numeric solver velocity accumulation and final rigid-motion publication.
use super::{ball_socket::SolverVelocities, inverse_length, step::SolverParameters};

#[derive(Clone, Copy, Debug)]
pub struct MotionProperties {
    pub gravity_factor: f32,
    pub maximum_linear_velocity: f32,
    pub maximum_angular_velocity: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
    pub linear_threshold: f32,
    pub linear_reduction: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct MotionHistory {
    pub linear: [f32; 3],
    pub integrated_linear: [f32; 3],
    pub maximum_angular_velocity: f32,
    pub maximum_linear_change: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct MotionPublication {
    pub linear: [f32; 3],
    pub angular: [f32; 3],
    pub integration_linear: [f32; 3],
    pub integration_angular: [f32; 3],
}

fn squared(v: [f32; 3]) -> f32 {
    (v[1] * v[1] + v[0] * v[0]) + v[2] * v[2]
}

fn linear_delta(
    mut delta: [f32; 3],
    properties: MotionProperties,
    step: &SolverParameters,
    final_step: bool,
) -> [f32; 3] {
    let length_squared = squared(delta);
    #[cfg(target_arch = "x86_64")]
    let estimate = if length_squared > 0. {
        unsafe {
            use std::arch::x86_64::*;
            _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(length_squared)))
        }
    } else {
        0.
    };
    #[cfg(not(target_arch = "x86_64"))]
    let estimate = if length_squared > 0. {
        1. / length_squared.sqrt()
    } else {
        0.
    };
    let length = estimate * length_squared;
    if !(properties.linear_threshold < length) {
        let reduction = (properties.linear_reduction * step.substep_seconds).min(length);
        delta = delta.map(|v| (length - reduction) * v * estimate);
    }
    if properties.maximum_linear_velocity * properties.maximum_linear_velocity < length_squared {
        let inverse = if final_step {
            inverse_length(length_squared)
        } else {
            inverse_length(squared(delta))
        };
        if final_step {
            delta = delta.map(|v| inverse * properties.maximum_linear_velocity * v);
        } else if properties.maximum_linear_velocity < inverse * squared(delta) {
            delta = delta.map(|v| properties.maximum_linear_velocity * inverse * v);
        }
    }
    delta
}

impl SolverVelocities {
    /// 1412812f0 initializes a zero accumulated velocity beside the current
    /// motion velocities. Gravity belongs to each simulation substep.
    pub fn begin_motion(
        linear: [f32; 3],
        angular: [f32; 3],
        inverse_mass: [f32; 4],
        properties: MotionProperties,
        step: &SolverParameters,
    ) -> Self {
        let mut result = Self {
            linear: [0.; 4],
            angular: [0.; 4],
            inverse_mass,
            initial_linear: [0.; 4],
            initial_angular: [0.; 4],
        };
        for i in 0..3 {
            result.linear[i] = linear[i] + properties.gravity_factor * step.gravity_substep[i];
            result.angular[i] = angular[i];
        }
        result
    }

    /// Physical lanes of 1412b4ce0. World gravity modifiers are applied by the
    /// owner before passing `gravity_delta`; no host time or gravity fallback.
    pub fn advance_substep(
        &mut self,
        properties: MotionProperties,
        step: &SolverParameters,
        maximum_angular: f32,
        gravity_delta: [f32; 3],
    ) {
        let linear = linear_delta(
            std::array::from_fn(|i| self.linear[i] - self.initial_linear[i]),
            properties,
            step,
            false,
        );
        let mut angular: [f32; 3] =
            std::array::from_fn(|i| self.angular[i] - self.initial_angular[i]);
        let length = squared(angular);
        let maximum = properties.maximum_angular_velocity.min(maximum_angular);
        if maximum * maximum < length {
            let inverse = inverse_length(length);
            if maximum < inverse * length {
                angular = angular.map(|v| maximum * inverse * v);
            }
        }
        for i in 0..3 {
            self.initial_linear[i] += linear[i] * step.inverse_velocity_scale;
            self.initial_angular[i] += angular[i] * step.inverse_velocity_scale;
            self.linear[i] = self.initial_linear[i] + linear[i] + gravity_delta[i];
            self.angular[i] = self.initial_angular[i] + angular[i];
        }
    }

    /// Physical lanes of 1412b5130's final velocity/pose producer. The motion
    /// velocities saved for the next step differ from the integration average.
    pub fn finish_motion(
        &self,
        properties: MotionProperties,
        step: &SolverParameters,
        history: MotionHistory,
    ) -> MotionPublication {
        let linear = linear_delta(
            std::array::from_fn(|i| self.linear[i] - self.initial_linear[i]),
            properties,
            step,
            true,
        );
        let mut angular: [f32; 3] =
            std::array::from_fn(|i| self.angular[i] - self.initial_angular[i]);
        let length = squared(angular);
        let maximum = properties
            .maximum_angular_velocity
            .min(history.maximum_angular_velocity);
        let mut angular_scale = 1.;
        if maximum * maximum < length {
            let inverse = inverse_length(length);
            angular_scale = (inverse * history.maximum_angular_velocity).min(1.);
            angular = angular.map(|v| v * inverse * maximum);
        }
        let mut integration_linear = std::array::from_fn(|i| {
            (step.inverse_velocity_scale * linear[i] + self.initial_linear[i])
                * step.velocity_scale
                * step.inverse_substeps
        });
        let integration_angular = std::array::from_fn(|i| {
            (step.inverse_velocity_scale * angular[i] + self.initial_angular[i])
                * step.velocity_scale
                * step.inverse_substeps
        });
        if history.maximum_linear_change < f32::from_bits(0x5f7ffff0) || angular_scale < 1. {
            let limit = step.inverse_seconds * history.maximum_linear_change;
            integration_linear = std::array::from_fn(|i| {
                let high = history.linear[i].max(0.).max(history.integrated_linear[i]) + limit;
                let low = history.linear[i].min(0.).min(history.integrated_linear[i]) - limit;
                (angular_scale * integration_linear[i]).min(high).max(low)
            });
        }
        MotionPublication {
            linear: linear.map(|v| v * (1. - (properties.linear_damping * step.seconds).min(1.))),
            angular: angular
                .map(|v| v * (1. - (properties.angular_damping * step.seconds).min(1.))),
            integration_linear,
            integration_angular,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intermediate_velocity_updates_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/solver_velocity_update.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let mut velocity = SolverVelocities {
                linear: [v(0), v(1), v(2), 0.],
                angular: [v(3), v(4), v(5), 0.],
                inverse_mass: [1., 1., 1., v(18)],
                initial_linear: [v(6), v(7), v(8), 0.],
                initial_angular: [v(9), v(10), v(11), 0.],
            };
            let p = MotionProperties {
                gravity_factor: v(12),
                maximum_linear_velocity: v(13),
                maximum_angular_velocity: v(14),
                linear_damping: 0.,
                angular_damping: 0.,
                linear_threshold: v(15),
                linear_reduction: v(16),
            };
            let step = SolverParameters::new(
                v(19),
                v(20) as u32,
                1,
                f32::from_bits(0x3f19999a),
                1.,
                [v(21), v(22), v(23), 0.],
            );
            let gravity = std::array::from_fn(|i| {
                if v(18) == 0. {
                    0.
                } else {
                    p.gravity_factor * step.gravity_substep[i]
                }
            });
            velocity.advance_substep(p, &step, v(17), gravity);
            let actual: Vec<u32> = velocity.linear[..3]
                .iter()
                .chain(&velocity.angular[..3])
                .chain(&velocity.initial_linear[..3])
                .chain(&velocity.initial_angular[..3])
                .map(|v| v.to_bits())
                .collect();
            assert_eq!(actual, bits[24..], "velocity update case {count}");
            count += 1;
        }
        assert_eq!(count, 256);
    }
    #[test]
    fn final_motion_matches_gameplay_publication() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/motion_publication_cases.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            let mut pose = crate::gameplay::rigid_body::BodyPose {
                center: vector(0),
                orientation: vector(4),
            };
            let velocity = SolverVelocities {
                linear: [v(16), v(17), v(18), 0.],
                angular: [v(19), v(20), v(21), 0.],
                inverse_mass: [0.; 4],
                initial_linear: [v(22), v(23), v(24), 0.],
                initial_angular: [v(25), v(26), v(27), 0.],
            };
            let properties = MotionProperties {
                gravity_factor: v(28),
                maximum_linear_velocity: v(29),
                maximum_angular_velocity: v(30),
                linear_damping: v(31),
                angular_damping: v(32),
                linear_threshold: v(33),
                linear_reduction: v(34),
            };
            let step =
                SolverParameters::new(v(35), v(36) as u32, v(37) as u32, v(38), v(39), vector(40));
            let result = velocity.finish_motion(
                properties,
                &step,
                MotionHistory {
                    linear: std::array::from_fn(|i| v(8 + i)),
                    integrated_linear: std::array::from_fn(|i| v(11 + i)),
                    maximum_angular_velocity: v(14),
                    maximum_linear_change: v(15),
                },
            );
            pose.integrate(
                [
                    result.integration_linear[0],
                    result.integration_linear[1],
                    result.integration_linear[2],
                    0.,
                ],
                [
                    result.integration_angular[0],
                    result.integration_angular[1],
                    result.integration_angular[2],
                    0.,
                ],
                [step.seconds; 4],
            );
            let actual: Vec<u32> = result
                .linear
                .into_iter()
                .chain(result.angular)
                .chain(result.integration_linear)
                .chain(result.integration_angular)
                .chain(pose.center)
                .chain(pose.orientation)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, bits[44..], "live motion publication case {count}");
            count += 1;
        }
        assert_eq!(count, 24);
    }
}
