use super::*;

#[test]
fn compound_traversal_matches_executed_float_tree() {
    compare_traversal(
        include_str!("../../../tests/fixtures/compound_query_tree.txt"),
        include_str!("../../../tests/fixtures/compound_query.txt"),
        false,
        640,
    );
}

#[test]
fn masked_compound_traversal_matches_executed_float_tree() {
    compare_traversal(
        include_str!("../../../tests/fixtures/compound_masked_query_tree.txt"),
        include_str!("../../../tests/fixtures/compound_masked_query.txt"),
        true,
        2560,
    );
}

fn compare_traversal(tree: &str, cases: &str, masked: bool, expected_count: usize) {
    let nodes: Vec<_> = tree
        .lines()
        .map(|line| {
            let w: Vec<_> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            CompoundNode {
                bounds: [
                    [w[0], w[1], w[2]].map(f32::from_bits),
                    [w[3], w[4], w[5]].map(f32::from_bits),
                ],
                children: if w[6] == 0 {
                    CompoundChildren::Instance(w[7] as u16)
                } else {
                    CompoundChildren::Branch {
                        left: w[6] as u16,
                        right: w[7] as u16,
                    }
                },
            }
        })
        .collect();
    struct Visitor {
        enabled: u32,
        mode: u32,
        visited: Vec<u16>,
    }
    impl CompoundQueryVisitor for Visitor {
        fn query_instance(&mut self, instance: u16, squared_distance: &mut f32) {
            if self.enabled >> instance & 1 == 0 {
                return;
            }
            self.visited.push(instance);
            if self.mode != 0 {
                let distance = if self.mode == 1 {
                    0.
                } else {
                    [0., 0.5, 2., f32::from_bits(0x7fc0_0011), -1.][usize::from(instance)]
                };
                let clipped = if 0. > distance { 0. } else { distance };
                *squared_distance = clipped * clipped;
            }
        }
    }
    let mut traversal = CompoundQuery::default();
    let mut count = 0;
    for (case, line) in cases.lines().enumerate() {
        let words: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let (node_mask, w) = if masked {
            ((words[0] != 0).then_some(&words[1..5]), &words[5..])
        } else {
            (None, &words[..])
        };
        let distance = f32::from_bits(w[7]);
        let clipped = if 0. > distance { 0. } else { distance };
        let mut squared = clipped * clipped;
        let mut visitor = Visitor {
            enabled: w[8],
            mode: w[9],
            visited: Vec::new(),
        };
        traversal
            .visit_masked(
                CompoundTree {
                    nodes: &nodes,
                    root: w[0] as u16,
                },
                CompoundQueryBounds {
                    center: [w[1], w[2], w[3]].map(f32::from_bits),
                    half_extent: [w[4], w[5], w[6]].map(f32::from_bits),
                },
                &mut squared,
                node_mask,
                &mut visitor,
            )
            .unwrap();
        assert_eq!(squared.to_bits(), w[10], "source squared radius case{case}");
        assert_eq!(
            visitor.visited.len(),
            w[11] as usize,
            "source visit count case{case}"
        );
        assert_eq!(
            visitor
                .visited
                .iter()
                .map(|&v| u32::from(v))
                .collect::<Vec<_>>(),
            w[12..],
            "source visit order case{case}"
        );
        count += 1;
    }
    assert_eq!(count, expected_count);
}
