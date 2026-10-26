use super::*;

#[test]
fn plane_publication_matches_complete_executed_water_component() {
    let empty = WaterComponent::default();
    assert!(empty.plane().is_none());
    assert!(empty.owner.entity.is_none() && empty.owner.declaration.is_none());
    assert_eq!(empty.owner.generation, u32::MAX);
    assert_eq!(empty.owner.flags, 0);
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/water_component_transform.txt").lines() {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let flags = words.next().unwrap();
        let previous = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        let bounds =
            std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())));
        let frame: ShapeMatrix =
            std::array::from_fn(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())));
        let mut water = WaterComponent {
            owner: ComponentOwner {
                flags,
                ..Default::default()
            },
            plane: Some(previous),
            ..Default::default()
        };
        let body = CollisionBody {
            shape: 17,
            shape_slots: Some([99; 8]),
            ..Default::default()
        };
        let mut calls = 0;
        let updated = water
            .update_transform(&body, frame, |shape, actual| {
                assert_eq!(shape, 17);
                assert_eq!(
                    actual.map(|r| r.map(f32::to_bits)),
                    frame.map(|r| r.map(f32::to_bits))
                );
                calls += 1;
                Ok(bounds)
            })
            .unwrap();
        assert_eq!(u32::from(updated), calls);
        assert_eq!(calls, words.next().unwrap(), "case {cases}");
        assert_eq!(
            water.plane().unwrap().map(f32::to_bits),
            std::array::from_fn(|_| words.next().unwrap()),
            "case {cases}"
        );
        assert_eq!(water.owner.flags, flags);
        assert!(words.next().is_none());
        cases += 1;
    }
    assert_eq!(cases, 1024);
}
