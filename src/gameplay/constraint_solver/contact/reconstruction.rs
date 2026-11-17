//! c583a0 header ownership and cb9320/c4e5c0's retired-Jacobian manifold query.
//! No old world-space witnesses are retained here: the live motion transforms
//! reconstruct them from the actual stream's angular rows and plane header.
use super::{ContactBuild, ContactPoint, ContactRows};
use crate::gameplay::{
    constraint_solver::{inverse_length, reciprocal, step::SolverParameters},
    motion::Motion,
};

#[derive(Clone, Copy, Debug)]
pub struct ManifoldHeader {
    plane_offset: f32,
    packed_plane_normal: [u16; 3],
}
impl ManifoldHeader {
    pub(super) fn build(input: &ContactBuild<'_>) -> Self {
        // Native motion+2c is the solver ID, not inverse mass: keyframed
        // bodies can have zero inverse mass and still select the B frame.
        let reference = input.motions[usize::from(input.motions[1].solver_id != 0)];
        let point = input.points[0].position;
        let delta: [f32; 3] = std::array::from_fn(|i| point[i] - reference.pose.center[i]);
        let plane_offset = dot(delta, input.normal);
        let mut plane = input.normal;
        if input.points.len() >= 3 {
            let a: [f32; 3] = std::array::from_fn(|i| input.points[1].position[i] - point[i]);
            let b: [f32; 3] = std::array::from_fn(|i| input.points[2].position[i] - point[i]);
            plane = cross(a, b);
            let inverse = inverse_length(dot(plane, plane));
            plane = plane.map(|v| v * inverse);
        }
        // c583a0: truncate the scaled normal then retain its biased high word.
        let packed_plane_normal = plane.map(|v| {
            let integer = (v * f32::from_bits(0x4eea_6000)) as i32;
            ((integer as u32).wrapping_add(0x8000_0000) >> 16) as u16
        });
        Self {
            plane_offset,
            packed_plane_normal,
        }
    }
}

impl ContactRows {
    /// c4e5c0, called by cb9320 when event.manifold.count is FFFFFFFF.
    /// The solver coefficients and motions are borrowed at callback time.
    pub fn reconstruct(&self, motions: [&Motion; 2], step: &SolverParameters) -> [ContactPoint; 4] {
        let side = usize::from(motions[1].solver_id != 0);
        let reference = motions[side];
        let sign = if side == 0 { 1. } else { -1. };
        let normal = self.normal;
        let mut plane = self.reconstruction.packed_plane_normal.map(|v| {
            (u32::from(v).wrapping_shl(16).wrapping_add(0x8000_0000) as i32) as f32
                * f32::from_bits(0x300b_cf65)
        });
        if plane.iter().all(|&v| v == 0.) {
            plane = normal;
        }
        let mut first = [0.; 3];
        let mut output = [ContactPoint::default(); 4];
        for (index, point) in output
            .iter_mut()
            .enumerate()
            .take(usize::from(self.count).min(4))
        {
            let row = self.rows[index];
            let local = if side == 0 {
                row.angular_a
            } else {
                row.angular_b
            };
            let rotated = rotate(reference.pose.orientation, local);
            // Native n cross rotated-angular-row, with the selected side sign.
            let mut position = cross(normal, rotated).map(|v| v * sign);
            if index == 0 {
                for i in 0..3 {
                    position[i] = self.reconstruction.plane_offset * normal[i] + position[i];
                }
                first = position;
            } else {
                let mut denominator = dot(normal, plane);
                if denominator.abs() < f32::from_bits(0x3400_0000) {
                    denominator = 1.;
                }
                let difference = std::array::from_fn(|i| first[i] - position[i]);
                let distance = reciprocal(denominator) * dot(difference, plane);
                for i in 0..3 {
                    position[i] = normal[i] * distance + position[i];
                }
            }
            for i in 0..3 {
                point.position[i] = position[i]
                    + (0. - step.seconds) * reference.previous_step_linear_velocity[i]
                    + reference.pose.center[i];
            }
            point.distance = row.rhs * (0. - step.velocity_scale * step.substep_seconds);
        }
        output
    }
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
fn rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let [vx, vy, vz] = v;
    let dot = (vy * y + vx * x) + vz * z;
    let factor = w * w - 0.5;
    [
        (dot * x + factor * vx) + (vz * y - vy * z) * w,
        (dot * y + factor * vy) + (vx * z - vz * x) * w,
        (dot * z + factor * vz) + (vy * x - vx * y) * w,
    ]
    .map(|v| v + v)
}
