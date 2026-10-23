use super::*;
use crate::assets::physworld::polytope::{PolytopeConnectivity, PolytopeFace};
use std::collections::HashMap;

fn synthetic_shape(id: u32) -> ConvexPolytope {
    let cube = vec![
        [-1., -1., -1.],
        [1., -1., -1.],
        [1., 1., -1.],
        [-1., 1., -1.],
        [-1., -1., 1.],
        [1., -1., 1.],
        [1., 1., 1.],
        [-1., 1., 1.],
    ];
    let (vertices, polygons) = if id == 1 {
        (
            vec![[0., 0., 0.], [2., 0., 0.], [0.5, 3., 0.], [0.25, 0.5, 4.]],
            vec![vec![0, 2, 1], vec![0, 1, 3], vec![1, 2, 3], vec![2, 0, 3]],
        )
    } else {
        (
            cube.into_iter()
                .map(|[x, y, z]| {
                    if id == 2 {
                        [
                            if x > 0. { 1.9 } else { -1.5 },
                            if y > 0. { 0.8 } else { 1.4 },
                            if z > 0. { 1.5 } else { -2.9 },
                        ]
                    } else {
                        [x, y, z]
                    }
                })
                .collect(),
            vec![
                vec![0, 3, 2, 1],
                vec![4, 5, 6, 7],
                vec![0, 1, 5, 4],
                vec![3, 7, 6, 2],
                vec![0, 4, 7, 3],
                vec![1, 2, 6, 5],
            ],
        )
    };
    let mut faces = Vec::new();
    let mut indices = Vec::new();
    let mut edges = HashMap::new();
    let mut anchors = vec![None; vertices.len()];
    for (face, polygon) in polygons.iter().enumerate() {
        faces.push(PolytopeFace {
            first_index: indices.len() as u16,
            num_indices: polygon.len() as u8,
            min_half_angle: 0,
        });
        for (edge, &a) in polygon.iter().enumerate() {
            let b = polygon[(edge + 1) % polygon.len()];
            let half = PolytopeEdge {
                face: face as u16,
                edge: edge as u8,
            };
            edges.insert((a, b), half);
            anchors[a as usize].get_or_insert(half);
        }
        indices.extend(polygon);
    }
    let mut links = Vec::new();
    for polygon in &polygons {
        for (edge, &a) in polygon.iter().enumerate() {
            let b = polygon[(edge + 1) % polygon.len()];
            links.push(edges[&(b, a)]);
        }
    }
    ConvexPolytope {
        vertices: vertices
            .into_iter()
            .map(|[x, y, z]| [x, y, z, 0.5])
            .collect(),
        planes: Vec::new(),
        faces,
        indices,
        radius: 0.,
        connectivity: Some(PolytopeConnectivity {
            vertex_edges: anchors.into_iter().map(Option::unwrap).collect(),
            face_links: links,
        }),
    }
}

#[test]
fn vertex_ring_hints_match_executed_polytope_routine() {
    let shapes: Vec<_> = (0..3).map(synthetic_shape).collect();
    let fixture = include_str!("../../../tests/fixtures/polytope_vertex_hint.txt");
    let mut count = 0;
    for (line, row) in fixture.lines().enumerate() {
        let row: Vec<_> = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        assert_eq!(row.len(), 6);
        let point = [row[2], row[3], row[4]].map(f32::from_bits);
        let result = vertex_edge_hint(&shapes[row[0] as usize], row[1] as usize, point).unwrap();
        assert_eq!(result.to_bits(), row[5], "executed case {line}");
        count += 1;
    }
    assert_eq!(count, 2400);
}

#[test]
fn zero_distance_queries_match_executed_point_polytope_kernel() {
    let mut shapes = [synthetic_shape(0), synthetic_shape(1)];
    shapes[1].vertices = vec![
        [0., 0., 0., 0.5],
        [1., 0., 0., 0.5],
        [0., 1., 0., 0.5],
        [0., 0., 1., 0.5],
    ];
    let mut count = 0;
    for row in include_str!("../../../tests/fixtures/polytope_zero_distance.txt").lines() {
        let words: Vec<_> = row.split_whitespace().collect();
        let shape: usize = words[1].parse().unwrap();
        let data: Vec<_> = words[2..]
            .iter()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        match words[0] {
            "planes" => {
                shapes[shape].planes = data
                    .chunks_exact(4)
                    .map(|p| [p[0], p[1], p[2], p[3]].map(f32::from_bits))
                    .collect();
            }
            "point" => {
                assert_eq!(data.len(), 4);
                let point = [data[0], data[1], data[2]].map(f32::from_bits);
                assert_eq!(
                    zero_distance_point_query(&shapes[shape], point).unwrap(),
                    Some(data[3] != 0),
                    "executed point {count}"
                );
                count += 1;
            }
            other => panic!("unknown fixture record {other}"),
        }
    }
    assert_eq!(count, 6086);
}
