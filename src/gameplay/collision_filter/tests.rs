use super::*;

#[test]
fn body_and_pair_filters_match_executed_predicates() {
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/collision_query_filter.txt").lines() {
        let v = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(v.len(), 7);
        let filter = CollisionQueryFilter {
            layers: v[0],
            required_flags: v[1],
        };
        assert_eq!(filter.accepts(v[2]), v[4] != 0, "case{cases}");
        assert_eq!(
            filter.accepts_pair(v[2], v[3], false),
            v[5] != 0,
            "case{cases}"
        );
        assert_eq!(
            filter.accepts_pair(v[2], v[3], true),
            v[6] != 0,
            "case{cases}"
        );
        cases += 1;
    }
    assert_eq!(cases, 2048);
}

#[test]
fn layer_filters_match_executed_rules() {
    let mut lines = include_str!("../../../tests/fixtures/collision_layers.txt").lines();
    let words: Vec<u32> = lines.next().unwrap().split_whitespace().map(|v| u32::from_str_radix(v, 16).unwrap()).collect();
    assert_eq!(words.len(), COLLISION_LAYERS.len());
    for (layer, word) in words.iter().enumerate() {
        assert_eq!(filter_info(layer as u32), *word, "layer {layer}");
    }
    for (a, row) in lines.enumerate() {
        for (b, enabled) in row.bytes().enumerate() {
            assert_eq!(pair_collides(words[a], words[b]), enabled == b'1', "layers {a} {b}");
        }
    }
    assert!(!layer_collides(layer_index("COLLISION_LAYER_HIDDEN").unwrap()));
    assert!(layer_collides(layer_index("COLLISION_LAYER_RIGID_BODY").unwrap()));
}
