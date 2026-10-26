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
fn layer_words_and_block_flags_follow_the_filter_builders() {
    assert_eq!(layer_filter(layer::PLAYER), 0x41c62b << 5 | 6);
    assert_eq!(layer_by_name("COLLISION_LAYER_WORLD_HIGH"), Some(layer::WORLD_HIGH));
    assert_eq!(layer_by_name("COLLISION_LAYER_HIDDEN"), Some(26));
    let world = layer_filter(layer::WORLD_LOW);
    assert_eq!(apply_block_flags(world, block::DEFAULT), world);
    // m_blockPlayer = false only drops PLAYER from the collides-with set.
    assert_eq!(apply_block_flags(world, block::DEFAULT & !block::PLAYER), world & !(1 << (layer::PLAYER + 5)));
    assert_eq!(apply_block_flags(u32::MAX, 0), 0x820001f);
    // Without eBLOCK_NPC, NPC_BODY_RAGDOLL (14) stays only with eBLOCK_RAGDOLL.
    let all = u32::MAX & !31;
    assert_eq!(apply_block_flags(all, block::RAGDOLL | block::PLAYER) >> 5 & 1 << 14, 1 << 14);
    assert_eq!(apply_block_flags(all, block::PLAYER) >> 5 & 1 << 14, 0);
}

#[test]
fn level_words_keep_detailed_world_away_from_the_player() {
    // eotw_p bodies: WORLD_HIGH collides with projectiles and melee only,
    // WORLD_LOW_HIGH includes PLAYER.
    let player = layer_filter(layer::PLAYER) & 0xfbffffff;
    assert!(!filters_collide(player, 0x2010002));
    assert!(filters_collide(player, 0x1802c01));
    assert!(!filters_collide(player, 0x2000));
    let ground = QueryFilter::layers(0x802b, 1 << layer::PLAYER);
    assert!(!ground.accepts(0x3c10702) && ground.accepts(0x3c12f01));
    assert!(QueryFilter::ALL.accepts(0x2010002));
}
