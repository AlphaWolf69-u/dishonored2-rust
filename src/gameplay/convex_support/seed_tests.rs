use super::{penetration::*, seed::*};

#[test]
fn penetration_seed_matches_executed_initialization() {
    for (case, line) in include_str!("../../../tests/fixtures/penetration_seed.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let v = |offset| std::array::from_fn(|i| f32::from_bits(w[offset + i]));
        let query = [v(27)];
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
        check_seed(case, &w, &query, &target);
    }
}

fn check_seed(case: usize, w: &[u32], query: &[[f32; 4]], target: &[[f32; 4]]) {
    let v = |offset| std::array::from_fn(|i| f32::from_bits(w[offset + i]));
    let support = PenetrationSupport {
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
    let outcome = seed
        .build(&support, f32::from_bits(w[22]), f32::from_bits(w[18]))
        .unwrap();
    let (done, status, solution) = match outcome {
        SeedOutcome::Ready => (0, 1, None),
        SeedOutcome::Failed => (1, 7, None),
        SeedOutcome::Contact(solution) => (1, solution.status as u32, Some(solution)),
    };
    let mut actual = vec![done, status, seed.count as u32];
    for field in 0..3 {
        for point in seed.points {
            actual.extend(
                match field {
                    0 => point.query,
                    1 => point.target,
                    _ => point.difference,
                }
                .map(f32::to_bits),
            );
        }
    }
    if let Some(solution) = solution {
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
    for (lane, (&got, &expected)) in actual.iter().zip(&w[79..]).enumerate() {
        assert_eq!(
            got, expected,
            "case {case} lane {lane}: {got:08x} != {expected:08x}"
        );
    }
}

#[test]
fn penetration_seed_matches_degenerate_and_retry_paths() {
    for (case, line) in include_str!("../../../tests/fixtures/penetration_seed_degenerate.txt")
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
        // Normalize the variable-size fixture header to the recorded-call layout.
        let mut normalized = w[..27].to_vec();
        normalized.extend(&w[27..31]);
        normalized.extend(&w[27 + (ac + bc) * 4..]);
        check_seed(case, &normalized, &vertices[..ac], &vertices[ac..]);
    }
}
