//! Motion pose and displacement outputs of c88590's dynamic-body branch.
use super::Motion;
use crate::{
    float::{add, multiply, subtract},
    gameplay::collision_body::CollisionBody,
};

fn length(squared: f32) -> f32 {
    #[cfg(target_arch = "x86_64")]
    let estimate = unsafe {
        use std::arch::x86_64::{_mm_cvtss_f32, _mm_rsqrt_ss, _mm_set_ss};
        _mm_cvtss_f32(_mm_rsqrt_ss(_mm_set_ss(squared)))
    };
    #[cfg(not(target_arch = "x86_64"))]
    let estimate: f32 = unimplemented!("body displacement requires x86-64 RSQRT");
    if squared <= 0. {
        0.
    } else {
        multiply(
            multiply(
                multiply(
                    subtract(3., multiply(multiply(estimate, squared), estimate)),
                    estimate,
                ),
                0.5,
            ),
            squared,
        )
    }
}

impl Motion {
    /// The body caches its local motion center in basis W and its relative
    /// orientation at+a0. An absent quaternion retains the motion orientation;
    /// retain_center preserves all four motion-center words. Neither mode
    /// normalizes the resulting quaternion. Returns linear/angular distances.
    pub fn update_body_pose(
        &mut self,
        body: &CollisionBody,
        orientation: Option<[f32; 4]>,
        retain_center: bool,
    ) -> [f32; 2] {
        let mut distances = [0.; 2];
        let t = body.transform;
        if !retain_center {
            let center = std::array::from_fn(|i| {
                add(
                    add(
                        add(multiply(t[1][3], t[1][i]), multiply(t[0][3], t[0][i])),
                        multiply(t[2][3], t[2][i]),
                    ),
                    t[3][i],
                )
            });
            let delta: [f32; 3] = std::array::from_fn(|i| subtract(center[i], self.pose.center[i]));
            self.pose.center = center;
            distances[0] = length(add(
                add(multiply(delta[1], delta[1]), multiply(delta[0], delta[0])),
                multiply(delta[2], delta[2]),
            ));
        }
        if let Some(p) = orientation {
            let b = body.motion_to_body_orientation;
            let q = [
                add(
                    add(
                        subtract(multiply(b[2], p[1]), multiply(p[2], b[1])),
                        multiply(b[0], p[3]),
                    ),
                    multiply(p[0], b[3]),
                ),
                add(
                    add(
                        subtract(multiply(b[0], p[2]), multiply(p[0], b[2])),
                        multiply(b[1], p[3]),
                    ),
                    multiply(p[1], b[3]),
                ),
                add(
                    add(
                        subtract(multiply(b[1], p[0]), multiply(p[1], b[0])),
                        multiply(b[2], p[3]),
                    ),
                    multiply(p[2], b[3]),
                ),
                subtract(
                    multiply(b[3], p[3]),
                    add(
                        add(multiply(p[1], b[1]), multiply(p[0], b[0])),
                        multiply(p[2], b[2]),
                    ),
                ),
            ];
            let old = self.pose.orientation;
            self.pose.orientation = q;
            let delta = [
                add(
                    subtract(
                        subtract(multiply(q[2], old[1]), multiply(old[2], q[1])),
                        multiply(q[3], old[0]),
                    ),
                    multiply(old[3], q[0]),
                ),
                add(
                    subtract(
                        subtract(multiply(q[0], old[2]), multiply(old[0], q[2])),
                        multiply(q[3], old[1]),
                    ),
                    multiply(old[3], q[1]),
                ),
                add(
                    subtract(
                        subtract(multiply(q[1], old[0]), multiply(old[1], q[0])),
                        multiply(q[3], old[2]),
                    ),
                    multiply(old[3], q[2]),
                ),
            ];
            let first = add(multiply(q[2], old[2]), multiply(q[0], old[0]));
            let second = add(multiply(q[3], old[3]), multiply(q[1], old[1]));
            let signs = [
                add(second, first),
                add(first, second),
                add(
                    add(
                        add(multiply(q[1], old[1]), multiply(q[3], old[3])),
                        multiply(q[0], old[0]),
                    ),
                    multiply(q[2], old[2]),
                ),
            ];
            let delta: [f32; 3] = std::array::from_fn(|i| {
                f32::from_bits(
                    add(delta[i], delta[i]).to_bits() ^ (signs[i].to_bits() & 0x80000000),
                )
            });
            distances[1] = length(add(
                add(multiply(delta[1], delta[1]), multiply(delta[0], delta[0])),
                multiply(delta[2], delta[2]),
            ));
        }
        distances
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_motion_pose_matches_executed_dynamic_world_transform() {
        let mut cases = 0;
        for line in include_str!("../../../tests/fixtures/motion_body_pose.txt").lines() {
            let w: Vec<_> = line
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
            let body = CollisionBody {
                transform: std::array::from_fn(|i| vector(1 + i * 4)),
                motion_to_body_orientation: vector(17),
                ..Default::default()
            };
            let mut motion = Motion::default();
            motion.pose.center = vector(21);
            motion.pose.orientation = vector(25);
            let distances =
                motion.update_body_pose(&body, (w[0] & 1 != 0).then(|| vector(29)), w[0] & 2 != 0);
            let actual: Vec<_> = motion
                .pose
                .center
                .into_iter()
                .chain(motion.pose.orientation)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, w[33..41], "pose case{}", w[0]);
            let mut events = vec![1, 2];
            events.extend([distances[0].to_bits(); 4]);
            events.extend([distances[1].to_bits(); 4]);
            events.push(3);
            assert_eq!(events, w[42..], "displacement case{}", w[0]);
            cases += 1;
        }
        assert_eq!(cases, 512);
    }
}
