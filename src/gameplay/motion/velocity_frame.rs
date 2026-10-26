use super::Motion;
impl Motion {
    /// ca71b0, used to preserve world angular velocity across mass-frame changes.
    /// The fourth lane follows the source arithmetic rather than being cleared.
    pub fn world_angular_velocity(&self) -> [f32; 4] {
        let [x, y, z, w] = self.pose.orientation;
        let [vx, vy, vz, vw] = self.angular_velocity;
        let dot = (vy * y + vx * x) + vz * z;
        let factor = w * w - 0.5;
        [
            (dot * x + factor * vx) + (vz * y - z * vy) * w,
            (dot * y + factor * vy) + (vx * z - x * vz) * w,
            (dot * z + factor * vz) + (vy * x - y * vx) * w,
            (dot * w + factor * vw) + (vw * w - w * vw) * w,
        ]
        .map(|v| v + v)
    }
    /// ca76c0 is the shared aa07e0 inverse rotation with a motion destination.
    pub fn set_world_angular_velocity(&mut self, velocity: [f32; 4]) {
        self.angular_velocity = self.pose.inverse_rotate_vector(velocity);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn angular_velocity_frames_match_executed_routines() {
        let mut count = 0;
        for (case, line) in include_str!("../../../tests/fixtures/angular_velocity_frame.txt")
            .lines()
            .enumerate()
        {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let mut motion = Motion::default();
            motion.pose.orientation = vector(0);
            motion.angular_velocity = vector(4);
            let actual = motion.world_angular_velocity().map(f32::to_bits);
            assert_eq!(actual, w[12..16], "read case {case}");
            motion.set_world_angular_velocity(vector(8));
            assert_eq!(
                motion.angular_velocity.map(f32::to_bits),
                w[16..],
                "write case {case}"
            );
            count += 1;
        }
        assert_eq!(count, 1024);
    }
}
