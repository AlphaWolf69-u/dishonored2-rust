use super::CollisionBody;
use crate::{
    assets::physworld::quaternion_basis,
    gameplay::{motion::Motion, shape_transform::ShapeMatrix},
};

impl CollisionBody {
    /// 1280b90's frame passed to the selected channel-3 shape's calcAabb.
    /// Its result is a rigid frame, with the source's defined padding lanes.
    pub fn predicted_shape_frame(&self, motion: &Motion, step: [f32; 4]) -> ShapeMatrix {
        let [x, y, z, w] = motion.pose.orientation;
        let [ax, ay, az] =
            std::array::from_fn::<_, 3, _>(|i| (step[i] * 0.5) * motion.angular_velocity[i]);
        let next = crate::assets::physworld::normalize_body_quaternion([
            ((az * y - z * ay) + ax * w) + x,
            ((ax * z - x * az) + ay * w) + y,
            ((ay * x - y * ax) + az * w) + z,
            w - ((ay * y + ax * x) + az * z),
        ]);
        let [nx, ny, nz, nw] = next;
        let [rx, ry, rz, rw] =
            crate::assets::physworld::normalize_body_quaternion(self.motion_to_body_orientation);
        let rotation = [
            ((nz * ry - rz * ny) - nw * rx) + rw * nx,
            ((nx * rz - rx * nz) - nw * ry) + rw * ny,
            ((ny * rx - ry * nx) - nw * rz) + rw * nz,
            (rx * nx + rz * nz) + (ry * ny + rw * nw),
        ];
        let basis = quaternion_basis(rotation);
        let local = [
            self.transform[0][3],
            self.transform[1][3],
            self.transform[2][3],
        ];
        let position = std::array::from_fn(|i| {
            (step[i] * motion.linear_velocity[i] + motion.pose.center[i])
                - ((local[1] * basis[1][i] + local[0] * basis[0][i]) + local[2] * basis[2][i])
        });
        [basis[0], basis[1], basis[2], position]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn predicted_frame_matches_executed_routine() {
        for (case, line) in include_str!("../../../tests/fixtures/predicted_body_frame.txt")
            .lines()
            .enumerate()
        {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let mut body = CollisionBody::default();
            body.motion_to_body_orientation = vector(8);
            for i in 0..3 {
                body.transform[i][3] = f32::from_bits(w[12 + i]);
            }
            let motion = Motion {
                pose: crate::gameplay::rigid_body::BodyPose {
                    center: vector(0),
                    orientation: vector(4),
                },
                linear_velocity: vector(15),
                angular_velocity: vector(19),
                ..Default::default()
            };
            let actual: Vec<u32> = body
                .predicted_shape_frame(&motion, vector(23))
                .into_iter()
                .flatten()
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, w[27..], "case {case}");
        }
    }
}
