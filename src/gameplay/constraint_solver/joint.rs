//! Shared numerical hinge/cone inputs. Hosts borrow their real motion data;
//! neither the ordinary physics world nor legacy machinery creates a body copy.
use super::{
    self as solver,
    angular::{hinge_limit_row, AngularFrictionRow, AngularLimitRow},
    ball_socket::{
        rotate_direction, AnchorFrame, BallSocketBody, BallSocketBuild, BallSocketRow,
        SolverVelocities,
    },
    step::{ConstraintParameters, SolverParameters},
    AngularLockRow, AngularMotorRow, AngularVelocityAccumulator, Basis, ConstraintBody,
    HingeHistory, HingeRowInput,
};
use crate::{
    entities::machinery::ConstraintSettings,
    gameplay::{
        machinery::{AngularMotorHistory, ConstraintMotor, MotorStep},
        rigid_body::BodyPose,
    },
    physworld,
};

#[derive(Clone, Copy)]
pub(crate) struct BodyInput<'a> {
    pub pose: &'a BodyPose,
    pub inverse_mass: &'a [u16; 4],
    pub body_transform: &'a super::atom_stack::Transform,
}
impl BodyInput<'_> {
    pub(crate) fn inverse_mass_values(self) -> [f32; 4] {
        self.inverse_mass
            .map(|v| f32::from_bits(u32::from(v) << 16))
    }
    fn constraint_body(self) -> ConstraintBody {
        let mass = self.inverse_mass_values();
        ConstraintBody {
            principal: transpose(physworld::quaternion_basis(self.pose.orientation)),
            center: xyz(self.pose.center),
            inverse_inertia: xyz(mass),
            inverse_mass: mass[3],
        }
    }
    fn anchor(self) -> BallSocketBody {
        BallSocketBody {
            frame: AnchorFrame {
                center: self.pose.center,
                world_to_mass: inverse(self.pose.orientation),
            },
            inverse_mass: self.inverse_mass_values(),
        }
    }
}
fn vector(v: [f32; 3]) -> [f32; 4] {
    [v[0], v[1], v[2], 0.]
}
fn xyz(v: [f32; 4]) -> [f32; 3] {
    [v[0], v[1], v[2]]
}
fn inverse(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}
fn transform(q: [f32; 4], basis: Basis) -> Basis {
    let rotation = physworld::quaternion_basis(q);
    basis.map(|axis| {
        std::array::from_fn(|i| {
            if i < 3 {
                (axis[1] * rotation[1][i] + axis[0] * rotation[0][i]) + axis[2] * rotation[2][i]
            } else {
                0.
            }
        })
    })
}
fn transpose(b: Basis) -> Basis {
    std::array::from_fn(|c| std::array::from_fn(|r| if r < 3 { b[r][c] } else { 0. }))
}
pub(crate) struct Joint {
    pub(crate) settings: ConstraintSettings,
    /// arkCpntConstraint +0x8c bit 1: the motor 1407149f0 disabled on freeze.
    pub(crate) motor_suspended: bool,
    pub(crate) a: usize,
    pub(crate) b: usize,
    pub(crate) local_a: Basis,
    pub(crate) local_b: Basis,
    pub(crate) lever_a: [f32; 3],
    pub(crate) lever_b: [f32; 3],
    pub(crate) motor: Option<ConstraintMotor>,
    pub(crate) measurement: AngularMotorHistory,
    pub(crate) hinge: HingeHistory,
    pub(crate) previous_motor_value: f32,
    pub(crate) previous_limit_angle: f32,
    pub(crate) previous_friction_value: f32,
}
pub(crate) struct JointRows {
    pub(crate) angular: [AngularLockRow; 2],
    pub(crate) motor: Option<AngularMotorRow>,
    pub(crate) limit: Option<AngularLimitRow>,
    pub(crate) friction: Option<AngularFrictionRow>,
    pub(crate) pivot: BallSocketRow,
    pub(crate) motor_scale: f32,
    pub(crate) friction_scale: f32,
    pub(crate) limit_impulse: f32,
}
impl Joint {
    pub(crate) fn new(
        settings: ConstraintSettings,
        a: usize,
        b: usize,
        a_body: BodyInput<'_>,
        b_body: BodyInput<'_>,
    ) -> Self {
        let frame = settings.basis.map(vector);
        let local = |body: BodyInput<'_>| transform(inverse(body.pose.orientation), frame);
        let lever = |body: BodyInput<'_>| {
            rotate_direction(
                inverse(body.pose.orientation),
                std::array::from_fn(|i| settings.pivot[i] - body.pose.center[i]),
            )
        };
        let local_a = local(a_body);
        let local_b = local(b_body);
        let lever_a = lever(a_body);
        let lever_b = lever(b_body);
        let motor = settings.motor.map(ConstraintMotor::from_settings);
        // The measurement history is relative BODY rotation, not the mass
        // principal axes. The constant mass frames cancel in relative changes;
        // initialize to the actual relative constraint frame before first step.
        let mut measurement = AngularMotorHistory::default();
        measurement.measure(frame, frame, xyz(local_a[settings.angular_axis]), 1., 1.);
        Self {
            settings,
            motor_suspended: false,
            a,
            b,
            local_a,
            local_b,
            lever_a,
            lever_b,
            motor,
            measurement,
            hinge: HingeHistory::default(),
            previous_motor_value: 0.,
            previous_limit_angle: 0.,
            previous_friction_value: 0.,
        }
    }
    pub(crate) fn build(
        &mut self,
        a: BodyInput<'_>,
        b: BodyInput<'_>,
        step: &SolverParameters,
        query: &ConstraintParameters,
        entity_seconds: f32,
    ) -> JointRows {
        let constraint_tau = query.stabilized_tau;
        let constraint_damping = query.stabilized_damping;
        let inverse_step_tau = query.inverse_step_tau;
        let fa = transform(a.pose.orientation, self.local_a);
        let fb = transform(b.pose.orientation, self.local_b);
        let ca = a.constraint_body();
        let cb = b.constraint_body();
        let axis = self.settings.angular_axis;
        let perpendicular = (axis + 1) % 3;
        let other = (axis + 2) % 3;
        let angular = [
            solver::angular_lock_row(
                ca,
                cb,
                xyz(fa[axis]),
                xyz(fb[other]),
                xyz(fb[perpendicular]),
                inverse_step_tau,
                step.damping,
            ),
            solver::angular_lock_row(
                ca,
                cb,
                xyz(fa[axis]),
                xyz(fb[perpendicular]).map(|v| -v),
                xyz(fb[other]),
                inverse_step_tau,
                step.damping,
            ),
        ];
        // Actual owning entity seconds; the caller supplies its canonical timespace delta.
        let scale = entity_seconds / step.seconds;
        let measured = if entity_seconds > 0. {
            self.measurement.measure(
                fa,
                fb,
                std::array::from_fn(|i| if i == axis { 1. } else { 0. }),
                entity_seconds,
                scale,
            )
        } else {
            0.
        };
        let motor = self
            .motor
            .as_mut()
            .filter(|_| self.settings.started)
            .map(|motor| {
                solver::hinge_motor_row(
                    motor,
                    &mut self.hinge,
                    HingeRowInput {
                        axis,
                        frame_a: fa,
                        frame_b: fb,
                        principal_a: ca.principal,
                        principal_b: cb.principal,
                        inverse_inertia_a: ca.inverse_inertia,
                        inverse_inertia_b: cb.inverse_inertia,
                        target_angle: self.settings.target_position,
                        step: MotorStep {
                            seconds: step.seconds,
                            inverse_seconds: step.inverse_seconds,
                            time_scale: scale,
                            measured_velocity: measured,
                            position_error: 0.,
                            previous_position_error: 0.,
                            previous_solver_value: self.previous_motor_value,
                        },
                        warm_start_scale: step.inverse_substep_seconds,
                        impulse_seconds: step.substep_seconds,
                    },
                )
            });
        let limit =
            (self.settings.angular_constraint == "ANGULAR_CONSTRAINT_LIMITED_HINGE").then(|| {
                hinge_limit_row(
                    ca,
                    cb,
                    fa,
                    fb,
                    axis,
                    self.settings.angular_minimum,
                    self.settings.angular_maximum,
                    &mut self.previous_limit_angle,
                    step.inverse_substep_seconds,
                    constraint_tau,
                    constraint_damping,
                )
            });
        let friction = if self.settings.angular_friction >= 0. && motor.is_none() {
            let mut ja = solver::project(ca.principal, xyz(fa[axis]));
            let jb = solver::project(cb.principal, xyz(fa[axis]).map(|v| -v));
            let inv: [f32; 3] = std::array::from_fn(|i| {
                ja[i] * ja[i] * ca.inverse_inertia[i] + jb[i] * jb[i] * cb.inverse_inertia[i]
            });
            let effective_mass =
                solver::reciprocal((inv[1] + inv[0]) + inv[2] + f32::from_bits(0x34000000));
            ja[0] = solver::schema_axis(ja[0], 14);
            Some(AngularFrictionRow {
                axis_a: ja,
                axis_b: jb,
                effective_mass,
                correction: inverse_step_tau * self.previous_friction_value,
                maximum_impulse: self.settings.angular_friction * step.substep_seconds,
            })
        } else {
            None
        };
        let huge = f32::from_bits(0x7f7fffee);
        let pivot = BallSocketRow::build(BallSocketBuild {
            a: a.anchor(),
            b: b.anchor(),
            lever_a: self.lever_a,
            lever_b: self.lever_b,
            inverse_step: step.inverse_substep_seconds,
            tau: constraint_tau,
            damping: step.damping,
            mass_stabilization: 0.,
            maximum_force: huge,
            maximum_impulse: huge,
            angular_impulse_limit: huge,
        });
        JointRows {
            angular,
            motor,
            limit,
            friction,
            pivot,
            motor_scale: 1.,
            friction_scale: 1.,
            limit_impulse: 0.,
        }
    }
    pub(crate) fn export(
        &mut self,
        rows: &JointRows,
        velocities: &[SolverVelocities],
        step: &SolverParameters,
    ) {
        let accumulator = |index: usize| {
            let v = velocities[index];
            AngularVelocityAccumulator {
                current: xyz(v.angular),
                initial: xyz(v.initial_angular),
                inverse_inertia: xyz(v.inverse_mass),
            }
        };
        let (a, b) = (self.a, self.b);
        let projection = |ja, jb| {
            solver::angular::integration_projection(
                ja,
                jb,
                accumulator(a),
                accumulator(b),
                step.substeps,
                step.inverse_velocity_scale,
                step.velocity_scale,
            )
        };
        if let Some(row) = rows.friction {
            self.previous_friction_value =
                (step.velocity_scale * step.substep_seconds * row.correction
                    - projection(row.axis_a, row.axis_b) * step.seconds)
                    * rows.friction_scale;
        }
        if let Some(row) = rows.limit {
            self.previous_limit_angle = -(row.correction * step.substep_seconds
                - projection(row.axis_a, row.axis_b) * step.seconds);
        }
        if let Some(row) = rows.motor {
            let accumulator = |index: usize| {
                let v = velocities[index];
                AngularVelocityAccumulator {
                    current: xyz(v.angular),
                    initial: xyz(v.initial_angular),
                    inverse_inertia: xyz(v.inverse_mass),
                }
            };
            self.previous_motor_value = solver::angular::motor_history_value(
                row,
                accumulator(self.a),
                accumulator(self.b),
                step.substeps,
                step.seconds,
                step.substep_seconds,
                step.inverse_velocity_scale,
                step.velocity_scale,
                rows.motor_scale,
            );
        }
    }
}
pub(crate) fn solve_hinge(
    joint: &Joint,
    rows: &mut JointRows,
    velocities: &mut [SolverVelocities],
    step: &SolverParameters,
    velocity_fraction: f32,
) {
    let mut a = velocities[joint.a];
    let mut b = velocities[joint.b];
    let mut angular_a = AngularVelocityAccumulator {
        current: xyz(a.angular),
        initial: xyz(a.initial_angular),
        inverse_inertia: xyz(a.inverse_mass),
    };
    let mut angular_b = AngularVelocityAccumulator {
        current: xyz(b.angular),
        initial: xyz(b.initial_angular),
        inverse_inertia: xyz(b.inverse_mass),
    };
    if let Some(row) = rows.motor {
        rows.motor_scale = row.solve(
            &mut angular_a,
            &mut angular_b,
            step.velocity_scale,
            velocity_fraction,
        )[1];
    }
    if let Some(row) = rows.friction {
        rows.friction_scale = row.solve(&mut angular_a, &mut angular_b)[1];
    }
    if let Some(row) = rows.limit {
        row.solve(
            &mut angular_a,
            &mut angular_b,
            step.velocity_scale,
            step.substep_seconds,
            &mut rows.limit_impulse,
        );
    }
    for row in rows.angular {
        row.solve(&mut angular_a, &mut angular_b);
    }
    a.angular = vector(angular_a.current);
    b.angular = vector(angular_b.current);
    rows.pivot
        .solve(&mut a, &mut b, step.substep_seconds, step.velocity_scale);
    velocities[joint.a] = a;
    velocities[joint.b] = b;
}
