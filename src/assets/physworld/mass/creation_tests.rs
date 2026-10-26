use super::*;
use crate::assets::physworld::body::MassDistribution;

#[test]
fn creation_mass_matches_executed_full_selection() {
    for (case, line) in include_str!("../../../../tests/fixtures/creation_mass.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let cached_kind = words.next().unwrap();
        let present = words.next().unwrap() != 0;
        let mass_override = f32::from_bits(words.next().unwrap());
        let compressed: Vec<u8> = words.by_ref().take(8).flat_map(u32::to_le_bytes).collect();
        let frame: [f32; 14] = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let calculated = PrincipalMassProperties {
            volume: frame[0],
            mass: frame[1],
            center: frame[2..6].try_into().unwrap(),
            inertia: frame[6..10].try_into().unwrap(),
            orientation: frame[10..14].try_into().unwrap(),
        };
        let distribution = MassDistribution {
            center_of_mass_and_volume: std::array::from_fn(|_| {
                f32::from_bits(words.next().unwrap())
            }),
            major_axis_space: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
            inertia_tensor: std::array::from_fn(|_| f32::from_bits(words.next().unwrap())),
        };
        let _old: [u32; 16] = std::array::from_fn(|_| words.next().unwrap());
        let expected: [u32; 16] = std::array::from_fn(|_| words.next().unwrap());
        let expected_calls = words.next().unwrap();
        let mut calls = 0;
        let actual = creation_mass_properties(
            if cached_kind == 1 {
                Some(decode_principal(&compressed).unwrap())
            } else {
                None
            },
            present.then_some(distribution),
            mass_override,
            |config| {
                calls += 1;
                assert_eq!(
                    config,
                    std::array::from_fn(|_| words.next().unwrap()),
                    "case {case} shape config"
                );
                Ok(calculated)
            },
        )
        .unwrap();
        assert_eq!(calls, expected_calls, "case {case} virtual mass call count");
        let actual: Vec<u32> = [actual.volume, actual.mass]
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
        assert_eq!(actual, expected, "case {case} defined mass fields");
        assert!(words.next().is_none());
    }
}
