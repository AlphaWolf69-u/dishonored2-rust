use super::*;
fn parameter(kind: u32, words: [u32; 4]) -> EnvironmentValue {
    if kind == 0 {
        EnvironmentValue::Boolean(words[0] != 0)
    } else {
        EnvironmentValue::Float(Vec4::from_array(words.map(f32::from_bits)))
    }
}
fn prepared(value: EnvironmentValue) -> EnvironmentParameters {
    EnvironmentParameters {
        values: [("sample".into(), value)].into_iter().collect(),
    }
}
#[test]
fn prepared_overrides_match_executed_float_and_bool_cases() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_parameter_overrides.txt")
        .lines()
        .enumerate()
    {
        let w = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        if w[0] != 0 && w[0] != 3 {
            continue;
        }
        let mut frame = prepared(parameter(w[0], w[1..5].try_into().unwrap()));
        let mut at = 7;
        let mut values = Vec::new();
        let mut weights = Vec::new();
        for _ in 0..w[5] {
            values.push(std::array::from_fn(|i| f32::from_bits(w[at + i])));
            weights.push(f32::from_bits(w[at + 4]));
            at += 5;
        }
        let mut profiles = Vec::new();
        let mut factors = Vec::new();
        for _ in 0..w[6] {
            profiles.push(prepared(parameter(w[0], w[at..at + 4].try_into().unwrap())));
            factors.push((f32::from_bits(w[at + 4]), f32::from_bits(w[at + 5])));
            at += 6;
        }
        let explicit = values
            .into_iter()
            .zip(weights)
            .map(|(value, weight)| NumericOverride {
                name: "sample",
                value,
                weight,
            })
            .collect::<Vec<_>>();
        let weighted = profiles
            .iter()
            .zip(factors)
            .map(|(parameters, (weight, influence))| WeightedEnvironment {
                parameters,
                weight,
                influence,
            })
            .collect::<Vec<_>>();
        frame.apply_numeric_overrides(&explicit, &weighted).unwrap();
        let n = w[at] as usize;
        let expected: [u32; 4] = if n == 0 {
            w[1..5].try_into().unwrap()
        } else {
            w[at + 1 + (n - 1) * 4..at + 1 + n * 4].try_into().unwrap()
        };
        match frame.values["sample"] {
            EnvironmentValue::Float(value) => {
                assert_eq!(value.to_array().map(f32::to_bits), expected, "case {case}")
            }
            EnvironmentValue::Boolean(value) => assert_eq!(value, expected[0] != 0, "case {case}"),
            _ => unreachable!(),
        }
        count += 1;
    }
    assert_eq!(count, 256);
}
#[test]
fn overrides_do_not_feed_back_into_ordinary_tracks() {
    let world = prepared(EnvironmentValue::Float(Vec4::splat(2.)));
    let profile = prepared(EnvironmentValue::Float(Vec4::splat(10.)));
    let mut tracks = EnvironmentTracks::new(&world);
    let frame = tracks
        .update_with_overrides(
            &world,
            0.,
            &[],
            0.1,
            &[],
            &[WeightedEnvironment {
                parameters: &profile,
                weight: 0.5,
                influence: 1.,
            }],
        )
        .unwrap();
    assert_eq!(frame.vector("sample").unwrap(), [6.; 4]);
    assert_eq!(
        tracks
            .update(&world, 0., &[], 0.1)
            .unwrap()
            .vector("sample")
            .unwrap(),
        [2.; 4]
    );
}
