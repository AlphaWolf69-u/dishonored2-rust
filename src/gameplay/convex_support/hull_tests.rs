use super::{faces::FaceEdge, hull::*, penetration::ConvexSupportPoint};
fn snapshot(hull: &PenetrationHull, outcome: ExpansionOutcome) -> Vec<u32> {
    let edge = |value: Option<FaceEdge>| {
        value.map_or([u32::MAX; 2], |edge| [edge.face as u32, edge.edge as u32])
    };
    let mut result = match outcome {
        ExpansionOutcome::Continue => vec![0, 1],
        ExpansionOutcome::Stopped(status) => vec![1, status],
    };
    result.extend([
        hull.horizon.lower.to_bits(),
        hull.horizon.upper.to_bits(),
        hull.faces.len() as u32,
        hull.horizon.retired.len() as u32,
    ]);
    result.extend(hull.horizon.retired.iter().map(|&i| i as u32));
    for face in &hull.faces {
        result.extend(face.vertices.map(|i| i as u32));
        result.extend(face.opposite.into_iter().flat_map(|e| edge(Some(e))));
        result.extend(
            face.normal
                .into_iter()
                .chain([face.distance])
                .map(f32::to_bits),
        );
        result.push(face.mark);
    }
    result.extend(hull.horizon.boundary_links.iter().copied().flat_map(edge));
    result
}
#[test]
fn penetration_expansion_matches_executed_hull_update() {
    fn words(line: &str) -> Vec<u32> {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect()
    }
    let inputs: Vec<_> = include_str!("../../../tests/fixtures/penetration_faces.txt")
        .lines()
        .map(words)
        .collect();
    for line in include_str!("../../../tests/fixtures/penetration_expansion.txt").lines() {
        let w = words(line);
        let case = w[0] as usize;
        let input = &inputs[case];
        let points = std::array::from_fn(|i| ConvexSupportPoint {
            difference: std::array::from_fn(|j| f32::from_bits(input[i * 4 + j])),
            query: [0.; 4],
            target: [0.; 4],
        });
        let mut hull = PenetrationHull::new(points);
        hull.vertices.push(ConvexSupportPoint {
            difference: std::array::from_fn(|i| f32::from_bits(w[8 + i])),
            query: [0.; 4],
            target: [0.; 4],
        });
        hull.vertex_count = w[2] as usize;
        let outcome = hull
            .expand(
                4,
                (w[1] != u32::MAX).then_some(w[1] as usize),
                std::array::from_fn(|i| f32::from_bits(w[4 + i])),
                [f32::from_bits(w[3]); 4],
            )
            .unwrap();
        assert_eq!(
            snapshot(&hull, outcome),
            w[12..],
            "penetration expansion case{case}"
        );
    }
}

#[test]
fn penetration_expansion_streams_match_capacity_and_rollback() {
    let mut hull = None;
    let mut run = 0;
    let mut stopped = 0;
    for line in include_str!("../../../tests/fixtures/penetration_expansion_stream.txt").lines() {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        if w[0] == 0 {
            run = w[1];
            hull = Some(PenetrationHull::new(std::array::from_fn(|i| {
                ConvexSupportPoint {
                    difference: std::array::from_fn(|j| f32::from_bits(w[2 + i * 4 + j])),
                    query: [0.; 4],
                    target: [0.; 4],
                }
            })));
            continue;
        }
        assert_eq!(w[1], run);
        let hull = hull.as_mut().unwrap();
        hull.vertices.push(ConvexSupportPoint {
            difference: std::array::from_fn(|i| f32::from_bits(w[9 + i])),
            query: [0.; 4],
            target: [0.; 4],
        });
        hull.vertex_count = hull.vertices.len();
        let outcome = hull
            .expand(
                hull.vertex_count - 1,
                Some(w[3] as usize),
                std::array::from_fn(|i| f32::from_bits(w[5 + i])),
                [f32::from_bits(w[4]); 4],
            )
            .unwrap();
        assert_eq!(
            snapshot(hull, outcome),
            w[13..],
            "hull stream{run} step{}",
            w[2]
        );
        if let ExpansionOutcome::Stopped(status) = outcome {
            assert_eq!(status, 3);
            assert_eq!(hull.faces.len(), 90);
            stopped += 1;
        }
    }
    assert_eq!(stopped, 3);
}
