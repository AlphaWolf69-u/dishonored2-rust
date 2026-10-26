use super::{penetration::*, seed::*, solver::*};
#[test]
fn penetration_solver_matches_complete_executed_search() {
    let inputs: Vec<Vec<Vec<u32>>> = [
        include_str!("../../../tests/fixtures/penetration_seed.txt"),
        include_str!("../../../tests/fixtures/penetration_seed_degenerate.txt"),
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
    for (case, line) in include_str!("../../../tests/fixtures/penetration_solver.txt")
        .lines()
        .enumerate()
    {
        let expected: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let source = expected[0] as usize;
        let raw = &inputs[source][expected[1] as usize];
        let mut w = raw.clone();
        let (query, target) = if source == 0 {
            let target: Vec<[f32; 4]> = if w[0] == 0 {
                (0..8)
                    .map(|i| {
                        [
                            ((i & 1) * 2) as f32 - 1.,
                            (((i >> 1) & 1) * 2) as f32 - 1.,
                            (((i >> 2) & 1) * 2) as f32 - 1.,
                            f32::from_bits(0x3f00_0000 + i),
                        ]
                    })
                    .collect()
            } else {
                [[0., 0., 0.], [2., 0., 0.], [0.5, 3., 0.], [0.25, 0.5, 4.]]
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| [p[0], p[1], p[2], f32::from_bits(0x3f00_0000 + i as u32)])
                    .collect()
            };

            (
                vec![std::array::from_fn(|i| f32::from_bits(w[27 + i]))],
                target,
            )
        } else {
            let ac = w[0] as usize;
            let bc = w[1] as usize;
            let vertices: Vec<[f32; 4]> = w[27..27 + (ac + bc) * 4]
                .chunks_exact(4)
                .map(|p| std::array::from_fn(|i| f32::from_bits(p[i])))
                .collect();
            let mut normalized = w[..27].to_vec();
            normalized.extend(&w[27..31]);
            normalized.extend(&w[27 + (ac + bc) * 4..]);
            w = normalized;
            (vertices[..ac].to_vec(), vertices[ac..].to_vec())
        };
        check_solver(case, &w, &query, &target, &expected[2..]);
    }
}

fn check_solver(case: usize, w: &[u32], query: &[[f32; 4]], target: &[[f32; 4]], expected: &[u32]) {
    let v = |offset| std::array::from_fn(|i| f32::from_bits(w[offset + i]));
    let mut support = PenetrationSupport {
        query: &query,
        target: &target,
        relative: std::array::from_fn(|i| v(2 + i * 4)),
    };
    let mut seed = PenetrationSeed {
        count: w[26] as usize,
        points: std::array::from_fn(|i| ConvexSupportPoint {
            query: v(31 + i * 4),
            target: v(47 + i * 4),
            difference: v(63 + i * 4),
        }),
    };
    let mut retries = 0;
    let result = solve_penetration(&mut seed, &mut support, &mut retries, v(22), v(18)).unwrap();
    let mut actual = vec![result.status];
    actual.extend(support.relative.into_iter().flatten().map(f32::to_bits));
    actual.extend([retries, seed.count as u32]);
    for field in 0..3 {
        for p in seed.points {
            actual.extend(
                match field {
                    0 => p.query,
                    1 => p.target,
                    _ => p.difference,
                }
                .map(f32::to_bits),
            );
        }
    }
    if let Some(solution) = result.solution {
        actual.extend(
            solution
                .distance
                .into_iter()
                .chain(solution.normal)
                .chain(solution.witness)
                .map(f32::to_bits),
        );
    } else {
        actual.extend([0x12345678; 12]);
    }
    assert_eq!(actual.len(), expected.len());
    for (lane, (&got, &want)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            got, want,
            "case {case} lane {lane}: {got:08x} != {want:08x}"
        );
    }
}

#[test]
fn penetration_solver_matches_rotated_convex_pairs() {
    for (case, line) in include_str!("../../../tests/fixtures/penetration_solver_shapes.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let ac = w[0] as usize;
        let bc = w[1] as usize;
        let vertices: Vec<[f32; 4]> = w[27..27 + (ac + bc) * 4]
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|i| f32::from_bits(p[i])))
            .collect();
        let start = 27 + (ac + bc) * 4;
        let mut normalized = w[..27].to_vec();
        normalized.extend(&w[27..31]);
        normalized.extend(&w[start..start + 48]);
        check_solver(
            case,
            &normalized,
            &vertices[..ac],
            &vertices[ac..],
            &w[start + 48..],
        );
    }
}
