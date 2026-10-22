use super::*;

#[test]
fn history_matches_executed_fog_owner() {
    let mut history = FogLookupHistory::new(1.);
    for (frame, line) in include_str!("../../../../tests/fixtures/fog_lookup_history.txt")
        .lines()
        .enumerate()
    {
        let words = line
            .split_whitespace()
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(words.len(), 11);
        let plan = history.prepare(
            f32::from_bits(words[0]),
            std::array::from_fn(|i| f32::from_bits(words[i + 1])),
        );
        assert_eq!(
            [plan.source, plan.output, plan.final_image],
            [words[5] as usize, words[6] as usize, words[7] as usize],
            "frame {frame}"
        );
        assert_eq!(plan.weight.to_bits(), words[8], "weight frame {frame}");
        assert_eq!(
            history.previous_weight.to_bits(),
            words[9],
            "raw weight frame {frame}"
        );
        assert_eq!(
            plan.apply_overrides,
            words[10] != 0,
            "override frame {frame}"
        );
    }
}
