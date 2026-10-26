use super::*;

#[test]
fn bound_frames_and_quaternion_helpers_match_executed_math() {
    let mut counts = [0; 2];
    for row in include_str!("../../../tests/fixtures/reference_frame_math.txt").lines() {
        let mut parts = row.split_whitespace();
        let kind = parts.next().unwrap();
        let w: Vec<_> = parts.map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
        let v3 = |i| std::array::from_fn(|j| f32::from_bits(w[i + j]));
        let v4 = |i| std::array::from_fn(|j| f32::from_bits(w[i + j]));
        let basis =
            |i| std::array::from_fn(|r| std::array::from_fn(|c| f32::from_bits(w[i + r * 3 + c])));
        let (actual, expected) = if kind == "math" {
            let mut actual: Vec<_> = quaternion_basis(v4(0))
                .into_iter()
                .flatten()
                .map(f32::to_bits)
                .collect();
            actual
                .extend(crate::gameplay::shape_transform::rotation::asin(v4(4)).map(f32::to_bits));
            actual.push(quaternion_angle(v4(0)).to_bits());
            actual.extend(quaternion_angles(v4(0)).map(f32::to_bits));
            counts[0] += 1;
            (actual, &w[8..])
        } else {
            let parent = ReferenceFrame {
                position: v3(7),
                basis: basis(10),
            };
            let previous = ReferenceFrame {
                position: v3(19),
                basis: basis(22),
            };
            let next = bind(parent, v3(0), v4(3));
            let mut actual: Vec<_> = next
                .position
                .into_iter()
                .chain(next.basis.into_iter().flatten())
                .map(f32::to_bits)
                .collect();
            actual.extend(
                velocities(previous, next, f32::from_bits(w[31]))
                    .into_iter()
                    .flatten()
                    .map(f32::to_bits),
            );
            counts[1] += 1;
            (actual, &w[32..])
        };
        assert_eq!(
            actual,
            expected,
            "{kind} case {}",
            counts[usize::from(kind != "math")] - 1
        );
    }
    assert_eq!(counts, [1024, 1024]);
}

#[test]
fn prediction_and_lazy_getters_match_executed_frame_state() {
    use crate::gameplay::physics_actor::ResolvedMotion;
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/reference_frame_prediction.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let v3 = |i| std::array::from_fn(|j| f32::from_bits(w[i + j]));
        let frame = |i| ReferenceFrame {
            position: v3(i),
            basis: std::array::from_fn(|r| v3(i + 3 + r * 3)),
        };
        let marker = w[1] as i32;
        let delta = f32::from_bits(w[2]);
        let source = frame(3);
        let mut cache = ReferenceFrameCache {
            current: frame(15),
            predicted: frame(27),
            marker: w[39] as i32,
            ..Default::default()
        };
        let mut motion = ResolvedMotion {
            velocities: [v3(40), v3(43)],
            valid: [w[46] != 0, w[47] != 0],
            dirty: [w[48] != 0, w[49] != 0],
        };
        let output: Vec<_> = match w[0] {
            0 => {
                cache.predict_position(source, &mut motion, delta);
                Vec::new()
            }
            1 => {
                cache.predict_basis(source, &mut motion, delta);
                Vec::new()
            }
            2 => cache
                .position(marker, source, &mut motion, delta)
                .into_iter()
                .map(f32::to_bits)
                .collect(),
            3 => cache
                .basis(marker, source, &mut motion, delta)
                .into_iter()
                .flatten()
                .map(f32::to_bits)
                .collect(),
            _ => unreachable!(),
        };
        let mut actual = Vec::new();
        for frame in [cache.current, cache.predicted] {
            actual.extend(
                frame
                    .position
                    .into_iter()
                    .chain(frame.basis.into_iter().flatten())
                    .map(f32::to_bits),
            );
        }
        actual.push(cache.marker as u32);
        actual.extend(motion.velocities.into_iter().flatten().map(f32::to_bits));
        actual.extend(motion.valid.map(u32::from));
        actual.extend(motion.dirty.map(u32::from));
        actual.extend(output);
        assert_eq!(actual, w[50..], "prediction {count}, operation {}", w[0]);
        count += 1;
    }
    assert_eq!(count, 512);
}
