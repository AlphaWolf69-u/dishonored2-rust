use super::*;
#[test]
fn numeric_overrides_match_executed_manager() {
    let mut count = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_parameter_overrides.txt")
        .lines()
        .enumerate()
    {
        let w = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let kind = [
            ParameterKind::Boolean,
            ParameterKind::Signed,
            ParameterKind::Unsigned,
            ParameterKind::Float,
        ][w[0] as usize];
        let baseline: [u32; 4] = w[1..5].try_into().unwrap();
        let mut overrides = ParameterOverrides::default();
        let mut at = 7;
        for _ in 0..w[5] {
            overrides.push_override(
                0,
                kind,
                baseline,
                std::array::from_fn(|i| f32::from_bits(w[at + i])),
                f32::from_bits(w[at + 4]),
            );
            at += 5;
        }
        for _ in 0..w[6] {
            overrides.add_environment(
                0,
                kind,
                baseline,
                w[at..at + 4].try_into().unwrap(),
                f32::from_bits(w[at + 4]),
                f32::from_bits(w[at + 5]),
            );
            at += 6;
        }
        let result = overrides.finish().collect::<Vec<_>>();
        assert_eq!(result.len(), w[at] as usize, "case {case} count");
        at += 1;
        for (key, words) in result {
            assert_eq!(key, 0);
            assert_eq!(words, w[at..at + 4], "case {case}, kind{kind:?}");
            at += 4;
        }
        assert_eq!(at, w.len());
        count += 1;
    }
    assert_eq!(count, 512);
}
