//! Ordinary one-/two-motion contact Jacobian projection (140c583a0).
use super::{ContactRows, ContactState, FrictionBlock, FrictionLimit, ImpulseLimit, NormalRow};
use crate::gameplay::constraint_solver::{
    inverse_length, project, reciprocal, step::SolverParameters, ConstraintBody,
};
use crate::gameplay::motion::Motion;

#[derive(Clone, Copy, Debug, Default)]
pub struct ContactPoint {
    /// The actual manifold's world point on B; pointA = pointB + normal*distance.
    pub position: [f32; 3],
    pub distance: f32,
}

impl ConstraintBody {
    /// Same principal-mass projection used by the existing constraint owner.
    pub fn from_motion(motion: &Motion) -> Self {
        let basis = crate::physworld::quaternion_basis(motion.pose.orientation);
        let inverse = motion
            .inverse_mass
            .map(|value| f32::from_bits(u32::from(value) << 16));
        Self {
            principal: std::array::from_fn(|row| {
                [basis[0][row], basis[1][row], basis[2][row], 0.0]
            }),
            center: std::array::from_fn(|i| motion.pose.center[i]),
            inverse_inertia: std::array::from_fn(|i| inverse[i]),
            inverse_mass: inverse[3],
        }
    }
}

/// Mutable data belong to the world's actual manifold/point correspondence.
/// Keeping this outside the row builder avoids a second contact cache owner.
#[derive(Clone, Copy, Debug, Default)]
pub struct ContactCache {
    pub position_error: [f32; 4],
    pub distance_offset: [f32; 4],
    pub normal_impulses: [f32; 4],
    pub friction_velocity: [f32; 4],
    pub friction_factor: f32,
}

impl ContactCache {
    /// c4eb10's uncached temporary block: only the first force estimate is
    /// seeded. The remaining manifold arrays and displacement start at zero.
    pub fn cold(motions: [&Motion; 2], step: &SolverParameters) -> Self {
        let [a, b] = motions.map(ConstraintBody::from_motion);
        let mut cache = Self::default();
        let mass = a.inverse_mass + b.inverse_mass;
        if mass != 0.0 {
            cache.normal_impulses[0] =
                super::reciprocal_estimate(mass) * (step.gravity_length * step.substep_seconds);
        }
        cache
    }

    /// c58110 writes solved impulses to the actual manifold and stores the
    /// friction result as an upper-half float, not the full temporary scalar.
    pub fn export(&mut self, state: &ContactState) {
        self.normal_impulses = state.normal_impulses;
        self.friction_factor = f32::from_bits(state.friction_factor.to_bits() & 0xffff_0000);
    }
}

/// Correction producers c4eb10/c54190. A persistent contact's flag80 comes
/// from its actual cache; it cannot be inferred from the contacted shape.
pub fn recovery_coefficients(cached_flags: Option<u8>, combined_body_flags: u32) -> (f32, f32) {
    if combined_body_flags & (1 << 28) != 0 {
        return (0.0, 0.0);
    }
    match cached_flags {
        None => (100.0, f32::from_bits(0xbd4c_cccd)),
        Some(flags) if flags & 0x80 != 0 => (1.0, f32::from_bits(0xbd4c_cccd)),
        Some(_) => (0.0, 0.0),
    }
}

pub struct ContactBuild<'a> {
    pub motions: [&'a Motion; 2],
    pub normal: [f32; 3],
    pub points: &'a [ContactPoint],
    pub step: &'a SolverParameters,
    /// Native cached base + static delta * prior friction factor, already
    /// decoded from the actual pair's half-float property words.
    pub base_friction: f32,
    pub friction_delta: f32,
    /// Native cached-pair flags+0a bit13. Uses current solved normal impulse
    /// when the previous-frame force estimate is zero.
    pub immediate_friction: bool,
    /// c4eb10 uses100 and-.05; persistent contact setup supplies its actual
    /// source-specific correction coefficients instead of guessed tuning.
    pub recovery_limit: f32,
    pub recovery_factor: f32,
    pub impulse_limit: Option<ImpulseLimit>,
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - b[1] * a[2],
        a[2] * b[0] - b[2] * a[0],
        a[0] * b[1] - b[0] * a[1],
    ]
}
fn relative(point: [f32; 3], body: ConstraintBody) -> [f32; 3] {
    std::array::from_fn(|i| point[i] - body.center[i])
}
fn angular(body: ConstraintBody, lever: [f32; 3], direction: [f32; 3]) -> [f32; 3] {
    project(body.principal, cross(lever, direction))
}
fn world_angular(body: ConstraintBody, angular: [f32; 4]) -> [f32; 3] {
    std::array::from_fn(|i| {
        (angular[1] * body.principal[i][1] + angular[0] * body.principal[i][0])
            + angular[2] * body.principal[i][2]
    })
}
fn weighted(a: [f32; 3], b: [f32; 3], bodies: [ConstraintBody; 2]) -> [f32; 3] {
    std::array::from_fn(|i| {
        (a[i] * a[i]) * bodies[0].inverse_inertia[i] + (b[i] * b[i]) * bodies[1].inverse_inertia[i]
    })
}
fn inverse(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        reciprocal(value)
    }
}

pub fn build(
    input: &ContactBuild<'_>,
    cache: &mut ContactCache,
) -> Result<ContactRows, &'static str> {
    if input.points.is_empty() || input.points.len() > 4 {
        return Err("contact Jacobian requires the actual one-to-four-point manifold");
    }
    let bodies = input.motions.map(ConstraintBody::from_motion);
    let [a, b] = bodies;
    let [va, vb] = input.motions;
    let linear_mass = a.inverse_mass + b.inverse_mass;
    let step = input.step;
    let mut output = ContactRows {
        // c583a0: ordinary response type0, contact Jacobian flag1. Its
        // impulse-clipping flag2 is set only below the source's max sentinel.
        stream_flags: 1 | u8::from(
            input
                .impulse_limit
                .is_some_and(|limit| limit.maximum < f32::from_bits(0x7f7f_0000)),
        ) * 2,
        stream_type: 0,
        normal: input.normal,
        reconstruction: super::reconstruction::ManifoldHeader::build(input),
        rows: [NormalRow::default(); 4],
        count: input.points.len() as u8,
        retract_normal_impulses: true,
        impulse_limit: input.impulse_limit,
        friction: None,
    };
    let previous_force = match input.points.len() {
        4 => {
            (cache.normal_impulses[3] + cache.normal_impulses[1])
                + cache.normal_impulses[2]
                + cache.normal_impulses[0]
        }
        count => cache.normal_impulses[..count]
            .iter()
            .copied()
            .fold(0.0, |sum, v| sum + v),
    };
    for (index, point) in input.points.iter().enumerate() {
        let angular_a = angular(a, relative(point.position, a), input.normal);
        let angular_b = angular(b, relative(point.position, b), input.normal.map(|v| -v));
        let inertia = weighted(angular_a, angular_b, bodies);
        // c583a0's fourth lane groups Z+X, then Y+linear mass.
        let inverse_mass = (inertia[0] + inertia[2]) + (inertia[1] + linear_mass);
        let velocity: [f32; 3] = std::array::from_fn(|i| {
            angular_a[i] * va.angular_velocity[i]
                + angular_b[i] * vb.angular_velocity[i]
                + input.normal[i] * (va.linear_velocity[i] - vb.linear_velocity[i])
        });
        let previous_error = cache.position_error[index];
        let previous_offset = cache.distance_offset[index];
        let predicted = if vb.cell == 0 {
            (((velocity[1] + velocity[0]) + velocity[2]) * step.seconds) + previous_error
        } else {
            previous_error
        };
        let cap = input.recovery_limit * step.seconds;
        let distance = point.distance;
        let mut error = distance;
        if previous_offset < -f32::from_bits(0x3400_0000) || distance <= predicted - cap {
            let correction = if vb.cell == 0 {
                super::min(previous_offset * input.recovery_factor, cap)
            } else {
                super::max(super::min(distance * input.recovery_factor, cap), 0.0)
            };
            let delta = distance - previous_offset;
            let release = super::min(predicted, 0.0) - delta;
            let release = if correction * 2.0 + cap < release {
                release
            } else {
                0.0
            };
            cache.distance_offset[index] = super::min(
                super::max(
                    (previous_offset + correction) - release,
                    distance - correction,
                ),
                -f32::from_bits(0x3400_0000),
            );
            error = (delta - correction) + release;
        }
        cache.position_error[index] = error;
        output.rows[index] = NormalRow {
            angular_a,
            angular_b,
            effective_mass: inverse(inverse_mass) * step.damping,
            rhs: -step.inverse_step_tau * error,
        };
    }
    let coefficient = input.base_friction + input.friction_delta * cache.friction_factor;
    if coefficient == 0.0 || (previous_force == 0.0 && !input.immediate_friction) {
        return Ok(output);
    }
    let normal = input.normal;
    let mut center = [0.0; 3];
    let mut distance = 0.0;
    for point in input.points {
        for i in 0..3 {
            center[i] += point.position[i];
        }
        distance += point.distance;
    }
    let count_inverse = reciprocal(input.points.len() as f32);
    center = center.map(|v| v * count_inverse);
    distance *= count_inverse;
    let difference: [f32; 3] = std::array::from_fn(|i| center[i] - input.points[0].position[i]);
    let radius_squared = dot(difference, difference) + f32::from_bits(0x2880_0000);
    let radius = super::inverse_length_estimate(radius_squared) * radius_squared;
    let tangent_seed = if normal[1] * normal[1] + normal[0] * normal[0]
        < normal[0] * normal[0] + normal[2] * normal[2]
    {
        [-normal[2], 0.0, normal[0]]
    } else {
        [-normal[1], normal[0], 0.0]
    };
    let length_inverse = inverse_length(dot(tangent_seed, tangent_seed));
    let tangent0 = tangent_seed.map(|v| v * length_inverse);
    let tangent1 = cross(tangent0, normal);
    let tangent = [tangent0, tangent1];
    let lever_a_initial = relative(center, a);
    let lever_b = relative(center, b);
    let lever_a: [f32; 3] = std::array::from_fn(|i| distance * normal[i] + lever_a_initial[i]);
    let angular_a = tangent.map(|direction| angular(a, lever_a, direction));
    let angular_b = tangent.map(|direction| angular(b, lever_b, direction.map(|v| -v)));
    let mass = std::array::from_fn::<_, 2, _>(|axis| {
        let inertia = weighted(angular_a[axis], angular_b[axis], bodies);
        (inertia[0] + inertia[2]) + (inertia[1] + linear_mass)
    });
    let cross_mass: [f32; 3] = std::array::from_fn(|i| {
        linear_mass * tangent[1][i] * tangent[0][i]
            + angular_a[0][i] * angular_a[1][i] * a.inverse_inertia[i]
            + angular_b[0][i] * angular_b[1][i] * b.inverse_inertia[i]
    });
    let coupling = (cross_mass[1] + cross_mass[0]) + cross_mass[2];
    let scaled = mass.map(|value| step.damping * value);
    let determinant = scaled[0] * scaled[1] - coupling * coupling;
    let scale = inverse(determinant) * step.damping;
    let spin_a = project(a.principal, normal);
    let spin_b = project(b.principal, normal.map(|v| -v));
    let spin_inertia = weighted(spin_a, spin_b, bodies);
    let spin_mass = (spin_inertia[1] + spin_inertia[0]) + spin_inertia[2];
    let angular_world_a = world_angular(a, va.angular_velocity);
    let angular_world_b = world_angular(b, vb.angular_velocity);
    let rotational_a = cross(angular_world_a, lever_a_initial);
    let rotational_b = cross(angular_world_b, lever_b);
    let relative_velocity: [f32; 3] = std::array::from_fn(|i| {
        (((rotational_a[i] + (va.linear_velocity[i] - vb.linear_velocity[i])) - rotational_b[i])
            * step.seconds
            + cache.friction_velocity[i])
            * cache.friction_factor
    });
    let normal_velocity = dot(normal, relative_velocity);
    let relative_tangent: [f32; 3] =
        std::array::from_fn(|i| relative_velocity[i] - normal_velocity * normal[i]);
    let spin_velocity = dot(
        std::array::from_fn(|i| angular_world_a[i] - angular_world_b[i]),
        normal,
    );
    let spin_error =
        (spin_velocity * step.seconds + cache.friction_velocity[3]) * cache.friction_factor;
    cache.friction_velocity = [
        relative_tangent[0],
        relative_tangent[1],
        relative_tangent[2],
        spin_error,
    ];
    output.friction = Some(FrictionBlock {
        tangent,
        angular_a,
        angular_b,
        rhs: tangent.map(|direction| dot(relative_tangent, direction) * step.minimum_correction),
        effective_mass: [scale * scaled[1], scale * scaled[0], -coupling * scale],
        spin_a,
        spin_b,
        spin_rhs: spin_error * step.minimum_correction,
        spin_effective_mass: inverse(spin_mass) * inverse(radius) * step.damping,
        radius,
        limit: if previous_force == 0.0 {
            FrictionLimit::NormalImpulse(coefficient)
        } else {
            FrictionLimit::Fixed(
                ((previous_force * step.inverse_substeps) * step.inverse_iterations) * coefficient,
            )
        },
    });
    Ok(output)
}
