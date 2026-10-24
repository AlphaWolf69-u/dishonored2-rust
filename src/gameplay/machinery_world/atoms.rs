//! Atom-stack constraints (ragdoll physics-system constraints and the
//! arkCpntConstraint cone) built by 140c45400 each step and solved in
//! stream order, with their runtime exports carried to the next step.
use super::{xyz, MachineryBody};
use crate::{
    entities::machinery::ConstraintSettings,
    gameplay::constraint_solver::{
        angular::integration_projection,
        atom_stack::{self, AngularLimit, AtomBody, AtomRows, AtomStack, AtomStep, FrictionAtom, FrictionRows, LimitRow},
        ball_socket::SolverVelocities,
        step::{ConstraintParameters, SolverParameters},
        AngularVelocityAccumulator,
    },
    physworld,
};

pub(super) type Transform = atom_stack::Transform;

pub(super) struct AtomJoint {
    pub a: usize,
    pub b: usize,
    pub stack: AtomStack,
    /// An arkCpntConstraint (LOD freeze participant) rather than a
    /// physics-system constraint.
    pub component: bool,
    runtime: Vec<u8>,
    rows: Option<AtomRows>,
    limit_impulses: [f32; 3],
    friction_scales: [f32; 3],
}

const RUNTIME: usize = 0x100;

impl AtomJoint {
    pub fn new(a: usize, b: usize, stack: AtomStack, component: bool) -> Self {
        Self {
            a,
            b,
            stack,
            component,
            runtime: vec![0; RUNTIME],
            rows: None,
            limit_impulses: [0.; 3],
            friction_scales: [1.; 3],
        }
    }
}

/// hkTransform inverse(a) * b (140ae3240's local constraint frame).
pub(super) fn inverse_multiply(a: &Transform, b: &Transform) -> Transform {
    let dot = |u: [f32; 4], v: [f32; 4]| (u[1] * v[1] + u[0] * v[0]) + u[2] * v[2];
    let mut out = [[0.; 4]; 4];
    for c in 0..3 {
        out[c] = [dot(a[0], b[c]), dot(a[1], b[c]), dot(a[2], b[c]), 0.];
    }
    let d: [f32; 4] = std::array::from_fn(|i| if i < 3 { b[3][i] - a[3][i] } else { 0. });
    out[3] = [dot(a[0], d), dot(a[1], d), dot(a[2], d), 0.];
    out
}

/// The constraint setup's per-body record (1412a43e0) for a machinery body.
pub(super) fn atom_body(body: &MachineryBody) -> AtomBody {
    let q = body.motion.pose.orientation;
    let rotation = physworld::quaternion_basis(q);
    AtomBody {
        transform: body.body_transform(),
        inverse_inertia: body.motion.inverse_mass_values(),
        // Principal lanes: the rows of the mass-to-world rotation.
        rows: std::array::from_fn(|r| [rotation[0][r], rotation[1][r], rotation[2][r], 0.]),
        center: body.motion.pose.center,
        world_to_mass: [-q[0], -q[1], -q[2], q[3]],
    }
}

/// 140713850 for LINEAR_CONSTRAINT_FIXED with ANGULAR_CONSTRAINT_CONE:
/// local transforms from both body frames, stabilization, three-axis
/// friction (m_angularFriction >= 0), the twist limit about the cone axis,
/// the cone limit (half the cone angle, angle offset at the end of the
/// runtime block) and the ball socket.
pub(super) fn cone_stack(settings: &ConstraintSettings, a: &MachineryBody, b: &MachineryBody) -> AtomStack {
    let frame: Transform = [
        [settings.basis[0][0], settings.basis[0][1], settings.basis[0][2], 0.],
        [settings.basis[1][0], settings.basis[1][1], settings.basis[1][2], 0.],
        [settings.basis[2][0], settings.basis[2][1], settings.basis[2][2], 0.],
        [settings.pivot[0], settings.pivot[1], settings.pivot[2], 1.],
    ];
    let axis = settings.angular_axis as u8;
    let huge = f32::from_bits(0x7f7f_ffee);
    let friction = (settings.angular_friction >= 0.).then(|| FrictionAtom {
        active: settings.angular_friction != 0.,
        first_axis: 0,
        axes: 3,
        torque: settings.angular_friction,
    });
    let minimum = settings.angular_minimum.min(settings.angular_maximum);
    let maximum = settings.angular_maximum;
    // Runtime words: friction 2 per axis, the twist/cone pair 2, ball socket 3.
    let words = friction.map_or(0, |f| f.axes as u16 * 2) + 2 + 3;
    AtomStack {
        transform_a: inverse_multiply(&a.body_transform(), &frame),
        transform_b: inverse_multiply(&b.body_transform(), &frame),
        stabilization: Some([huge, huge, f32::from_bits(0x5f7f_fff0)]),
        disabled_motors: false,
        friction,
        twist: Some(AngularLimit {
            enabled: maximum - minimum < std::f32::consts::TAU,
            axis,
            reference: (axis + 1) % 3,
            mode: 0,
            memory_offset: 0,
            minimum,
            maximum,
            tau: settings.angular_limits_tau_factor,
            damping: 1.,
        }),
        cone: Some(AngularLimit {
            enabled: true,
            axis,
            reference: axis,
            mode: 0,
            memory_offset: words * 8,
            minimum: -100.,
            maximum: settings.angular_cone * 0.5,
            tau: settings.angular_limits_tau_factor,
            damping: 1.,
        }),
        planes: None,
        limited: false,
        breach_impulse: huge,
        inertia_stabilization: 0.,
    }
}

/// Build info from the solver step (1412a4360, mapped through the machinery
/// solver's live-sampled constraint coefficients).
pub(super) fn atom_step(step: &SolverParameters, query: &ConstraintParameters) -> AtomStep {
    AtomStep {
        torque_seconds: step.substep_seconds,
        inverse_step: step.inverse_substep_seconds,
        offset_rate: step.seconds,
        export_seconds: query.inverse_step_tau,
        damping: query.stabilized_damping,
        tau: query.stabilized_tau,
        limit_damping: query.stabilized_damping,
    }
}

fn accumulator(v: &SolverVelocities) -> AngularVelocityAccumulator {
    AngularVelocityAccumulator {
        current: xyz(v.angular),
        initial: xyz(v.initial_angular),
        inverse_inertia: xyz(v.inverse_mass),
    }
}

impl AtomJoint {
    pub fn build(&mut self, bodies: &[MachineryBody], step: &AtomStep) {
        let a = atom_body(&bodies[self.a]);
        let b = atom_body(&bodies[self.b]);
        self.rows = Some(atom_stack::build(&self.stack, &a, &b, step, &mut self.runtime));
        self.limit_impulses = [0.; 3];
        self.friction_scales = [1.; 3];
    }
    pub fn skip(&mut self) {
        self.rows = None;
    }

    /// One substep of the joint's rows in stream order.
    pub fn solve(&mut self, velocities: &mut [SolverVelocities], step: &SolverParameters) {
        let Some(rows) = self.rows else {
            return;
        };
        let mut a = velocities[self.a];
        let mut b = velocities[self.b];
        let mut angular_a = accumulator(&a);
        let mut angular_b = accumulator(&b);
        if let Some(FrictionRows::Rows(friction)) = rows.friction {
            for (k, row) in friction.iter().enumerate() {
                if let Some(row) = row {
                    self.friction_scales[k] = row.solve(&mut angular_a, &mut angular_b)[1];
                }
            }
        }
        for (k, limit) in [rows.twist, rows.cone, rows.planes].into_iter().enumerate() {
            if let Some(LimitRow::Limit(row)) = limit {
                row.solve(
                    &mut angular_a,
                    &mut angular_b,
                    step.velocity_scale,
                    step.substep_seconds,
                    &mut self.limit_impulses[k],
                );
            }
        }
        a.angular = [angular_a.current[0], angular_a.current[1], angular_a.current[2], 0.];
        b.angular = [angular_b.current[0], angular_b.current[1], angular_b.current[2], 0.];
        rows.pivot.solve(&mut a, &mut b, step.substep_seconds, step.velocity_scale);
        velocities[self.a] = a;
        velocities[self.b] = b;
    }

    /// 14129c3f0 schema 0xd/0xe exports into the runtime block: the limit's
    /// impulse and solver value, and each friction axis' value.
    pub fn export(&mut self, velocities: &[SolverVelocities], step: &SolverParameters) {
        let Some(rows) = self.rows else {
            return;
        };
        let a = accumulator(&velocities[self.a]);
        let b = accumulator(&velocities[self.b]);
        let projection = |ja, jb| {
            integration_projection(ja, jb, a, b, step.substeps, step.inverse_velocity_scale, step.velocity_scale)
        };
        let mut write = |at: usize, value: f32| self.runtime[at..at + 4].copy_from_slice(&value.to_le_bytes());
        if let Some(FrictionRows::Rows(friction)) = rows.friction {
            for (k, row) in friction.iter().enumerate() {
                if let Some(row) = row {
                    let value = (step.velocity_scale * step.substep_seconds * row.correction
                        - projection(row.axis_a, row.axis_b) * step.seconds)
                        * self.friction_scales[k];
                    write(rows.friction_slot + k * 0x10 + 4, value);
                }
            }
        }
        for (k, limit) in [rows.twist, rows.cone, rows.planes].into_iter().enumerate() {
            if let Some(LimitRow::Limit(row)) = limit {
                let slot = rows.limit_slots[k];
                write(slot, self.limit_impulses[k]);
                write(
                    slot + 4,
                    row.correction * step.substep_seconds - projection(row.axis_a, row.axis_b) * step.seconds,
                );
            }
        }
    }
}
