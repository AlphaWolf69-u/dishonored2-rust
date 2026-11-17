//! Trigger validation cb9c90 and its c4e5c0/d0f9e0 motion projections.
//! Reference data belong to the real manifold; rows/motions remain borrowed.
use crate::gameplay::{
    constraint_solver::{
        ball_socket::rotate_direction,
        contact::{ContactPoint, ContactRows},
        inverse_length, reciprocal,
        step::SolverParameters,
    },
    motion::Motion,
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct ValidationReference {
    normal: [f32; 4],
    packed_normal: [u16; 3],
    selected_b: bool,
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[1] * b[1] + a[0] * b[0]) + a[2] * b[2]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}
fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}
impl ValidationReference {
    /// c583a0 header+20/+30. Capture before integration, at the same time
    /// the actual row owner builds its angular coefficients. `b_dynamic`
    /// comes from the body's actual nonzero motion ID: Rust's per-cell
    /// solver index zero is a valid dynamic entry, unlike the native anchor0.
    pub(crate) fn capture(
        normal: [f32; 3],
        points: &[ContactPoint],
        motions: [&Motion; 2],
        b_dynamic: bool,
    ) -> Result<Self, String> {
        if points.is_empty() || points.len() > 4 {
            return Err("trigger reference requires its actual one-to-four contact points".into());
        }
        let selected = motions[usize::from(b_dynamic)];
        let relative = std::array::from_fn(|i| points[0].position[i] - selected.pose.center[i]);
        let offset = dot(relative, normal);
        let plane_normal = if points.len() < 3 {
            normal
        } else {
            let a = std::array::from_fn(|i| points[1].position[i] - points[0].position[i]);
            let b = std::array::from_fn(|i| points[2].position[i] - points[0].position[i]);
            let cross = cross(a, b);
            let inverse = inverse_length(dot(cross, cross));
            cross.map(|value| value * inverse)
        };
        // c583a0/c4e5c0's biased unit-vector packing; constants read at
        //1423ccf10/1423ccf20 and141f669c0, not an approximate snorm codec.
        let packed_normal = plane_normal.map(|value| {
            let scaled = crate::float::truncate_i32(value * f32::from_bits(0x4eea6000));
            ((scaled as u32).wrapping_add(0x80000000) >> 16) as u16
        });
        Ok(Self {
            normal: [normal[0], normal[1], normal[2], offset],
            packed_normal,
            selected_b: b_dynamic,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct Reconstructed {
    count: usize,
    normal: [f32; 3],
    points: [[f32; 4]; 4],
    distances: [f32; 4],
}
/// c4e5c0: reconstruct the real Jacobian's reference plane/points after the
/// motions integrate. Rebuilding from retained collision points would bypass
/// the source's changed angular rows and its reference-normal quantization.
fn reconstruct(
    reference: &ValidationReference,
    rows: &ContactRows,
    motions: [&Motion; 2],
    step: &SolverParameters,
) -> Reconstructed {
    let normal = rows.normal;
    let selected = motions[usize::from(reference.selected_b)];
    let sign = if reference.selected_b { -1. } else { 1. };
    let mut plane_normal = reference.packed_normal.map(|value| {
        let signed = 0x80000000u32.wrapping_add(u32::from(value) << 16) as i32;
        signed as f32 * f32::from_bits(0x300bcf65)
    });
    if plane_normal == [0.; 3] {
        plane_normal = normal;
    }
    let mut out = Reconstructed {
        count: usize::from(rows.count).min(4),
        normal,
        points: [[0.; 4]; 4],
        distances: [0.; 4],
    };
    let mut first = [0.; 3];
    for index in 0..out.count {
        let row = rows.rows[index];
        let angular = if reference.selected_b {
            row.angular_b
        } else {
            row.angular_a
        };
        let angular = rotate_direction(selected.pose.orientation, angular);
        let mut lever = cross(normal, angular).map(|value| value * sign);
        let coordinate = if index == 0 {
            for i in 0..3 {
                lever[i] = reference.normal[3] * normal[i] + lever[i];
            }
            first = lever;
            reference.normal[3]
        } else {
            let projection = dot(normal, plane_normal);
            let projection = if projection.abs() < f32::EPSILON {
                1.
            } else {
                projection
            };
            let delta = std::array::from_fn(|i| first[i] - lever[i]);
            let correction = reciprocal(projection) * dot(delta, plane_normal);
            for i in 0..3 {
                lever[i] = normal[i] * correction + lever[i];
            }
            correction
        };
        for i in 0..3 {
            out.points[index][i] = (lever[i] + (0. - step.seconds) * selected.linear_velocity[i])
                + selected.pose.center[i];
        }
        out.points[index][3] = (coordinate * reference.normal[3]
            + (0. - step.seconds) * selected.linear_velocity[3])
            + selected.pose.center[3];
        out.distances[index] = row.rhs * (0. - step.velocity_scale * step.substep_seconds);
    }
    out
}

/// d0f9e0 followed by cb9bf0: tangent-reproject the reconstructed points,
/// evaluate both actual current angular velocities there, then predict the
/// normal distances using the whole step's seconds (+20, not substep+40).
fn predicted(frame: &Reconstructed, motions: [&Motion; 2], step: &SolverParameters) -> [f32; 4] {
    let relative: [f32; 3] =
        std::array::from_fn(|i| motions[0].linear_velocity[i] - motions[1].linear_velocity[i]);
    let normal_velocity = dot(frame.normal, relative);
    let shift: [f32; 3] =
        std::array::from_fn(|i| (relative[i] - frame.normal[i] * normal_velocity) * step.seconds);
    let angular = motions.map(|motion| {
        rotate_direction(
            motion.pose.orientation,
            motion.angular_velocity[..3].try_into().unwrap(),
        )
    });
    std::array::from_fn(|index| {
        let point: [f32; 3] = std::array::from_fn(|i| frame.points[index][i] + shift[i]);
        let levers =
            motions.map(|motion| std::array::from_fn(|i| point[i] - motion.pose.center[i]));
        let a = cross(angular[0], levers[0]);
        let b = cross(angular[1], levers[1]);
        let delta = std::array::from_fn(|i| a[i] - b[i]);
        let velocity = dot(frame.normal, delta) + normal_velocity;
        velocity * step.seconds + frame.distances[index]
    })
}

/// cb9c90. A result below one is the source ce9b00 trigger-validation Enter2
/// gate. `threshold` is that consumer's actual vector (zero for trigger
/// validation); this is independent of cf1fe0's impulse-limit-exceeded flag.
pub(crate) fn crossing_fraction(
    reference: &ValidationReference,
    rows: &ContactRows,
    motions: [&Motion; 2],
    step: &SolverParameters,
    threshold: [f32; 4],
) -> Result<f32, String> {
    if rows.count > 4 {
        return Err("trigger Jacobian has more than four source rows".into());
    }
    let frame = reconstruct(reference, rows, motions, step);
    if frame.count == 0 {
        return Ok(1.);
    }
    let after = predicted(&frame, motions, step);
    let values: [f32; 4] = std::array::from_fn(|i| {
        let decrease = max(frame.distances[i] - after[i], f32::EPSILON);
        let below = min(frame.distances[i] - threshold[i], 0.);
        reciprocal(decrease) * (frame.distances[i] - below)
    });
    Ok(match frame.count {
        1 => values[0],
        2 => min(values[1], values[0]),
        3 => min(values[2], min(values[1], values[0])),
        _ => min(min(values[2], values[0]), min(values[3], values[1])),
    })
}
