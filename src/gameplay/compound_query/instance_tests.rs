use super::instance::*;
use crate::assets::physworld::compound::CompoundInstance;
use crate::gameplay::shape_transform::ScaledShapeTransform;

#[test]
fn compound_instances_match_executed_leaf_preparation() {
    for (case, line) in include_str!("../../../tests/fixtures/compound_instances.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let matrix = |at| std::array::from_fn(|i| vector(at + i * 4));
        let parent = CompoundShapeState {
            scaled: w[1] != 0,
            keep_radius: w[2] != 0,
            radius: f32::from_bits(w[3]),
            transform: ScaledShapeTransform {
                scale: vector(4),
                offset: vector(8),
                world_from_local: matrix(12),
            },
        };
        let instance = CompoundInstance {
            flags: w[0],
            transform: matrix(44),
            scale: vector(60),
            shape: 0,
            shape_tag: 0,
        };
        let count = w[67] as usize;
        let vertices: Vec<_> = (0..count).map(|i| vector(68 + i * 4)).collect();
        let key_at = 68 + count * 4;
        let key = ShapeKey {
            value: w[key_at],
            bits: w[key_at + 1],
        }
        .compound_child(w[key_at + 3] as u16, w[key_at + 2] as u8)
        .unwrap();
        let prepared = prepare_instance(
            &instance,
            parent,
            matrix(28),
            CompoundChildGeometry {
                shape_type: w[64] as u8,
                convex: w[65] != 0,
                radius: f32::from_bits(w[66]),
                vertices: &vertices,
            },
        )
        .unwrap();
        let state = prepared.state;
        let mut result = vec![
            key.value,
            key.bits,
            u32::from(state.scaled),
            u32::from(state.keep_radius),
            state.radius.to_bits(),
        ];
        result.extend(
            state
                .transform
                .scale
                .into_iter()
                .chain(state.transform.offset)
                .map(f32::to_bits),
        );
        result.extend(
            state
                .transform
                .world_from_local
                .into_iter()
                .flatten()
                .map(f32::to_bits),
        );
        result.extend(prepared.relative.into_iter().flatten().map(f32::to_bits));
        assert_eq!(
            result,
            w[key_at + 4..w.len() - 1],
            "compound instance case {case}"
        );
    }
}
