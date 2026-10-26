use super::*;
#[test]
fn leaf_body_sort_matches_executed_stable_sort() {
    for (case, line) in include_str!("../../../../tests/fixtures/hybrid_leaf_sort.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let count = words.next().unwrap() as usize;
        let mut read_body = || PointQueryBody {
            bounds: QuantizedBounds {
                min: std::array::from_fn(|_| words.next().unwrap() as u16),
                max: std::array::from_fn(|_| words.next().unwrap() as u16),
            },
            index: words.next().unwrap(),
        };
        let mut bodies: Vec<_> = (0..count).map(|_| read_body()).collect();
        sort_leaf_bodies(&mut bodies);
        for actual in bodies {
            let expected = read_body();
            assert_eq!(actual.bounds.min, expected.bounds.min, "case {case}");
            assert_eq!(actual.bounds.max, expected.bounds.max, "case {case}");
            assert_eq!(actual.index, expected.index, "case {case}");
        }
        assert_eq!(words.next(), None);
    }
}
#[test]
fn insertion_helpers_match_executed_tree_routines() {
    for (case, line) in include_str!("../../../../tests/fixtures/hybrid_insertion.txt")
        .lines()
        .enumerate()
    {
        let mut words = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let size = words.next().unwrap() as usize;
        let expected_count = words.next().unwrap();
        let mut nodes = Vec::new();
        let mut counts = Vec::new();
        for _ in 0..size {
            let min = std::array::from_fn(|_| words.next().unwrap() as u16);
            let max = std::array::from_fn(|_| words.next().unwrap() as u16);
            let left = words.next().unwrap() as usize;
            let right = words.next().unwrap() as usize;
            counts.push(words.next().unwrap());
            nodes.push(PointQueryNode {
                bounds: QuantizedBounds { min, max },
                children: if right == 0x7fffffff {
                    PointNodeChildren::Leaf { block: left }
                } else {
                    PointNodeChildren::Branch { left, right }
                },
            });
        }
        assert_eq!(
            refresh_branch_bounds(&mut nodes, &mut counts, 0).unwrap(),
            expected_count,
            "case {case}"
        );
        for i in 0..size {
            let expected_min: [u16; 3] = std::array::from_fn(|_| words.next().unwrap() as u16);
            let expected_max: [u16; 3] = std::array::from_fn(|_| words.next().unwrap() as u16);
            let count = words.next().unwrap();
            assert_eq!(nodes[i].bounds.min, expected_min, "case {case} node {i}");
            assert_eq!(nodes[i].bounds.max, expected_max, "case {case} node {i}");
            assert_eq!(counts[i], count, "case {case} node {i}");
        }
        let queries = words.next().unwrap();
        for q in 0..queries {
            let root = if words.next().unwrap() == 0 {
                None
            } else {
                Some(0)
            };
            let center = std::array::from_fn(|_| words.next().unwrap() as i32);
            let limit = words.next().unwrap();
            let expected = words.next().unwrap();
            let actual = select_leaf(&nodes, root, center, limit)
                .unwrap()
                .map_or(0x7fffffff, |i| i as u32);
            assert_eq!(actual, expected, "case {case} query {q}");
        }
        assert_eq!(words.next(), None);
    }
}
