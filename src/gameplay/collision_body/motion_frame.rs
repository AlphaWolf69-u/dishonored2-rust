use super::CollisionBody;
use crate::{
    assets::physworld::{normalize_body_quaternion, rotation_quaternion},
    gameplay::rigid_body::BodyPose,
};

fn local_motion_center(body: &CollisionBody, center: [f32; 4]) -> [f32; 3] {
    let d: [f32; 3] = std::array::from_fn(|i| center[i] - body.transform[3][i]);
    std::array::from_fn(|i| {
        (d[1] * body.transform[i][1] + d[0] * body.transform[i][0]) + d[2] * body.transform[i][2]
    })
}
fn length(squared: f32) -> f32 {
    if squared <= 0. {
        0.
    } else {
        crate::gameplay::constraint_solver::inverse_length(squared) * squared
    }
}
impl CollisionBody {
    /// ca6aa0 caches the attached motion's center and orientation in body space.
    /// Supplying the actual creation quaternion preserves its rounding; a later
    /// update may instead reconstruct it from the current body basis.
    pub fn attach_motion_frame(&mut self, motion: &BodyPose, orientation: Option<[f32; 4]>) {
        let center = local_motion_center(self, motion.center);
        for i in 0..3 {
            self.transform[i][3] = center[i];
        }
        let b = orientation.unwrap_or_else(|| {
            rotation_quaternion([self.transform[0], self.transform[1], self.transform[2]])
        });
        let m = motion.orientation;
        let q = [
            (b[3] * m[0] + (b[2] * m[1] - m[2] * b[1])) - m[3] * b[0],
            (b[3] * m[1] + (b[0] * m[2] - m[0] * b[2])) - m[3] * b[1],
            (b[3] * m[2] + (b[1] * m[0] - m[1] * b[0])) - m[3] * b[2],
            (m[2] * b[2] + m[0] * b[0]) + (m[3] * b[3] + m[1] * b[1]),
        ];
        self.motion_to_body_orientation = normalize_body_quaternion(q);
    }

    /// ca6c20 after the selected shape's identity calcAabb. Bounds are not a
    /// narrow-phase replacement: this is only the source's cached motion radius.
    pub fn update_motion_radius(&mut self, motion_center: [f32; 4], local_bounds: [[f32; 4]; 2]) {
        let center = local_motion_center(self, motion_center);
        let far: [f32; 3] = std::array::from_fn(|i| {
            let low = (local_bounds[0][i] - center[i]).abs();
            let high = (local_bounds[1][i] - center[i]).abs();
            if low > high {
                low
            } else {
                high
            }
        });
        let squared = (far[1] * far[1] + far[0] * far[0]) + far[2] * far[2];
        self.packed_radius = (length(squared).to_bits() >> 16) as u16;
    }

    /// cc8b40 only updates flag bit0 and, on success, the packed reference.
    pub fn update_speed_lod(&mut self, velocity: [f32; 4], threshold: f32, reference: f32) {
        let squared =
            (velocity[1] * velocity[1] + velocity[0] * velocity[0]) + velocity[2] * velocity[2];
        if length(squared) > threshold {
            self.lod_flags |= 1;
            self.packed_average_surface_velocity = (reference.to_bits() >> 16) as u16;
        } else {
            self.lod_flags &= !1;
        }
    }
}

#[cfg(test)]
#[path = "motion_frame_tests.rs"]
mod tests;
