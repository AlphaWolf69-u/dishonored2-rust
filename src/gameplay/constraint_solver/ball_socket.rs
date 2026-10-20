//! Stabilized ball-socket rows used by the constraint component's fixed pivot.
use super::{inverse_length, reciprocal};

#[derive(Clone, Copy, Debug)]
pub struct AnchorFrame {
    pub center: [f32; 4],
    pub world_to_mass: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct SolverVelocities {
    pub linear: [f32; 4],
    pub angular: [f32; 4],
    pub inverse_mass: [f32; 4],
    pub initial_linear: [f32; 4],
    pub initial_angular: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct PredictedFrame {
    pub center: [f32; 4],
    pub mass_to_world: [f32; 4],
}

/// Quaternion-vector arithmetic shared by 140ae1890 and the stabilized row
/// solver. Its three inputs are direction lanes; Havok padding is not a vector.
pub fn rotate_direction(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let dot = (v[1] * q[1] + v[0] * q[0]) + v[2] * q[2];
    let diagonal = q[3] * q[3] - 0.5;
    let cross = [
        v[2] * q[1] - q[2] * v[1],
        v[0] * q[2] - q[0] * v[2],
        v[1] * q[0] - q[1] * v[0],
    ];
    std::array::from_fn(|i| {
        let t = (dot * q[i] + diagonal * v[i]) + cross[i] * q[3];
        t + t
    })
}

#[derive(Clone, Copy, Debug)]
pub struct BallSocketJacobian {
    pub inverse_effective_mass: [[f32; 3]; 3],
    pub angular_a: [[f32; 3]; 3],
    pub angular_b: [[f32; 3]; 3],
}

/// 14129d200's three physical matrix lanes. The extra SIMD padding lanes in
/// its temporary arrays do not participate in the 3-by-3 solve.
pub fn ball_socket_jacobian(
    axes: [[f32; 3]; 3],
    a: SolverVelocities,
    b: SolverVelocities,
    predicted_a: PredictedFrame,
    predicted_b: PredictedFrame,
    lever_a: [f32; 3],
    lever_b: [f32; 3],
    length_a: f32,
    length_b: f32,
    angular_scale_a: f32,
    angular_scale_b: f32,
) -> BallSocketJacobian {
    let angular = |q: [f32; 4], lever: [f32; 3]| {
        let inverse = [-q[0], -q[1], -q[2], q[3]];
        axes.map(|axis| {
            rotate_direction(
                inverse,
                [
                    axis[2] * lever[1] - axis[1] * lever[2],
                    axis[0] * lever[2] - axis[2] * lever[0],
                    axis[1] * lever[0] - axis[0] * lever[1],
                ],
            )
        })
    };
    let angular_a = angular(predicted_a.mass_to_world, lever_a);
    let angular_b = angular(predicted_b.mass_to_world, lever_b);
    let weight_a = a.inverse_mass.map(|v| v * (length_a * angular_scale_a));
    let weight_b = b.inverse_mass.map(|v| v * (length_b * angular_scale_b));
    let linear = b.inverse_mass[3] + a.inverse_mass[3];
    let inverse_effective_mass = std::array::from_fn(|r| {
        std::array::from_fn(|c| {
            let z = angular_b[r][2] * angular_b[c][2] * weight_b[2]
                + angular_a[r][2] * angular_a[c][2] * weight_a[2];
            let x = angular_b[r][0] * angular_b[c][0] * weight_b[0]
                + angular_a[r][0] * angular_a[c][0] * weight_a[0];
            let y = angular_b[r][1] * angular_b[c][1] * weight_b[1]
                + angular_a[r][1] * angular_a[c][1] * weight_a[1];
            ((z + x) + y) + if r == c { linear } else { 0. }
        })
    });
    BallSocketJacobian {
        inverse_effective_mass,
        angular_a,
        angular_b,
    }
}

/// One body's half of 14129d6f0. Solver anchor frames store inverse rotations;
/// prediction publishes the forward rotation used to rebuild the Jacobians.
pub fn predict_anchor(
    frame: AnchorFrame,
    velocity: SolverVelocities,
    seconds: f32,
    velocity_scale: f32,
) -> PredictedFrame {
    let scale = velocity_scale - 1.;
    let center = std::array::from_fn(|i| {
        (velocity.initial_linear[i] * scale + velocity.linear[i]) * seconds + frame.center[i]
    });
    let half = seconds * 0.5;
    let [ax, ay, az] = std::array::from_fn::<_, 3, _>(|i| {
        (velocity.initial_angular[i] * scale + velocity.angular[i]) * half
    });
    let [x, y, z, w] = frame.world_to_mass;
    let next = [
        (w * ax + (z * ay - az * y)) - x,
        (w * ay + (x * az - ax * z)) - y,
        (w * az + (y * ax - ay * x)) - z,
        (x * ax + z * az) + (y * ay + w),
    ];
    let mass_to_world = crate::physworld::normalize_body_quaternion(next);
    PredictedFrame {
        center,
        mass_to_world,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BallSocketBody {
    pub frame: AnchorFrame,
    pub inverse_mass: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct BallSocketBuild {
    pub a: BallSocketBody,
    pub b: BallSocketBody,
    /// Pivot offset in each body's principal mass frame.
    pub lever_a: [f32; 3],
    pub lever_b: [f32; 3],
    pub inverse_step: f32,
    pub tau: f32,
    pub damping: f32,
    pub mass_stabilization: f32,
    pub maximum_force: f32,
    pub maximum_impulse: f32,
    pub angular_impulse_limit: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct BallSocketRow {
    pub direction_a: [f32; 3],
    pub direction_b: [f32; 3],
    pub length_a: f32,
    pub length_b: f32,
    pub angular_scale_a: f32,
    pub angular_scale_b: f32,
    pub inverse_step_tau: f32,
    pub damping: f32,
    pub maximum_force_squared: f32,
    pub maximum_impulse: f32,
    pub angular_impulse_limit: f32,
    pub a: AnchorFrame,
    pub b: AnchorFrame,
}

impl BallSocketRow {
    /// 1412a0ce0 and 14129fda0: predict the two anchors, rebuild their coupled
    /// effective mass and apply separately capped linear/angular impulses.
    pub fn solve(
        &self,
        a: &mut SolverVelocities,
        b: &mut SolverVelocities,
        seconds: f32,
        velocity_scale: f32,
    ) -> [f32; 3] {
        let predicted_a = predict_anchor(self.a, *a, seconds, velocity_scale);
        let predicted_b = predict_anchor(self.b, *b, seconds, velocity_scale);
        let lever_a = rotate_direction(predicted_a.mass_to_world, self.direction_a);
        let lever_b = rotate_direction(predicted_b.mass_to_world, self.direction_b);
        let jacobian = ball_socket_jacobian(
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            *a,
            *b,
            predicted_a,
            predicted_b,
            lever_a,
            lever_b,
            self.length_a,
            self.length_b,
            self.angular_scale_a,
            self.angular_scale_b,
        );
        let correction: [f32; 3] = std::array::from_fn(|i| {
            ((lever_b[i] * self.length_b - lever_a[i] * self.length_a)
                + (predicted_b.center[i] - predicted_a.center[i]))
                * self.inverse_step_tau
        });
        let k = jacobian.inverse_effective_mass;
        let cross = |a: [f32; 3], b: [f32; 3]| {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        };
        let cofactors = [cross(k[1], k[2]), cross(k[2], k[0]), cross(k[0], k[1])];
        let determinant =
            (k[0][1] * cofactors[0][1] + k[0][0] * cofactors[0][0]) + k[0][2] * cofactors[0][2];
        let l1 = |v: [f32; 3]| (v[1].abs() + v[0].abs()) + v[2].abs();
        let threshold = l1(k[2]) * l1(k[1]) * l1(k[0]) * f32::from_bits(0x34000000);
        let impulse = if threshold < determinant.abs() {
            let inverse = reciprocal(determinant);
            std::array::from_fn(|i| {
                ((correction[1] * (inverse * cofactors[1][i])
                    + correction[0] * (inverse * cofactors[0][i]))
                    + correction[2] * (inverse * cofactors[2][i]))
                    * self.damping
            })
        } else {
            [0.; 3]
        };
        let squared = (impulse[1] * impulse[1] + impulse[0] * impulse[0]) + impulse[2] * impulse[2];
        let inverse = inverse_length(squared);
        let cap = |maximum: f32| {
            if maximum < inverse * squared {
                impulse.map(|v| maximum * inverse * v)
            } else {
                impulse
            }
        };
        let linear = cap(self.maximum_impulse);
        let angular = cap(self.angular_impulse_limit);
        for i in 0..3 {
            let torque_a = (jacobian.angular_a[0][i] * angular[0]
                + jacobian.angular_a[1][i] * angular[1])
                + jacobian.angular_a[2][i] * angular[2];
            let torque_b = (jacobian.angular_b[0][i] * angular[0]
                + jacobian.angular_b[1][i] * angular[1])
                + jacobian.angular_b[2][i] * angular[2];
            a.angular[i] += (torque_a * self.angular_scale_a) * a.inverse_mass[i];
            b.angular[i] -= b.inverse_mass[i] * torque_b * self.angular_scale_b;
            a.linear[i] += a.inverse_mass[3] * linear[i];
            b.linear[i] -= b.inverse_mass[3] * linear[i];
        }
        linear
    }

    /// 140c49b20, with the atom's optional impulse cap resolved by its owner.
    pub fn build(input: BallSocketBuild) -> Self {
        let direction = |lever: [f32; 3]| {
            let squared = (lever[1] * lever[1] + lever[0] * lever[0]) + lever[2] * lever[2];
            let inverse = inverse_length(squared);
            let direction = if squared > f32::from_bits(0x28800000) {
                lever.map(|v| inverse * v)
            } else {
                [1., 0., 0.]
            };
            (direction, inverse * squared)
        };
        let (direction_a, length_a) = direction(input.lever_a);
        let (direction_b, length_b) = direction(input.lever_b);
        let stabilize = |body: BallSocketBody, length| {
            let maximum = body.inverse_mass[0]
                .max(body.inverse_mass[1])
                .max(body.inverse_mass[2]);
            let scale = maximum * length * input.mass_stabilization;
            let fraction = if f32::from_bits(0x34000000) < scale {
                reciprocal(scale) * body.inverse_mass[3]
            } else {
                1.
            };
            fraction.min(1.) * length
        };
        Self {
            direction_a,
            direction_b,
            length_a,
            length_b,
            angular_scale_a: stabilize(input.a, length_a),
            angular_scale_b: stabilize(input.b, length_b),
            inverse_step_tau: input.tau * input.inverse_step,
            damping: input.damping,
            maximum_force_squared: if input.maximum_force < f32::from_bits(0x5f7ffff0) {
                input.maximum_force * input.maximum_force
            } else {
                input.maximum_force
            },
            maximum_impulse: input.maximum_impulse,
            angular_impulse_limit: input.angular_impulse_limit,
            a: input.a.frame,
            b: input.b.frame,
        }
    }

    /// The numerical fields of schema 17, excluding its packed first word.
    pub fn schema_fields(&self) -> [f32; 32] {
        let mut fields = [0.; 32];
        fields[1] = self.maximum_impulse;
        fields[2] = self.maximum_force_squared;
        fields[4..7].copy_from_slice(&self.direction_a);
        fields[7] = self.length_a;
        fields[8..11].copy_from_slice(&self.direction_b);
        fields[11] = self.length_b;
        fields[12] = self.inverse_step_tau;
        fields[13] = self.angular_impulse_limit;
        fields[14] = self.angular_scale_a;
        fields[15] = self.angular_scale_b;
        fields[16..20].copy_from_slice(&self.a.center);
        fields[19] = self.damping;
        fields[20..24].copy_from_slice(&self.b.center);
        fields[24..28].copy_from_slice(&self.a.world_to_mass);
        fields[28..32].copy_from_slice(&self.b.world_to_mass);
        fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ball_socket_solves_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ball_socket_solve.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            let body = |pose, mass| BallSocketBody {
                frame: AnchorFrame {
                    center: vector(pose),
                    world_to_mass: vector(pose + 4),
                },
                inverse_mass: vector(mass),
            };
            let row = BallSocketRow::build(BallSocketBuild {
                a: body(0, 16),
                b: body(8, 20),
                lever_a: std::array::from_fn(|i| v(24 + i)),
                lever_b: std::array::from_fn(|i| v(27 + i)),
                inverse_step: v(30),
                tau: v(31),
                damping: v(32),
                mass_stabilization: v(33),
                maximum_force: v(34),
                maximum_impulse: v(35),
                angular_impulse_limit: v(36),
            });
            let velocity = |at| SolverVelocities {
                linear: vector(at),
                angular: vector(at + 4),
                inverse_mass: vector(at + 8),
                initial_linear: vector(at + 12),
                initial_angular: vector(at + 16),
            };
            let mut a = velocity(37);
            let mut b = velocity(57);
            let impulse = row.solve(&mut a, &mut b, v(77), v(78));
            let actual: Vec<f32> = a.linear[..3]
                .iter()
                .chain(&a.angular[..3])
                .chain(&b.linear[..3])
                .chain(&b.angular[..3])
                .chain(&impulse)
                .copied()
                .collect();
            for (i, actual) in actual.into_iter().enumerate() {
                let expected = v(79 + i);
                assert!(
                    (actual - expected).abs() <= 0.0002 * expected.abs().max(1.),
                    "fixed pivot case {count} field {i}: {actual} vs {expected}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 256);
    }
    #[test]
    fn ball_socket_jacobians_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ball_socket_jacobians.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            let predicted = |at| PredictedFrame {
                center: [0.; 4],
                mass_to_world: vector(at),
            };
            let body = |at| SolverVelocities {
                linear: [0.; 4],
                angular: [0.; 4],
                inverse_mass: vector(at),
                initial_linear: [0.; 4],
                initial_angular: [0.; 4],
            };
            let row = ball_socket_jacobian(
                [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
                body(18),
                body(22),
                predicted(0),
                predicted(4),
                std::array::from_fn(|i| v(8 + i)),
                std::array::from_fn(|i| v(11 + i)),
                v(14),
                v(15),
                v(16),
                v(17),
            );
            let actual: Vec<f32> = row
                .inverse_effective_mass
                .into_iter()
                .chain(row.angular_a)
                .chain(row.angular_b)
                .flatten()
                .collect();
            for (i, actual) in actual.into_iter().enumerate() {
                let expected = v(26 + i);
                assert!(
                    (actual - expected).abs() <= 0.00001 * expected.abs().max(1.),
                    "Jacobian case {count} field {i}: {actual} vs {expected}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 256);
    }
    #[test]
    fn constraint_predictions_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/constraint_prediction.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            for side in 0..2 {
                let p = side * 8;
                let b = 16 + side * 20;
                let predicted = predict_anchor(
                    AnchorFrame {
                        center: vector(p),
                        world_to_mass: vector(p + 4),
                    },
                    SolverVelocities {
                        linear: vector(b),
                        angular: vector(b + 4),
                        inverse_mass: vector(b + 8),
                        initial_linear: vector(b + 12),
                        initial_angular: vector(b + 16),
                    },
                    v(56),
                    v(57),
                );
                let actual: Vec<u32> = predicted
                    .mass_to_world
                    .into_iter()
                    .chain(predicted.center)
                    .map(f32::to_bits)
                    .collect();
                assert_eq!(
                    actual,
                    bits[58 + side * 8..66 + side * 8],
                    "prediction case {count}, side {side}"
                );
            }
            count += 1;
        }
        assert_eq!(count, 256);
    }
    #[test]
    fn ball_socket_rows_match_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/ball_socket_rows.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            let body = |pose, mass| BallSocketBody {
                frame: AnchorFrame {
                    center: vector(pose),
                    world_to_mass: vector(pose + 4),
                },
                inverse_mass: vector(mass),
            };
            let row = BallSocketRow::build(BallSocketBuild {
                a: body(0, 16),
                b: body(8, 20),
                lever_a: std::array::from_fn(|i| v(24 + i)),
                lever_b: std::array::from_fn(|i| v(27 + i)),
                inverse_step: v(30),
                tau: v(31),
                damping: v(32),
                mass_stabilization: v(33),
                maximum_force: v(34),
                maximum_impulse: v(35),
                angular_impulse_limit: v(36),
            });
            for (i, actual) in row.schema_fields().into_iter().enumerate().skip(1) {
                assert_eq!(
                    actual.to_bits(),
                    bits[37 + i],
                    "ball socket case {count} field {i}: {actual} vs {}",
                    v(37 + i)
                );
            }
            count += 1;
        }
        assert_eq!(count, 256);
    }
}
