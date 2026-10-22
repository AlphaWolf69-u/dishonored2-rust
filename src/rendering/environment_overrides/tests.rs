use super::super::environment_images::ImageOverrides;
use super::*;

#[test]
fn override_slots_match_executed_lifecycle_and_image_publication() {
    let mut slots = EnvironmentOverrides::default();
    let mut blend = ImageOverrides::default().finish();
    let mut cases = 0;
    let image = |word| (word != 0).then_some(word);
    let id = |word| (word != u32::MAX).then_some(word as usize);
    for (case, row) in include_str!("../../../tests/fixtures/environment_override_slots.txt")
        .lines()
        .enumerate()
    {
        let words: Vec<u32> = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        assert_eq!(words.len(), 78);
        let mut outcome = None;
        match words[0] {
            0 => {
                slots = EnvironmentOverrides::default();
                blend = ImageOverrides::default().finish();
            }
            1 => outcome = slots.register(image(words[1])),
            2 => slots.set_weights(
                id(words[1]),
                f32::from_bits(words[2]),
                f32::from_bits(words[3]),
            ),
            3 => slots.retire(id(words[1])),
            4 => {
                let mut mix = ImageOverrides::default();
                slots.visit_frame(|image, weight, influence| {
                    mix.add_environment(Some(*image), weight, influence)
                });
                blend = mix.finish();
            }
            op => panic!("unknown fixture operation {op}"),
        }
        assert_eq!(outcome, id(words[4]), "case {case} registration");
        for (i, slot) in slots.slots.iter().enumerate() {
            let at = 5 + i * 4;
            assert_eq!(
                slot.environment,
                image(words[at]),
                "case {case} slot {i} environment"
            );
            assert_eq!(
                slot.weight.to_bits(),
                words[at + 1],
                "case {case} slot {i} weight"
            );
            assert_eq!(
                slot.influence.to_bits(),
                words[at + 2],
                "case {case} slot {i} influence"
            );
            assert_eq!(
                slot.retiring,
                words[at + 3] != 0,
                "case {case} slot {i} retirement"
            );
        }
        for i in 0..4 {
            assert_eq!(
                blend.images[i],
                image(words[69 + i * 2]),
                "case {case} published image {i}"
            );
            assert_eq!(
                blend.weights[i].to_bits(),
                words[70 + i * 2],
                "case {case} published weight {i}"
            );
        }
        assert_eq!(
            1 + u32::from(blend.is_active()),
            words[77],
            "case {case} fog mode"
        );
        cases += 1;
    }
    assert_eq!(cases, 512);
}
