use super::*;
#[test]
fn lod_variants_match_executed_constructor_and_masked_rules() {
    for (case, line) in include_str!("../../../tests/fixtures/body_lod.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let enabled = words.next().unwrap() != 0;
        let mut config = BodyLodConfig::default();
        assert_eq!(
            [
                u32::from(config.register_default_config)
                    | (u32::from(config.auto_build_on_dynamic_body_added) << 8)
                    | (u32::from(config.auto_build_on_mesh_body_added) << 16),
                config.accuracy.to_bits(),
                config.slow_to_fast_threshold.to_bits(),
                config.fast_to_slow_threshold.to_bits(),
                config.body_is_big_threshold.to_bits(),
                config.average_velocity_gain.to_bits()
            ],
            std::array::from_fn(|_| words.next().unwrap())
        );
        config.register_default_config = enabled;
        let mut variants = LodVariants::new(config);
        assert_eq!(
            variants
                .table()
                .iter()
                .flatten()
                .map(|&v| u32::from(v))
                .collect::<Vec<_>>(),
            words.by_ref().take(64).collect::<Vec<_>>(),
            "case {case} constructor"
        );
        let count = words.next().unwrap();
        for rule in 0..count {
            let args: [u32; 6] = std::array::from_fn(|_| words.next().unwrap());
            variants.set_matching(
                args[0],
                args[1],
                args[2],
                args[3],
                args[4] as u8,
                args[5] as u8,
            );
            assert_eq!(
                variants
                    .table()
                    .iter()
                    .flatten()
                    .map(|&v| u32::from(v))
                    .collect::<Vec<_>>(),
                words.by_ref().take(64).collect::<Vec<_>>(),
                "case {case} rule {rule}"
            );
        }
        assert!(words.next().is_none());
    }
}
