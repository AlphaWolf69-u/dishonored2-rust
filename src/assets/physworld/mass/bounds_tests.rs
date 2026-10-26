use super::*;
#[test]
fn box_inertia_matches_executed_helper() {
    for (case, line) in include_str!("../../../../tests/fixtures/box_mass.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let half = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let mass = f32::from_bits(words.next().unwrap());
        let status = words.next().unwrap();
        let expected: [u32; 16] = std::array::from_fn(|_| words.next().unwrap());
        let actual = box_mass_properties(half, mass);
        assert_eq!(actual.is_none(), status != 0, "case {case} result");
        if let Some(actual) = actual {
            assert_eq!(actual.volume.to_bits(), expected[0], "case {case} volume");
            assert_eq!(
                actual.inertia.map(f32::to_bits),
                expected[8..12],
                "case {case} inertia"
            );
        }
        assert!(words.next().is_none());
    }
}
#[test]
fn bounds_mass_matches_executed_helper() {
    for (case, line) in include_str!("../../../../tests/fixtures/bounds_mass.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let config = [0, words.next().unwrap(), words.next().unwrap()];
        let bounds =
            std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())));
        let expected: [u32; 16] = std::array::from_fn(|_| words.next().unwrap());
        let actual = bounds_mass_properties(config, bounds);
        let actual: Vec<_> = [actual.volume, actual.mass]
            .into_iter()
            .chain(actual.center)
            .chain(actual.inertia)
            .chain(actual.orientation)
            .map(f32::to_bits)
            .collect();
        let expected: Vec<_> = expected[..2]
            .iter()
            .chain(&expected[4..])
            .copied()
            .collect();
        assert_eq!(actual, expected, "case {case}");
        assert_eq!(words.count(), 16);
    }
}
