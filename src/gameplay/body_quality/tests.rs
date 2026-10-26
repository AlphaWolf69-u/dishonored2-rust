use super::*;
#[test]
fn quality_presets_match_executed_library_initialization() {
    for (case, line) in include_str!("../../../tests/fixtures/body_quality.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let defaults = BodyQualityConfig::default();
        assert_eq!(
            [
                u32::from(defaults.enable_welding_for_default_objects),
                u32::from(defaults.enable_welding_for_critical_objects),
                u32::from(defaults.enable_contact_caching)
            ],
            std::array::from_fn(|_| words.next().unwrap())
        );
        let config = BodyQualityConfig {
            enable_welding_for_default_objects: words.next().unwrap() != 0,
            enable_welding_for_critical_objects: words.next().unwrap() != 0,
            enable_contact_caching: words.next().unwrap() != 0,
        };
        let library = BodyQualityLibrary::new(config);
        for (index, quality) in library.qualities().iter().enumerate() {
            assert_eq!(
                [
                    quality.priority,
                    quality.supported_flags,
                    quality.requested_flags,
                    quality
                        .contact_caching_relative_movement_threshold
                        .to_bits()
                ],
                std::array::from_fn(|_| words.next().unwrap()),
                "case {case} quality {index}"
            );
        }
        assert!(words.next().is_none());
    }
}
