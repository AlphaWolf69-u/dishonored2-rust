use super::*;
pub(super) fn body(words: &mut impl Iterator<Item = u32>) -> PointQueryBody {
    PointQueryBody {
        bounds: QuantizedBounds {
            min: std::array::from_fn(|_| words.next().unwrap() as u16),
            max: std::array::from_fn(|_| words.next().unwrap() as u16),
        },
        index: words.next().unwrap(),
    }
}

#[test]
fn owned_trees_match_executed_insertion_and_rebuild() {
    for (case, line) in include_str!("../../../tests/fixtures/hybrid_tree_build.txt")
        .lines()
        .enumerate()
    {
        let mut w = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap());
        let kind = w.next().unwrap();
        let count = w.next().unwrap() as usize;
        let axis = w.next().unwrap();
        assert!(axis < 2);
        let mut bodies = Vec::new();
        let mut locations = Vec::new();
        for _ in 0..count {
            bodies.push(body(&mut w));
            locations.push(w.next().unwrap() | 0x1fff_ffff);
        }
        let mut tree = HybridTree::default();
        let mut update = |id: u32, leaf: usize| {
            let at = (id & 0xff_ffff) as usize;
            locations[at] = (locations[at] & 0xe000_0000) | leaf as u32;
        };
        if kind == 0 {
            for body in &bodies {
                tree.insert(*body, &mut update).unwrap();
            }
        } else if kind == 1 {
            let root = tree.allocate_node();
            tree.root = Some(root);
            tree.body_count = count as u32;
            tree.ensure_free_nodes((count / (LEAF_LIMIT >> 2)) * 2 + 1);
            let centers = center_bounds(&bodies);
            tree.rebuild(root, &mut bodies, centers, &mut update)
                .unwrap();
            refresh_branch_bounds(&mut tree.nodes, &mut tree.branch_leaf_counts, root).unwrap();
        } else {
            let seed = if kind == 3 { count / 3 } else { 0 };
            for body in &bodies[..seed] {
                tree.insert(*body, &mut update).unwrap();
            }
            tree.insert_batch(&bodies[seed..], &mut update).unwrap();
        }
        assert_tree_matches(&tree, &locations, &mut w, &case.to_string());
        assert_eq!(w.next(), None, "case {case} trailing data");
    }
}

pub(super) fn assert_tree_matches(
    tree: &HybridTree,
    locations: &[u32],
    w: &mut impl Iterator<Item = u32>,
    case: &str,
) {
    let root = tree.query_tree().root.map_or(0x7fff_ffff, |i| i as u32);
    assert_eq!(root, w.next().unwrap(), "case {case} root");
    let mut active = Vec::new();
    let mut pending: Vec<_> = tree.root.into_iter().collect();
    while let Some(index) = pending.pop() {
        assert!(
            !active.contains(&index),
            "case {case} repeated node {index}"
        );
        active.push(index);
        if let PointNodeChildren::Branch { left, right } = tree.nodes[index].children {
            pending.extend([left, right]);
        }
    }
    active.sort_unstable();
    assert_eq!(
        active.len(),
        w.next().unwrap() as usize,
        "case {case} active nodes"
    );
    let mut blocks = Vec::new();
    for index in active {
        assert_eq!(index, w.next().unwrap() as usize, "case {case} node index");
        let expected_min: [u16; 3] = std::array::from_fn(|_| w.next().unwrap() as u16);
        let expected_max: [u16; 3] = std::array::from_fn(|_| w.next().unwrap() as u16);
        assert_eq!(
            tree.nodes[index].bounds.min, expected_min,
            "case {case} node {index} min"
        );
        assert_eq!(
            tree.nodes[index].bounds.max, expected_max,
            "case {case} node {index} max"
        );
        let branch = w.next().unwrap();
        let left = w.next().unwrap();
        let right = w.next().unwrap();
        match tree.nodes[index].children {
            PointNodeChildren::Leaf { block } => {
                assert_eq!(branch, 0, "case {case} node {index}");
                assert_eq!(
                    (block as u32, 0x7fff_ffff),
                    (left, right),
                    "case {case} leaf {index}"
                );
                blocks.push(block);
            }
            PointNodeChildren::Branch { left: a, right: b } => {
                assert_eq!(branch, 1, "case {case} node {index}");
                assert_eq!(
                    (a as u32, b as u32),
                    (left, right),
                    "case {case} children {index}"
                );
            }
        }
        let cached = if branch != 0 {
            tree.branch_leaf_counts[index]
        } else {
            0
        };
        assert_eq!(
            cached,
            w.next().unwrap(),
            "case {case} branch count {index}"
        );
        assert_eq!(
            tree.parents[index].map_or(0x7fff_ffff, |i| i as u32),
            w.next().unwrap(),
            "case {case} parent {index}"
        );
    }
    blocks.sort_unstable();
    assert_eq!(
        blocks.len(),
        w.next().unwrap() as usize,
        "case {case} blocks"
    );
    for index in blocks {
        assert_eq!(index, w.next().unwrap() as usize, "case {case} block index");
        assert_eq!(
            tree.blocks[index].len(),
            w.next().unwrap() as usize,
            "case {case} block {index} size"
        );
        for actual in &tree.blocks[index] {
            let expected = body(w);
            assert_eq!(
                actual.bounds.min, expected.bounds.min,
                "case {case} block {index}"
            );
            assert_eq!(
                actual.bounds.max, expected.bounds.max,
                "case {case} block {index}"
            );
            assert_eq!(
                actual.index, expected.index,
                "case {case} block {index} order"
            );
        }
    }
    for &actual in locations {
        assert_eq!(actual, w.next().unwrap(), "case {case} body location");
    }
    assert_eq!(
        tree.nodes.len(),
        w.next().unwrap() as usize,
        "case {case} node storage"
    );
    assert_eq!(
        tree.free_nodes.len(),
        w.next().unwrap() as usize,
        "case {case} free nodes"
    );
    for &actual in &tree.free_nodes {
        assert_eq!(
            actual,
            w.next().unwrap() as usize,
            "case {case} free node order"
        );
    }
    assert_eq!(
        tree.blocks.len(),
        w.next().unwrap() as usize,
        "case {case} block storage"
    );
    assert_eq!(
        tree.free_blocks.len(),
        w.next().unwrap() as usize,
        "case {case} free blocks"
    );
    for actual in &tree.free_blocks {
        assert_eq!(
            *actual,
            w.next().unwrap() as usize,
            "case {case} free block order"
        );
    }
    assert_eq!(
        tree.leaf_count,
        w.next().unwrap() as usize,
        "case {case} leaf count"
    );
    assert_eq!(
        tree.body_count(),
        w.next().unwrap(),
        "case {case} body count"
    );
}
