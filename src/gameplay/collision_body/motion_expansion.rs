use super::CollisionBody;
use crate::gameplay::motion::Motion;

/// Swept-bounds inputs produced by 1280a10. Step and tolerance retain all four
/// lanes from the world; no frame duration is inferred from the render clock.
#[derive(Clone, Copy, Debug)]
pub struct MotionExpansion {
    pub displacement: [f32; 4],
    pub cage_displacement: [f32; 4],
    pub angular_padding: [f32; 4],
    pub travel: [f32; 4],
    pub speed_bound: [f32; 4],
}

fn estimated_length(v: [f32; 4]) -> f32 {
    let squared = (v[1] * v[1] + v[0] * v[0]) + v[2] * v[2];
    #[cfg(target_arch = "x86_64")]
    let estimate = unsafe {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(squared)))
    };
    #[cfg(not(target_arch = "x86_64"))]
    let estimate: f32 = unimplemented!("DH2 motion expansion requires x86-64 RSQRT");
    if squared <= 0. {
        0.
    } else {
        estimate * squared
    }
}

impl CollisionBody {
    /// c92500's maximum TIM distance uses the same unrefined length, without
    /// the swept-bounds safety multiplier. Preserve the world's multiply order.
    pub fn update_tim_distance(&mut self, velocity: [f32; 4], contact_scale: f32, duration: f32) {
        let distance = (contact_scale * duration) * estimated_length(velocity);
        let distance = if distance > 0. { distance } else { 0. };
        let distance = if distance < 65535. { distance } else { 65535. };
        self.max_tim_distance = crate::float::truncate_i32(distance) as u16;
    }

    /// Source 1280a10, including its unrefined RSQRT length estimates and 1.01
    /// safety multiplier for linear and angular speed (not cage speed).
    pub fn motion_expansion(
        &self,
        motion: &Motion,
        tolerance: [f32; 4],
        step: [f32; 4],
    ) -> MotionExpansion {
        let unpack = |v: u16| f32::from_bits(u32::from(v) << 16);
        let radius = unpack(self.packed_radius);
        let cage = [
            unpack(motion.linear_velocity_cage[0]),
            unpack(motion.linear_velocity_cage[1]),
            unpack(motion.linear_velocity_cage[2]),
            unpack(motion.integration_factor),
        ];
        let linear = estimated_length(motion.linear_velocity) * f32::from_bits(0x3f81_47ae);
        let angular = estimated_length(motion.angular_velocity) * f32::from_bits(0x3f81_47ae);
        let cage_speed = estimated_length(cage);
        // MAXPS returns its second operand for equal or unordered values.
        let speed = if linear > cage_speed {
            linear
        } else {
            cage_speed
        };
        MotionExpansion {
            displacement: std::array::from_fn(|i| motion.linear_velocity[i] * step[i]),
            cage_displacement: std::array::from_fn(|i| step[i] * cage[i]),
            angular_padding: std::array::from_fn(|i| {
                let margin = tolerance[i] * 0.5;
                let lookahead = self.transform[3][3];
                let margin = if lookahead > margin {
                    lookahead
                } else {
                    margin
                };
                let angle = step[i] * angular;
                let angle = if angle < 1. { angle } else { 1. };
                angle * radius + margin
            }),
            travel: step.map(|dt| dt * speed),
            speed_bound: [radius * angular + linear; 4],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn motion_expansion_matches_executed_routine() {
        for (case, line) in include_str!("../../../tests/fixtures/motion_expansion.txt")
            .lines()
            .enumerate()
        {
            let w: Vec<u32> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let mut body = CollisionBody::default();
            body.packed_radius = w[0] as u16;
            body.transform[3][3] = f32::from_bits(w[1]);
            let motion = Motion {
                linear_velocity: vector(2),
                angular_velocity: vector(6),
                linear_velocity_cage: [w[10] as u16, w[11] as u16, w[12] as u16],
                integration_factor: w[13] as u16,
                ..Default::default()
            };
            let result = body.motion_expansion(&motion, vector(14), vector(18));
            let actual: Vec<u32> = [
                result.displacement,
                result.cage_displacement,
                result.angular_padding,
                result.travel,
                result.speed_bound,
            ]
            .into_iter()
            .flatten()
            .map(f32::to_bits)
            .collect();
            assert_eq!(actual, w[22..], "case {case}");
        }
    }
}
