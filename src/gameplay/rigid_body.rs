//! Owned rigid-body motion primitives shared by machinery and physics.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyPose {
    pub center: [f32; 4],
    pub orientation: [f32; 4],
}

/// Havok motion-space pose and the four packed inverse principal mass lanes.
/// Retaining the packed form also retains the game's intentional precision.
#[derive(Clone, Copy, Debug)]
pub struct BodyMotion {
    pub pose: BodyPose,
    pub inverse_mass: [u16; 4],
}

impl BodyMotion {
    /// 140ca6f60: compose a shape's mass frame with its rigid world transform.
    pub fn from_mass_frame(
        frame: crate::physworld::mass::PrincipalMassProperties,
        orientation: [f32; 4],
        translation: [f32; 4],
    ) -> Self {
        let pose = BodyPose::from_mass_frame(frame, orientation, translation);
        let inverse_mass = std::array::from_fn(|i| {
            let value = if i == 3 { frame.mass } else { frame.inertia[i] };
            let inverse = if value == 0. {
                0.
            } else {
                super::constraint_solver::reciprocal(value)
            };
            let rounded = (inverse * f32::from_bits(0x3f808000)).to_bits() as i32 >> 16;
            rounded.clamp(i16::MIN as i32, i16::MAX as i32) as u16
        });
        Self { pose, inverse_mass }
    }

    pub fn inverse_mass_values(&self) -> [f32; 4] {
        self.inverse_mass
            .map(|v| f32::from_bits(u32::from(v) << 16))
    }
}

impl BodyPose {
    /// aecc50 compares all quaternion lanes after choosing its sign-equivalent
    /// representative, then the center's XYZ lanes. Center W is ignored.
    pub fn matches_pose(&self, other: &Self, tolerance: f32) -> bool {
        let p: [f32; 4] = std::array::from_fn(|i| other.orientation[i] * self.orientation[i]);
        let signs = [
            (p[3] + p[1]) + (p[2] + p[0]),
            (p[2] + p[0]) + (p[3] + p[1]),
            (p[1] + p[3]) + (p[0] + p[2]),
            (p[0] + p[2]) + (p[1] + p[3]),
        ];
        (0..4).all(|i| {
            let aligned =
                f32::from_bits(other.orientation[i].to_bits() ^ (signs[i].to_bits() & 0x8000_0000));
            (self.orientation[i] - aligned).abs() <= tolerance
        }) && (0..3).all(|i| (self.center[i] - other.center[i]).abs() <= tolerance)
    }

    /// aa07e0's inverse quaternion rotation, including its defined W lane.
    pub fn inverse_rotate_vector(&self, vector: [f32; 4]) -> [f32; 4] {
        let [x, y, z, w] = self.orientation;
        let [vx, vy, vz, vw] = vector;
        let dot = (y * vy + x * vx) + z * vz;
        let factor = w * w - 0.5;
        let values = [
            (dot * x + factor * vx) + (z * vy - vz * y) * w,
            (dot * y + factor * vy) + (x * vz - vx * z) * w,
            (dot * z + factor * vz) + (y * vx - vy * x) * w,
            (dot * w + factor * vw) + (w * vw - vw * w) * w,
        ];
        values.map(|v| v + v)
    }

    /// Shared mass-frame composition used by ca6f60 and cb9680.
    pub fn from_mass_frame(
        frame: crate::physworld::mass::PrincipalMassProperties,
        orientation: [f32; 4],
        translation: [f32; 4],
    ) -> Self {
        let [x, y, z, w] = orientation;
        let [cx, cy, cz, cw] = frame.center;
        let dot = (y * cy + x * cx) + z * cz;
        let rotated = [
            (dot * x + (w * w - 0.5) * cx) + (cz * y - z * cy) * w,
            (dot * y + (w * w - 0.5) * cy) + (cx * z - x * cz) * w,
            (dot * z + (w * w - 0.5) * cz) + (cy * x - y * cx) * w,
            (dot * w + (w * w - 0.5) * cw) + (cw * w - w * cw) * w,
        ];
        let center = std::array::from_fn(|i| rotated[i] + rotated[i] + translation[i]);
        let [qx, qy, qz, qw] = frame.orientation;
        let orientation = [
            ((qz * y - z * qy) + w * qx) + x * qw,
            ((qx * z - x * qz) + w * qy) + y * qw,
            ((qy * x - y * qx) + w * qz) + z * qw,
            qw * w - ((y * qy + x * qx) + z * qz),
        ];
        Self {
            center,
            orientation,
        }
    }

    /// Complete FUN_1412815e0 motion integration. Angular velocity is in
    /// principal-axis space. The game normalizes the Euler quaternion step;
    /// replacing this with an axis-angle exponential changes its trajectory.
    pub fn integrate(
        &mut self,
        linear_velocity: [f32; 4],
        angular_velocity: [f32; 4],
        seconds: [f32; 4],
    ) {
        let [x, y, z, w] = self.orientation;
        let [ax, ay, az] =
            std::array::from_fn::<_, 3, _>(|i| angular_velocity[i] * seconds[i] * 0.5);
        let next = [
            ((az * y - z * ay) + w * ax) + x,
            ((ax * z - x * az) + w * ay) + y,
            ((ay * x - y * ax) + w * az) + z,
            w - ((y * ay + x * ax) + z * az),
        ];
        let length = ((next[2] * next[2] + next[0] * next[0])
            + (next[3] * next[3] + next[1] * next[1]))
            .sqrt();
        self.orientation = next.map(|v| v * (1. / length));
        for i in 0..4 {
            self.center[i] += seconds[i] * linear_velocity[i];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_comparison_matches_executed_routine() {
        let mut count = 0;
        for (case, line) in include_str!("../../tests/fixtures/pose_comparison.txt")
            .lines()
            .enumerate()
        {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let a = BodyPose {
                orientation: vector(0),
                center: vector(4),
            };
            let b = BodyPose {
                orientation: vector(8),
                center: vector(12),
            };
            assert_eq!(
                u32::from(a.matches_pose(&b, f32::from_bits(w[16]))),
                w[17],
                "case {case}"
            );
            count += 1;
        }
        assert_eq!(count, 1024);
    }

    #[test]
    fn motion_mass_frames_match_executable() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/body_mass_frame_cases.txt")
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
        {
            let bits: Vec<u32> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |i| f32::from_bits(bits[i]);
            let vector = |at| std::array::from_fn(|i| v(at + i));
            let frame = crate::physworld::mass::PrincipalMassProperties {
                volume: v(0),
                mass: v(1),
                center: vector(4),
                inertia: vector(8),
                orientation: vector(12),
            };
            let motion = BodyMotion::from_mass_frame(frame, vector(16), vector(20));
            let mut actual: Vec<u32> = motion
                .pose
                .center
                .into_iter()
                .chain(motion.pose.orientation)
                .map(f32::to_bits)
                .collect();
            actual
                .push(u32::from(motion.inverse_mass[0]) | u32::from(motion.inverse_mass[1]) << 16);
            actual
                .push(u32::from(motion.inverse_mass[2]) | u32::from(motion.inverse_mass[3]) << 16);
            assert_eq!(actual, bits[24..], "motion frame case {count}");
            count += 1;
        }
        assert_eq!(count, 256);
    }

    #[test]
    fn body_pose_integration_matches_executable() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/body_integration_cases.txt")
            .lines()
            .filter(|row| !row.starts_with('#') && !row.is_empty())
        {
            let bits: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let v = |offset| std::array::from_fn(|i| f32::from_bits(bits[offset + i]));
            let mut pose = BodyPose {
                center: v(0),
                orientation: v(4),
            };
            pose.integrate(v(8), v(12), v(16));
            let actual: Vec<_> = pose
                .center
                .into_iter()
                .chain(pose.orientation)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, bits[20..], "integration case {count}");
            count += 1;
        }
        assert_eq!(count, 512);
    }
}
