use super::*;

#[test]
fn four_image_overrides_match_executed_publication() {
    let mut cases = 0;
    for (case, row) in include_str!("../../../tests/fixtures/environment_image_overrides.txt")
        .lines()
        .enumerate()
    {
        let words: Vec<u32> = row
            .split_whitespace()
            .map(|word| u32::from_str_radix(word, 16).unwrap())
            .collect();
        assert_eq!(words.len(), 51);
        let image = |word: u32| (word != 0).then_some(word);
        let mut mix = ImageOverrides::default();
        for i in 0..words[0] as usize {
            let offset = 2 + i * 2;
            mix.set_override(image(words[offset]), f32::from_bits(words[offset + 1]));
        }
        for i in 0..words[1] as usize {
            let offset = 18 + i * 3;
            mix.add_environment(
                image(words[offset]),
                f32::from_bits(words[offset + 1]),
                f32::from_bits(words[offset + 2]),
            );
        }
        let result = mix.finish();
        for i in 0..4 {
            assert_eq!(
                result.images[i],
                image(words[42 + i * 2]),
                "case {case}, slot {i}"
            );
            assert_eq!(
                result.weights[i].to_bits(),
                words[43 + i * 2],
                "case {case}, slot {i} weight"
            );
        }
        assert_eq!(
            1 + u32::from(result.is_active()),
            words[50],
            "case {case} fog mode"
        );
        cases += 1;
    }
    assert_eq!(cases, 512);
}
