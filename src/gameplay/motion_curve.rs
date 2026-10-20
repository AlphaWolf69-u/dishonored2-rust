//! Midnight motion interpolation815f90 and Euler basis10d490. Angles are
//! unwrapped degrees: quaternion interpolation would erase a full revolution.
pub use crate::entities::scene_motion::{CurveKey, Interpolation, MotionCurve};
impl MotionCurve {
    pub fn sample(&self, time_ms: i32) -> ([f32; 3], [f32; 3]) {
        let keys = &self.keys;
        let first = &keys[0];
        let last = keys.last().unwrap();
        if time_ms <= first.time_ms {
            return (first.position, first.orientation);
        }
        if time_ms >= last.time_ms {
            return (last.position, last.orientation);
        }
        let i = keys.partition_point(|k| k.time_ms < time_ms) - 1;
        let a = &keys[i];
        let b = &keys[i + 1];
        let t = ((time_ms - a.time_ms) as f32 * 0.001) / ((b.time_ms - a.time_ms) as f32 * 0.001);
        let interpolate = |angles: bool| {
            let value = |k: &CurveKey| if angles { k.orientation } else { k.position };
            let av = value(a);
            let bv = value(b);
            match self.interpolation {
                Interpolation::Constant => av,
                Interpolation::Linear => std::array::from_fn(|c| av[c] + t * (bv[c] - av[c])),
                Interpolation::Bezier => {
                    let out = if angles { a.angle_out } else { a.tangent_out };
                    let input = if angles { b.angle_in } else { b.tangent_in };
                    let u = 1. - t;
                    std::array::from_fn(|c| {
                        bv[c] * (t * t * t)
                            + (out[c] + av[c]) * (t * 3. * u * u)
                            + av[c] * (u * u * u)
                            + (bv[c] + input[c]) * (t * 3. * t * u)
                    })
                }
                Interpolation::CatmullRom => {
                    //815af0 clamps a missing leading point and extrapolates a
                    //trailing one. The asymmetry is present in the executable.
                    let prev = if i > 0 { value(&keys[i - 1]) } else { av };
                    let next = if i + 2 < keys.len() {
                        value(&keys[i + 2])
                    } else {
                        std::array::from_fn(|c| 2. * bv[c] - av[c])
                    };
                    let h = t * 0.5;
                    let w0 = ((2. - t) * t - 1.) * h;
                    let w1 = (t * 3. - 5.) * t * h + 1.;
                    let w2 = ((4. - 3. * t) * t + 1.) * h;
                    let w3 = (t - 1.) * t * h;
                    std::array::from_fn(|c| w1 * av[c] + w0 * prev[c] + w2 * bv[c] + w3 * next[c])
                }
            }
        };
        (interpolate(false), interpolate(true))
    }
}
/// Columns in game Z-up coordinates. Motion orientation is pitch,yaw,roll;
/// blueprint child rotations have a different serialized convention.
pub fn angle_basis(angles: [f32; 3]) -> [[f32; 3]; 3] {
    let (sp, cp) = angles[0].to_radians().sin_cos();
    let (sy, cy) = angles[1].to_radians().sin_cos();
    let (sr, cr) = angles[2].to_radians().sin_cos();
    [
        [cp * cy, cp * sy, -sp],
        [sr * sp * cy - cr * sy, sr * sp * sy + cr * cy, sr * cp],
        [cr * sp * cy + sr * sy, cr * sp * sy - sr * cy, cr * cp],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    fn words(line: &str) -> Vec<f32> {
        line.split_whitespace()
            .map(|s| f32::from_bits(u32::from_str_radix(s, 16).unwrap()))
            .collect()
    }
    #[test]
    fn motion_curves_match_installed_interpolator() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/motion_curve.txt")
            .lines()
            .filter(|s| !s.starts_with('#') && !s.is_empty())
        {
            let w = words(line);
            let n = w[1] as usize;
            let keys = (0..n)
                .map(|i| {
                    let at = 3 + i * 18;
                    let v = |k: usize| std::array::from_fn(|c| w[at + k * 3 + c]);
                    CurveKey {
                        time_ms: i as i32 * 1301,
                        position: v(0),
                        orientation: v(1),
                        tangent_in: v(2),
                        tangent_out: v(3),
                        angle_in: v(4),
                        angle_out: v(5),
                    }
                })
                .collect();
            let interpolation = match w[0] as u8 {
                1 => Interpolation::Constant,
                2 => Interpolation::Linear,
                3 => Interpolation::CatmullRom,
                4 => Interpolation::Bezier,
                _ => unreachable!(),
            };
            let (p, a) = MotionCurve {
                keys,
                interpolation,
            }
            .sample(w[2] as i32);
            for (i, v) in p.into_iter().chain(a).enumerate() {
                assert!(
                    (v - w[3 + n * 18 + i]).abs() < 0.0002,
                    "case{count} field{i}: {v} != {}",
                    w[3 + n * 18 + i]
                );
            }
            count += 1;
        }
        assert_eq!(count, 128);
    }
    #[test]
    fn motion_angles_match_installed_basis() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/motion_angle_basis.txt")
            .lines()
            .filter(|s| !s.starts_with('#') && !s.is_empty())
        {
            let w = words(line);
            let b = angle_basis([w[0], w[1], w[2]]);
            for (i, v) in b.into_iter().flatten().enumerate() {
                assert!((v - w[3 + i]).abs() < 0.00001, "case{count} basis{i}");
            }
            count += 1;
        }
        assert_eq!(count, 128);
    }
}
