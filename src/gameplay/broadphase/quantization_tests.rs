use super::*;

#[test]
fn body_world_bounds_match_executed_packed_getter() {
    let mut count = 0;
    for (case, line) in include_str!("../../../tests/fixtures/body_world_bounds.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|s| u32::from_str_radix(s, 16).unwrap())
            .collect();
        let world = std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[i * 4 + j])));
        let q = BroadphaseQuantization::new(world);
        let packed = PackedBodyBounds {
            words: w[8..12].try_into().unwrap(),
        };
        let actual: Vec<u32> = q
            .packed
            .unpack_bounds(packed)
            .into_iter()
            .flatten()
            .map(f32::to_bits)
            .collect();
        assert_eq!(actual, w[12..], "case {case}");
        count += 1;
    }
    assert_eq!(count, 1536);
}

#[test]
fn packed_body_bounds_match_executed_static_preparation() {
    for (case, line) in include_str!("../../../tests/fixtures/packed_body_bounds.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let world = std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[i * 4 + j])));
        let bounds =
            std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[8 + i * 4 + j])));
        let tolerance = std::array::from_fn(|i| f32::from_bits(w[16 + i]));
        let q = BroadphaseQuantization::new(world);
        let packed = q.packed.pack_bounds(bounds, tolerance);
        assert_eq!(packed.words, &w[20..24], "case {case} packed bounds");
        let coarse = q.packed.query_bounds(packed);
        let actual: Vec<_> = coarse
            .min
            .into_iter()
            .chain(coarse.max)
            .map(u32::from)
            .collect();
        assert_eq!(actual, &w[24..], "case {case} query bounds");
    }
}
#[test]
fn quantization_matches_executed_world_setup() {
    for (case, line) in include_str!("../../../tests/fixtures/broadphase_quantization.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let bounds = std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[i * 4 + j])));
        let q = BroadphaseQuantization::new(bounds);
        let mut actual = Vec::new();
        for v in [
            q.offset,
            q.scale,
            q.inverse_scale,
            q.bounds[0],
            q.bounds[1],
            q.packed.offset,
            q.packed.scale,
            q.packed.inverse_scale,
        ] {
            actual.extend(v.map(f32::to_bits));
        }
        actual.push(u32::from(q.packed.swap_yz));
        for shuffle in [
            q.packed.layout.pack_shuffle,
            q.packed.layout.min_shuffle,
            q.packed.layout.max_shuffle,
        ] {
            actual.extend(
                shuffle
                    .chunks_exact(4)
                    .map(|v| u32::from_le_bytes(v.try_into().unwrap())),
            );
        }
        actual.extend(q.packed.layout.component_mask);
        actual.extend([if q.packed.swap_yz { u32::MAX } else { 0 }; 4]);
        actual.extend(q.packed.maximum().map(f32::to_bits));
        actual.extend(q.packed.layout.max_quantized);
        actual.extend([q.packed.boundary_step; 4]);
        actual.extend([q.packed.nonzero_extent_mask; 4]);
        assert_eq!(actual, &w[8..], "case {case}");
        let p = q.point_quantization();
        for i in 0..3 {
            assert_eq!(p.offset[i].to_bits(), q.offset[i].to_bits());
            assert_eq!(p.scale[i].to_bits(), q.inverse_scale[i].to_bits());
        }
    }
}
