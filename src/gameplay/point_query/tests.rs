use super::*;

#[test]
fn hybrid_traversal_matches_executed_point_query() {
    let words = |line: &str| {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>()
    };
    let mut lines = include_str!("../../../tests/fixtures/hybrid_point_query_tree.txt").lines();
    let p = words(lines.next().unwrap());
    let quantization = PointQuantization {
        offset: std::array::from_fn(|i| f32::from_bits(p[i])),
        scale: std::array::from_fn(|i| f32::from_bits(p[i + 3])),
    };
    let count = words(lines.next().unwrap())[0] as usize;
    let mut storage = Vec::new();
    for _ in 0..count {
        let nodes = (0..7)
            .map(|_| {
                let v = words(lines.next().unwrap());
                PointQueryNode {
                    bounds: QuantizedBounds {
                        min: std::array::from_fn(|i| v[i] as u16),
                        max: std::array::from_fn(|i| v[i + 3] as u16),
                    },
                    children: if v[7] == i32::MAX as u32 {
                        PointNodeChildren::Leaf {
                            block: v[6] as usize,
                        }
                    } else {
                        PointNodeChildren::Branch {
                            left: v[6] as usize,
                            right: v[7] as usize,
                        }
                    },
                }
            })
            .collect::<Vec<_>>();
        let blocks = (0..4)
            .map(|_| {
                let count = words(lines.next().unwrap())[0];
                (0..count)
                    .map(|_| {
                        let v = words(lines.next().unwrap());
                        PointQueryBody {
                            bounds: QuantizedBounds {
                                min: std::array::from_fn(|i| v[i] as u16),
                                max: std::array::from_fn(|i| v[i + 3] as u16),
                            },
                            index: v[6],
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        storage.push((nodes, blocks));
    }
    assert!(lines.next().is_none());
    let trees = storage
        .iter()
        .map(|(nodes, blocks)| PointQueryTree {
            root: Some(0),
            nodes,
            blocks,
        })
        .collect::<Vec<_>>();
    struct Visitor {
        trees: u32,
        bodies: u32,
        after: usize,
        mode: u32,
        visits: Vec<(u32, u32)>,
    }
    impl PointQueryVisitor for Visitor {
        fn accepts_tree(&mut self, index: usize) -> bool {
            self.trees & (1 << index) != 0
        }
        fn accepts_body(&mut self, index: u32) -> bool {
            self.bodies & (1 << index) != 0
        }
        fn query_shape(&mut self, body: u32, control: &mut PointQueryControl) {
            self.visits.push((body, control.distance.to_bits()));
            if self.mode != 0 {
                control.distance = match self.mode {
                    1 => 0.5 / self.visits.len() as f32,
                    2 => f32::NAN,
                    _ => -0.25,
                };
            }
            if self.after != 0 && self.visits.len() >= self.after {
                control.early_out = true;
            }
        }
    }
    let mut query = HybridPointQuery::default();
    let mut cases = 0;
    let mut total = 0;
    for line in include_str!("../../../tests/fixtures/hybrid_point_query.txt").lines() {
        let v = words(line);
        let mut visitor = Visitor {
            trees: v[5],
            bodies: v[6],
            after: v[7] as usize,
            mode: v[8],
            visits: Vec::new(),
        };
        let mut control = PointQueryControl {
            distance: f32::from_bits(v[4]),
            early_out: false,
        };
        query
            .visit(
                &trees,
                quantization,
                std::array::from_fn(|i| f32::from_bits(v[i])),
                f32::from_bits(v[3]),
                &mut control,
                &mut visitor,
            )
            .unwrap();
        let n = v[9] as usize;
        let expected = v[10..10 + n * 2]
            .chunks_exact(2)
            .map(|p| (p[0], p[1]))
            .collect::<Vec<_>>();
        assert_eq!(visitor.visits, expected, "case{cases}");
        let distance = f32::from_bits(v[10 + n * 2]);
        assert_eq!(control.distance.to_bits(), distance.to_bits(), "case{cases}");
        assert_eq!(control.early_out, v[11 + n * 2] != 0, "case{cases}");
        total += n;
        cases += 1;
    }
    assert_eq!(cases, 268);
    assert!(total > 0);
}
