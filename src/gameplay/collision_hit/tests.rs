use super::*;
#[test]
fn closest_point_hits_match_executed_publication() {
    for (case, line) in include_str!("../../../tests/fixtures/closest_point_hit.txt")
        .lines()
        .enumerate()
    {
        let w: Vec<u32> = line
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let shape = |offset| ShapeHitInfo {
            body_id: (w[offset] != 0).then_some(w[offset + 1]),
            shape_key: w[offset + 2],
            material_id: w[offset + 3] as u16,
            filter_info: w[offset + 4],
            user_data: u64::from(w[offset + 5]) | (u64::from(w[offset + 6]) << 32),
        };
        let v = |offset| std::array::from_fn(|i| f32::from_bits(w[offset + i]));
        let accepted = w[49] != 0;
        let geometry = accepted.then(|| ConvexClosestPoint {
            point: v(33),
            normal: v(37),
            distance: v(41),
        });
        let mut recorded = None;
        let status = publish_closest_point(
            geometry,
            std::array::from_fn(|i| v(17 + i * 4)),
            shape(2),
            shape(9),
            w[16],
            |hit| recorded = Some(hit),
        );
        assert_eq!(status, accepted, "case {case} admission");
        assert_eq!(recorded.is_some(), accepted, "case {case} callback");
        if let Some(hit) = recorded {
            let mut actual: Vec<u32> = hit
                .position
                .into_iter()
                .chain(hit.normal)
                .chain([hit.distance])
                .map(f32::to_bits)
                .collect();
            for body in [hit.query, hit.target] {
                actual.extend([
                    body.body_id,
                    body.material_id as u32,
                    body.shape_key,
                    body.filter_info,
                    body.flags,
                    body.user_data as u32,
                    (body.user_data >> 32) as u32,
                ]);
            }
            actual.extend([hit.query_type as u32, (hit.query_type >> 32) as u32]);
            assert_eq!(actual, &w[50..], "case {case} defined fields");
        }
    }
}
