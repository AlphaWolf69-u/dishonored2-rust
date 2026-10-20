//! Angular row consumers from 14129af90 (schemas 12, 13 and 14).
use super::ball_socket::rotate_direction;
use super::{
    hinge_angle, inverse_length, project, reciprocal, schema_axis, AngularLockRow,
    AngularVelocityAccumulator, Basis, ConstraintBody,
};

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn apply(
    a: &mut AngularVelocityAccumulator,
    b: &mut AngularVelocityAccumulator,
    ja: [f32; 3],
    jb: [f32; 3],
    impulse: f32,
) {
    for i in 0..3 {
        a.current[i] += impulse * a.inverse_inertia[i] * ja[i];
        b.current[i] += impulse * b.inverse_inertia[i] * jb[i];
    }
}
impl AngularLockRow {
    pub fn solve(
        self,
        a: &mut AngularVelocityAccumulator,
        b: &mut AngularVelocityAccumulator,
    ) -> f32 {
        let velocity: [f32; 3] =
            std::array::from_fn(|i| a.current[i] * self.axis_a[i] + b.current[i] * self.axis_b[i]);
        let impulse =
            (self.correction - ((velocity[1] + velocity[0]) + velocity[2])) * self.effective_mass;
        apply(a, b, self.axis_a, self.axis_b, impulse);
        impulse
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AngularLimitRow {
    pub axis_a: [f32; 3],
    pub axis_b: [f32; 3],
    pub effective_mass: f32,
    pub correction: f32,
    pub lower: f32,
    pub upper: f32,
    pub tau: f32,
    pub damping: f32,
}
/// Intermediate angle producer 140c4a2d0. The previous solver value stores
/// the negative unwrapped angle, keeping limits continuous across +/-pi.
pub fn limit_angle(
    frame_a: Basis,
    frame_b: Basis,
    axis: usize,
    reference: usize,
    previous_internal: f32,
) -> f32 {
    let a = frame_a[reference];
    let b = frame_b[reference];
    let v = frame_b[axis];
    let cross = [
        b[2] * v[1] - v[2] * b[1],
        b[0] * v[2] - v[0] * b[2],
        b[1] * v[0] - v[1] * b[0],
    ];
    let angle = hinge_angle(
        dot(cross, [a[0], a[1], a[2]]),
        dot([b[0], b[1], b[2]], [a[0], a[1], a[2]]),
    );
    let turns =
        ((angle + previous_internal + std::f32::consts::PI) / std::f32::consts::TAU).floor();
    angle - turns * std::f32::consts::TAU
}

pub fn hinge_limit_row(
    a: ConstraintBody,
    b: ConstraintBody,
    fa: Basis,
    fb: Basis,
    axis: usize,
    minimum: f32,
    maximum: f32,
    previous_angle: &mut f32,
    inverse_step: f32,
    tau: f32,
    damping: f32,
) -> AngularLimitRow {
    let angle = limit_angle(fa, fb, axis, (axis + 1) % 3, -*previous_angle);
    *previous_angle = angle;
    let world = [fb[axis][0], fb[axis][1], fb[axis][2]];
    let mut ja = project(a.principal, world);
    let jb = project(b.principal, world.map(|v| -v));
    let inv: [f32; 3] = std::array::from_fn(|i| {
        ja[i] * ja[i] * a.inverse_inertia[i] + jb[i] * jb[i] * b.inverse_inertia[i]
    });
    let effective_mass = tau * reciprocal((inv[1] + inv[0]) + inv[2] + f32::from_bits(0x34000000));
    ja[0] = schema_axis(ja[0], 13);
    AngularLimitRow {
        axis_a: ja,
        axis_b: jb,
        effective_mass,
        correction: angle * -inverse_step,
        lower: minimum * -inverse_step * tau,
        upper: maximum * -inverse_step * tau,
        tau,
        damping,
    }
}

impl AngularLimitRow {
    pub fn solve(
        self,
        a: &mut AngularVelocityAccumulator,
        b: &mut AngularVelocityAccumulator,
        global_tau: f32,
        half_step: f32,
        accumulated: &mut f32,
    ) -> f32 {
        let predicted = |body: &AngularVelocityAccumulator| {
            let v: [f32; 3] = std::array::from_fn(|i| {
                (body.initial[i] * (global_tau - 1.) + body.current[i]) * half_step
            });
            let squared = (v[2] * v[2] + v[0] * v[0]) + (4. + v[1] * v[1]);
            let inverse = inverse_length(squared);
            [v[0] * inverse, v[1] * inverse, v[2] * inverse, 2. * inverse]
        };
        let ja = rotate_direction(predicted(a), self.axis_a);
        let jb = rotate_direction(predicted(b), self.axis_b);
        let velocity: [f32; 3] = std::array::from_fn(|i| {
            ((a.current[i] - a.initial[i]) * self.axis_a[i]
                + (b.current[i] - b.initial[i]) * self.axis_b[i])
                * self.damping
                + (a.initial[i] * self.axis_a[i] + b.initial[i] * self.axis_b[i])
                    * global_tau
                    * self.tau
        });
        let rhs = self.correction * self.tau - ((velocity[1] + velocity[0]) + velocity[2]);
        let mut impulse = -*accumulated;
        let lower = (rhs - self.lower) * self.effective_mass;
        if lower > impulse {
            impulse = lower;
        } else {
            let upper = (rhs - self.upper) * self.effective_mass;
            if upper < impulse {
                impulse = upper;
            }
        }
        apply(a, b, ja, jb, impulse);
        *accumulated += impulse;
        impulse
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AngularFrictionRow {
    pub axis_a: [f32; 3],
    pub axis_b: [f32; 3],
    pub effective_mass: f32,
    pub correction: f32,
    pub maximum_impulse: f32,
}
impl AngularFrictionRow {
    pub fn solve(
        self,
        a: &mut AngularVelocityAccumulator,
        b: &mut AngularVelocityAccumulator,
    ) -> [f32; 2] {
        let velocity: [f32; 3] =
            std::array::from_fn(|i| a.current[i] * self.axis_a[i] + b.current[i] * self.axis_b[i]);
        let requested =
            (self.correction - ((velocity[1] + velocity[0]) + velocity[2])) * self.effective_mass;
        let (impulse, scale) = if requested.abs() > self.maximum_impulse {
            (
                self.maximum_impulse.copysign(requested),
                reciprocal(requested.abs()) * self.maximum_impulse,
            )
        } else {
            (requested, 1.)
        };
        apply(a, b, self.axis_a, self.axis_b, impulse);
        [impulse, scale]
    }
}

pub fn integration_projection(
    axis_a: [f32; 3],
    axis_b: [f32; 3],
    a: AngularVelocityAccumulator,
    b: AngularVelocityAccumulator,
    substeps: u32,
    inverse_velocity_scale: f32,
    velocity_scale: f32,
) -> f32 {
    let prepare = |v: AngularVelocityAccumulator| {
        std::array::from_fn::<_, 3, _>(|i| {
            ((v.current[i] - v.initial[i]) * inverse_velocity_scale + v.initial[i])
                * (velocity_scale / substeps as f32)
        })
    };
    let a = prepare(a);
    let b = prepare(b);
    (((axis_b[1] * b[1] + a[1] * axis_a[1]) + axis_b[0] * b[0]) + a[0] * axis_a[0])
        + axis_b[2] * b[2]
        + a[2] * axis_a[2]
}

/// Angular part of the 1412a1570 export preparation and schema15 in
/// 14129c3f0. This value is persisted for the next motor callback.
pub fn motor_history_value(
    row: super::AngularMotorRow,
    a: AngularVelocityAccumulator,
    b: AngularVelocityAccumulator,
    substeps: u32,
    seconds: f32,
    substep_seconds: f32,
    inverse_velocity_scale: f32,
    velocity_scale: f32,
    saturation: f32,
) -> f32 {
    let velocity = integration_projection(
        row.axis_a,
        row.axis_b,
        a,
        b,
        substeps,
        inverse_velocity_scale,
        velocity_scale,
    );
    (((substeps as f32 * row.target_velocity + row.initial_impulse) * substep_seconds)
        - velocity * seconds)
        * saturation
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn friction_history_matches_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/angular_friction_history.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(words[i]);
            let vec = |at| std::array::from_fn(|i| v(at + i));
            let a = AngularVelocityAccumulator {
                current: vec(0),
                initial: vec(6),
                inverse_inertia: vec(12),
            };
            let b = AngularVelocityAccumulator {
                current: vec(3),
                initial: vec(9),
                inverse_inertia: vec(15),
            };
            let projection = integration_projection(vec(20), vec(24), a, b, 2, 0.6, 1. / 0.6);
            let actual = (v(18) * (1. / 120.) * v(27) - projection * (1. / 60.)) * 0.75;
            assert!(
                (actual - v(39)).abs() < 0.000002,
                "friction history case{count} {actual} vs {}",
                v(39)
            );
            count += 1;
        }
        assert_eq!(count, 256);
    }
    #[test]
    fn motor_history_matches_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/angular_motor_history.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(words[i]);
            let vec = |at| std::array::from_fn(|i| v(at + i));
            let a = AngularVelocityAccumulator {
                current: vec(0),
                initial: vec(6),
                inverse_inertia: vec(12),
            };
            let b = AngularVelocityAccumulator {
                current: vec(3),
                initial: vec(9),
                inverse_inertia: vec(15),
            };
            let row = super::super::AngularMotorRow {
                axis_a: vec(20),
                effective_mass: v(23),
                axis_b: vec(24),
                initial_impulse: v(27),
                maximum_impulse: v(28),
                minimum_impulse: v(29),
                target_velocity: v(30),
                tau: v(31),
                damping: v(32),
            };
            let actual =
                motor_history_value(row, a, b, 2, 1. / 60., 1. / 120., 0.6, 1. / 0.6, 0.75);
            let expected = v(40);
            assert!(
                (actual - expected).abs() < 0.000002,
                "history case{count} {actual} vs {expected}"
            );
            count += 1;
        }
        assert_eq!(count, 256);
    }
    #[test]
    fn limited_hinge_angle_matches_executable() {
        let mut count = 0;
        for line in include_str!("../../../tests/fixtures/hinge_limit_angles.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let words: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(words[i]);
            let basis = |at| std::array::from_fn(|c| std::array::from_fn(|r| v(at + c * 4 + r)));
            let actual = limit_angle(basis(7), basis(19), v(0) as usize, v(1) as usize, v(2));
            assert!(
                (actual - v(37)).abs() < 0.000002,
                "angle case{count}: {actual} vs {}",
                v(37)
            );
            count += 1;
        }
        assert_eq!(count, 256);
    }
    #[test]
    fn angular_consumers_match_executable() {
        for (kind, fixture) in [
            (
                12,
                include_str!("../../../tests/fixtures/angular_schema_12_solve.txt"),
            ),
            (
                13,
                include_str!("../../../tests/fixtures/angular_schema_13_solve.txt"),
            ),
            (
                14,
                include_str!("../../../tests/fixtures/angular_schema_14_solve.txt"),
            ),
        ] {
            let mut count = 0;
            for line in fixture
                .lines()
                .filter(|l| !l.starts_with('#') && !l.is_empty())
            {
                let words: Vec<u32> = line
                    .split_whitespace()
                    .map(|v| u32::from_str_radix(v, 16).unwrap())
                    .collect();
                let v = |i| f32::from_bits(words[i]);
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
                let (impulse, internal) = match kind {
                    12 => (
                        AngularLockRow {
                            axis_a: vector(20),
                            effective_mass: v(23),
                            axis_b: vector(24),
                            correction: v(27),
                        }
                        .solve(&mut a, &mut b),
                        0.,
                    ),
                    13 => {
                        let mut sum = 0.125;
                        let impulse = AngularLimitRow {
                            axis_a: vector(20),
                            effective_mass: v(23),
                            axis_b: vector(24),
                            correction: v(27),
                            lower: v(28),
                            upper: v(29),
                            tau: v(30),
                            damping: v(31),
                        }
                        .solve(
                            &mut a,
                            &mut b,
                            v(18),
                            0.004166666883975267,
                            &mut sum,
                        );
                        (impulse, 0.)
                    }
                    _ => {
                        let r = AngularFrictionRow {
                            axis_a: vector(20),
                            effective_mass: v(23),
                            axis_b: vector(24),
                            correction: v(27),
                            maximum_impulse: v(28),
                        }
                        .solve(&mut a, &mut b);
                        (r[0], r[1])
                    }
                };
                let output = if kind == 12 { 28 } else { 32 };
                let actual: Vec<f32> = a
                    .current
                    .into_iter()
                    .chain(b.current)
                    .chain([impulse + 0.125, internal])
                    .collect();
                for (i, x) in actual.into_iter().enumerate() {
                    let expected = v(output + i);
                    assert!(
                        (x - expected).abs() <= 0.000002 * expected.abs().max(1.),
                        "schema{kind} case{count} field{i}: {x} vs {expected}"
                    );
                }
                count += 1;
            }
            assert_eq!(count, 256);
        }
    }
}
