use super::{feature_query::*, penetration::PenetrationSupport};
#[test]
fn penetration_feature_query_matches_executed_wrapper() {
    let inputs: Vec<Vec<Vec<u32>>> = [
        include_str!("../../../tests/fixtures/penetration_seed_degenerate.txt"),
        include_str!("../../../tests/fixtures/penetration_solver_shapes.txt"),
    ]
    .into_iter()
    .map(|text| {
        text.lines()
            .map(|line| {
                line.split_whitespace()
                    .map(|v| u32::from_str_radix(v, 16).unwrap())
                    .collect()
            })
            .collect()
    })
    .collect();
    for (case, line) in include_str!("../../../tests/fixtures/penetration_feature_query.txt")
        .lines()
        .enumerate()
    {
        let expected: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let w = &inputs[expected[0] as usize][expected[1] as usize];
        let ac = w[0] as usize;
        let bc = w[1] as usize;
        let vertices: Vec<[f32; 4]> = w[27..27 + (ac + bc) * 4]
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|i| f32::from_bits(p[i])))
            .collect();
        let v = |offset| std::array::from_fn(|i| f32::from_bits(w[offset + i]));
        let start = 27 + (ac + bc) * 4;
        let mut features = ConvexFeatures {
            query: std::array::from_fn(|i| v(start + i * 4)),
            target: std::array::from_fn(|i| v(start + 16 + i * 4)),
            query_count: expected[2] as usize,
            target_count: expected[3] as usize,
        };
        let support = PenetrationSupport {
            query: &vertices[..ac],
            target: &vertices[ac..],
            relative: std::array::from_fn(|i| v(2 + i * 4)),
        };
        let result = query_penetration(&mut features, &support).unwrap();
        let mut actual = vec![
            result.status as u32,
            features.query_count as u32,
            features.target_count as u32,
        ];
        actual.extend(
            features
                .query
                .into_iter()
                .flatten()
                .chain(features.target.into_iter().flatten())
                .map(f32::to_bits),
        );
        actual.extend(
            result
                .distance
                .into_iter()
                .chain(result.normal)
                .chain(result.witness)
                .chain(result.normal)
                .map(f32::to_bits),
        );
        assert_eq!(actual.len(), expected.len() - 4);
        for (lane, (&got, &want)) in actual.iter().zip(&expected[4..]).enumerate() {
            assert_eq!(
                got, want,
                "case {case} source {} input {} lane {lane}: {got:08x} != {want:08x}",
                expected[0], expected[1]
            );
        }
    }
}
