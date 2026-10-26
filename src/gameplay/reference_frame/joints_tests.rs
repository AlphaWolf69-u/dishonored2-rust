use super::*;
use crate::gameplay::{physics_actor::ResolvedMotion, reference_frame::ReferenceFrameCache};

#[test]
fn joint_bound_frames_match_complete_executed_local_and_world_rules() {
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/joint_bound_frames.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let vector = |i| std::array::from_fn(|j| f32::from_bits(w[i + j]));
        let frame = |i| ReferenceFrame {
            position: vector(i),
            basis: std::array::from_fn(|r| vector(i + 3 + r * 3)),
        };
        let channel = w[1] as usize;
        let joint = w[2] as i16;
        let remaps: Vec<Vec<i16>> = (0..4)
            .map(|i| {
                if w[4] >> i & 1 != 0 {
                    w[5 + i * 8..13 + i * 8].iter().map(|v| *v as i16).collect()
                } else {
                    Vec::new()
                }
            })
            .collect();
        let transforms: Vec<_> = w[37..165]
            .chunks_exact(16)
            .map(|v| std::array::from_fn(|i| f32::from_bits(v[i])))
            .collect();
        let source = JointFrames {
            joint_count: w[3] as u16,
            channel_remaps: &remaps,
            transforms: &transforms,
            model_scale: vector(165),
        };
        let offset = vector(168);
        let orientation = std::array::from_fn(|i| f32::from_bits(w[171 + i]));
        let mut output = frame(232);
        let result = match w[0] {
            0 => local_bound_frame(&source, channel, joint, offset, orientation, &mut output),
            1 => world_bound_frame(
                &source,
                channel,
                joint,
                offset,
                orientation,
                || Ok(frame(175)),
                &mut output,
            ),
            2 => world_bound_frame(
                &source,
                channel,
                joint,
                offset,
                orientation,
                || {
                    let mut world = frame(187);
                    if w[225] != 0 {
                        let mut cache = ReferenceFrameCache {
                            current: frame(199),
                            predicted: frame(211),
                            marker: w[223] as i32,
                            ..Default::default()
                        };
                        let mut motion = ResolvedMotion::default();
                        world.position = cache.position(w[224] as i32, world, &mut motion, 0.);
                        world.basis = cache.basis(w[224] as i32, world, &mut motion, 0.);
                    }
                    world.position = bound_origin(world.position, vector(226), vector(229));
                    Ok(world)
                },
                &mut output,
            ),
            _ => unreachable!(),
        }
        .unwrap();
        let mut actual = vec![u32::from(result)];
        actual.extend(
            output
                .position
                .into_iter()
                .chain(output.basis.into_iter().flatten())
                .map(f32::to_bits),
        );
        assert_eq!(actual, w[244..], "case {count}, operation {}", w[0]);
        count += 1;
    }
    assert_eq!(count, 768);
}
