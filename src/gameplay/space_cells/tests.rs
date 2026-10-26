use super::*;
#[test]
fn cell_assignment_matches_executed_four_lane_reduction() {
    for (case, line) in include_str!("../../../tests/fixtures/space_cells.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap() as usize;
        let centers: Vec<_> = (0..count)
            .map(|_| std::array::from_fn(|_| f32::from_bits(words.next().unwrap())))
            .collect();
        let point = std::array::from_fn(|_| f32::from_bits(words.next().unwrap()));
        assert_eq!(
            u32::from(SpaceCells::Dynamic(centers).select(point).unwrap()),
            words.next().unwrap(),
            "case {case}"
        );
        assert!(words.next().is_none());
    }
}
#[test]
fn cell_pair_indices_match_initialized_global_table() {
    let mut words = include_str!("../../../tests/fixtures/cell_pairs.txt")
        .split_whitespace()
        .map(|v| u16::from_str_radix(v, 16).unwrap());
    for b in 0..33u8 {
        for a in 0..33u8 {
            assert_eq!(
                cell_pair_index(a.checked_sub(1), b.checked_sub(1)).unwrap(),
                words.next().unwrap(),
                "pair {a}/{b}"
            );
        }
    }
    assert!(words.next().is_none());
}
