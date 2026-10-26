use super::super::distance::query_point;
use super::*;

#[test]
fn point_distance_and_publication_match_rotated_targets() {
    for (case, line) in include_str!("../../../../tests/fixtures/point_distance_shapes.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let bc = w[0] as usize;
        let v = |offset| std::array::from_fn(|i| f32::from_bits(w[offset + i]));
        let point = v(1);
        let relative = std::array::from_fn(|i| v(5 + i * 4));
        let target: Vec<[f32; 4]> = w[24..24 + bc * 4]
            .chunks_exact(4)
            .map(|p| std::array::from_fn(|i| f32::from_bits(p[i])))
            .collect();
        let start = 24 + bc * 4;
        let sentinel = [f32::from_bits(0x12345678); 4];
        let mut result = ConvexDistanceSolution {
            status: 0,
            distance: sentinel,
            normal: sentinel,
            witness: sentinel,
        };
        solve_point_distance(point, &target, relative, &mut result).unwrap();
        let actual: Vec<u32> = std::iter::once(result.status as u32)
            .chain(
                result
                    .distance
                    .into_iter()
                    .chain(result.normal)
                    .chain(result.witness)
                    .map(f32::to_bits),
            )
            .collect();
        assert_eq!(actual, &w[start..start + 13], "case {case} solve");
        let result = query_point(
            point,
            &target,
            relative,
            f32::from_bits(w[21]),
            f32::from_bits(w[22]),
            f32::from_bits(w[23]),
        )
        .unwrap();
        assert_eq!(result.is_some(), w[start + 13] != 0, "case {case} accepted");
        if let Some(result) = result {
            let actual: Vec<u32> = result
                .point
                .into_iter()
                .chain(result.normal)
                .chain(result.distance)
                .map(f32::to_bits)
                .collect();
            assert_eq!(actual, &w[start + 14..], "case {case} publication");
        }
    }
}
#[test]
fn point_distance_matches_complete_executed_solver() {
    let relative = std::array::from_fn(|i| std::array::from_fn(|j| if i == j { 1. } else { 0. }));
    let mut failures = 0;
    for (case, line) in include_str!("../../../../tests/fixtures/point_distance.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let point = [
            f32::from_bits(w[1]),
            f32::from_bits(w[2]),
            f32::from_bits(w[3]),
            0.5,
        ];
        let target: Vec<[f32; 4]> = if w[0] == 0 {
            (0..8)
                .map(|i| {
                    [
                        (((i >> 0) & 1) * 2) as f32 - 1.,
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
        let sentinel = [f32::from_bits(0x12345678); 4];
        let mut result = ConvexDistanceSolution {
            status: 0,
            distance: sentinel,
            normal: sentinel,
            witness: sentinel,
        };
        solve_point_distance(point, &target, relative, &mut result).unwrap();
        let actual: Vec<u32> = std::iter::once(result.status as u32)
            .chain(
                result
                    .distance
                    .into_iter()
                    .chain(result.normal)
                    .chain(result.witness)
                    .map(f32::to_bits),
            )
            .collect();
        if actual != w[4..17] {
            failures += 1;
            if failures <= 8 {
                eprintln!(
                    "case {case} actual {:08x?} expected {:08x?}",
                    actual,
                    &w[4..17]
                );
            }
        }
    }
    assert_eq!(failures, 0);
}
