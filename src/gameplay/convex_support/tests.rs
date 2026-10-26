use super::*;

#[test]
fn support_preparation_matches_executed_convex_wrapper() {
    let mut workspace = ConvexSupportWorkspace::default();
    let mut cases = 0;
    for line in include_str!("../../../tests/fixtures/convex_support.txt").lines() {
        let v = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect::<Vec<_>>();
        let count = v[1] as usize;
        let vertices = v[11..11 + count * 4]
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|i| f32::from_bits(p[i])))
            .collect::<Vec<_>>();
        let prepared = workspace
            .prepare(ConvexSupportInput {
                shape_type: v[0] as u8,
                vertices: &vertices,
                radius: f32::from_bits(v[2]),
                scaled: (v[3] != 0).then(|| ConvexSupportScale {
                    scale: std::array::from_fn(|i| f32::from_bits(v[4 + i])),
                    offset: std::array::from_fn(|i| f32::from_bits(v[7 + i])),
                    radius: f32::from_bits(v[10]),
                }),
            })
            .unwrap();
        let expected = &v[11 + count * 4..];
        assert_eq!(prepared.kind as u32, expected[0], "case{cases}");
        assert_eq!(prepared.vertices.len() as u32, expected[1], "case{cases}");
        assert_eq!(prepared.radius.to_bits(), expected[2], "case{cases}");
        let actual = prepared
            .vertices
            .iter()
            .flatten()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected[3..], "case{cases}");
        if v[3] == 0 {
            assert_eq!(prepared.vertices.as_ptr(), vertices.as_ptr());
        }
        cases += 1;
    }
    assert_eq!(cases, 224);
}
#[test]
fn transformed_vertex_bounds_match_executed_kernel() {
    for (case, line) in include_str!("../../../tests/fixtures/convex_vertex_bounds.txt")
        .lines()
        .enumerate()
    {
        let words: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let count = words[0] as usize;
        let columns =
            std::array::from_fn(|i| std::array::from_fn(|j| f32::from_bits(words[1 + i * 4 + j])));
        let vertices: Vec<_> = (0..count)
            .map(|i| std::array::from_fn(|j| f32::from_bits(words[17 + i * 4 + j])))
            .collect();
        let bounds = super::transformed_vertex_bounds(&vertices, columns);
        let bits: Vec<_> = bounds.iter().flatten().map(|v| v.to_bits()).collect();
        assert_eq!(bits, words[17 + count * 4..], "executed bounds case {case}");
    }
}
#[test]
fn convex_scaling_matches_executed_kernel() {
    for (case, line) in include_str!("../../../tests/fixtures/convex_scaling.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let count = w[1] as usize;
        let mut scale = std::array::from_fn(|i| f32::from_bits(w[3 + i]));
        let mut radius = f32::from_bits(w[7]);
        let vertices: Vec<_> = (0..count)
            .map(|i| std::array::from_fn(|j| f32::from_bits(w[8 + i * 4 + j])))
            .collect();
        let offset =
            super::scaling::adjust_scale(w[0] as u8, &vertices, w[2] != 0, &mut scale, &mut radius)
                .unwrap();
        let result: Vec<_> = scale
            .into_iter()
            .chain([radius])
            .chain(offset)
            .map(f32::to_bits)
            .collect();
        assert_eq!(result, w[8 + count * 4..], "convex scaling case {case}");
    }
}
#[test]
fn distance_publication_matches_executed_wrapper() {
    use super::distance::*;
    for (case, line) in include_str!("../../../tests/fixtures/convex_distance_output.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let result = finish_distance(
            ConvexDistanceSolution {
                status: w[0] as i32,
                distance: vector(20),
                normal: vector(24),
                witness: vector(28),
            },
            f32::from_bits(w[1]),
            f32::from_bits(w[2]),
            std::array::from_fn(|i| vector(4 + i * 4)),
            f32::from_bits(w[3]),
        );
        assert_eq!(
            result.is_some(),
            w[32] != 0,
            "distance admission case {case}"
        );
        if let Some(result) = result {
            let bits: Vec<_> = result
                .point
                .into_iter()
                .chain(result.normal)
                .chain(result.distance)
                .map(f32::to_bits)
                .collect();
            assert_eq!(bits, w[33..], "distance output case {case}");
        }
    }
}
#[test]
fn support_vertices_match_executed_pair_kernel() {
    for (case, line) in include_str!("../../../tests/fixtures/convex_support_vertices.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<_> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let ac = w[0] as usize;
        let bc = w[1] as usize;
        let vector = |at: usize| std::array::from_fn(|i| f32::from_bits(w[at + i]));
        let a: Vec<_> = (0..ac).map(|i| vector(22 + i * 4)).collect();
        let b: Vec<_> = (0..bc).map(|i| vector(22 + (ac + i) * 4)).collect();
        let pair = super::vertices::support_pair(
            &a,
            &b,
            vector(2),
            std::array::from_fn(|i| vector(6 + i * 4)),
        )
        .unwrap();
        let bits: Vec<_> = pair
            .query
            .into_iter()
            .chain(pair.target)
            .chain(pair.target_in_query)
            .map(f32::to_bits)
            .collect();
        assert_eq!(
            bits,
            w[22 + (ac + bc) * 4..],
            "support vertex pair case {case}"
        );
    }
}
