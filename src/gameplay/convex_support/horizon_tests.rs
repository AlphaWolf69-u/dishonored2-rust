use super::{faces::tetrahedron_faces, horizon::*};
#[test]
fn penetration_horizons_match_executed_walk() {
    fn words(line: &str) -> Vec<u32> {
        line.split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect()
    }
    let inputs: Vec<_> = include_str!("../../../tests/fixtures/penetration_faces.txt")
        .lines()
        .map(words)
        .collect();
    for line in include_str!("../../../tests/fixtures/penetration_horizon.txt").lines() {
        let w = words(line);
        let case = w[0] as usize;
        let input = &inputs[case];
        let points =
            std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(input[i * 4 + j])));
        let mut faces = tetrahedron_faces(points);
        for i in 0..4 {
            faces[i].mark = w[9 + i];
        }
        let mut state = HorizonWorkspace::new(4, f32::from_bits(w[3]), f32::from_bits(w[4]));
        state.retired.resize(w[2] as usize, 0);
        let status = state
            .collect(
                &points,
                &mut faces,
                w[1] as usize,
                std::array::from_fn(|i| f32::from_bits(w[5 + i])),
            )
            .unwrap();
        let edge = |value: Option<super::faces::FaceEdge>| {
            value.map_or([u32::MAX; 2], |edge| [edge.face as u32, edge.edge as u32])
        };
        let mut result = vec![
            if status == HorizonStatus::Complete {
                0
            } else {
                2
            },
            state.lower.to_bits(),
            state.upper.to_bits(),
            state.boundary_count as u32,
        ];
        result.extend(edge(state.last_edge));
        result.push(state.retired.len() as u32);
        result.extend(state.retired.into_iter().map(|i| i as u32));
        for face in faces {
            result.extend([face.mark, face.distance.to_bits()]);
        }
        result.extend(state.boundary_links.into_iter().flat_map(edge));
        assert_eq!(result, w[13..], "penetration horizon case{case}");
    }
}
