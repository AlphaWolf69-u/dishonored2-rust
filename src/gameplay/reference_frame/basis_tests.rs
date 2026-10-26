use super::*;

#[test]
fn retained_relative_binding_matches_complete_executed_post_physics_branch() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/relative_binding.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let frames: [ReferenceFrame; 2] = std::array::from_fn(|_| ReferenceFrame {
            position: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            basis: std::array::from_fn(|_| {
                std::array::from_fn(|_| f32::from_bits(words.next().unwrap()))
            }),
        });
        let (offset, orientation) = relative_binding(frames[0], frames[1]);
        let actual: Vec<_> = offset
            .into_iter()
            .chain(orientation)
            .map(f32::to_bits)
            .collect();
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 512);
}

#[test]
fn compact_inverse_and_quaternion_match_complete_executed_game_functions() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/compact_basis.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let original =
            std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())));
        let expected = words.next().unwrap() != 0;
        let mut inverse = original;
        assert_eq!(
            invert_basis(&mut inverse),
            expected,
            "case {cases}: inversion admission"
        );
        let mut actual: Vec<_> = inverse.iter().flatten().map(|v| v.to_bits()).collect();
        actual.extend(basis_quaternion(original).map(f32::to_bits));
        actual.extend(basis_quaternion(inverse).map(f32::to_bits));
        assert_eq!(actual, words.collect::<Vec<_>>(), "case {cases}");
        cases += 1;
    }
    assert_eq!(cases, 1024);
}
