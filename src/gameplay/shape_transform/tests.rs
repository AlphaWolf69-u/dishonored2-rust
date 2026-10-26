use super::*;

#[test]
fn transformed_bounds_match_executed_compound_virtual() {
    for (case, line) in include_str!("../../../tests/fixtures/shape_aabb.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let matrix = std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[i * 4 + j])));
        let bounds =
            std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(w[16 + i * 4 + j])));
        let actual = transform_bounds(matrix, bounds);
        let values: Vec<_> = actual.into_iter().flatten().map(f32::to_bits).collect();
        assert_eq!(values, &w[24..], "case {case}");
    }
}
#[test]
fn shape_transforms_match_executed_kernels() {
    for (case, line) in include_str!("../../../tests/fixtures/shape_transforms.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let matrix = |at| std::array::from_fn(|i| vector(at + i * 4));
        let a = matrix(0);
        let b = matrix(16);
        let query = ScaledShapeTransform {
            world_from_local: a,
            scale: vector(32),
            offset: vector(40),
        };
        let target = ScaledShapeTransform {
            world_from_local: b,
            scale: vector(36),
            offset: vector(44),
        };
        let (relative, reciprocal) = relative_scaled(query, target);
        let mut result = rotate_vector(a, b[0]).to_vec();
        result.extend(inverse_rigid(a).into_iter().flatten());
        result.extend(compose(a, b).into_iter().flatten());
        result.extend(relative.into_iter().flatten());
        result.extend(reciprocal);
        for (lane, (actual, &expected)) in result.into_iter().zip(&w[48..]).enumerate() {
            assert_eq!(
                actual.to_bits(),
                expected,
                "transform case {case}, result lane {lane}"
            );
        }
    }
}
#[test]
fn body_transform_copy_and_comparison_match_executed_routines() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/body_transform.txt").lines() {
        let words: Vec<_> = line.split_whitespace().map(|w| u32::from_str_radix(w, 16).unwrap()).collect();
        let matrix = |start: usize| std::array::from_fn(|i| std::array::from_fn(|j|
            f32::from_bits(words[start + i * 4 + j])));
        let first = matrix(0);
        let second = matrix(16);
        assert_eq!(super::matches_transform(first, second, f32::from_bits(words[32])), words[33] != 0, "comparison case{cases}");
        let mut body = crate::gameplay::collision_body::CollisionBody { transform: first, ..Default::default() };
        body.set_transform(second);
        assert_eq!(body.transform.into_iter().flatten().map(f32::to_bits).collect::<Vec<_>>(), words[34..], "copy case{cases}");
        cases += 1;
    }
    assert_eq!(cases, 1024);
}

