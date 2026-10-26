use super::*;

#[test]
fn constraint_neighbors_match_executed_adjacency_and_body_admission() {
    let mut cases = 0;
    let mut pairs = 0;
    for row in include_str!("../../../tests/fixtures/constraint_neighbor_activation.txt").lines() {
        let w: Vec<_> = row
            .split_whitespace()
            .map(|v| u32::from_str_radix(v, 16).unwrap())
            .collect();
        let mut bodies = BodyStorage::new(8).unwrap();
        for i in 0..8 {
            let id = if i == 0 {
                0
            } else {
                bodies.reserve(&mut []).unwrap()
            };
            let body = bodies.get_mut(id).unwrap();
            body.id = w[3 + i * 3];
            body.flags = w[4 + i * 3];
            body.broadphase_id = w[5 + i * 3];
        }
        let constraints: Vec<_> = w[35..67]
            .chunks_exact(4)
            .map(|v| ConstraintLink {
                bodies: [v[0], v[1]],
                flags: v[2] as u8,
                link_index: v[3] as usize,
            })
            .collect();
        let next: Vec<_> = w[67..83].chunks_exact(2).map(|v| [v[0], v[1]]).collect();
        let links = ConstraintLinks {
            heads: &w[27..27 + w[2] as usize],
            constraints: &constraints,
            next: &next,
        };
        let mut actual = Vec::new();
        links
            .activate_neighbors(w[0] != 0, &bodies, w[1], |first, second| {
                actual.extend([first, second]);
                Ok(())
            })
            .unwrap();
        assert_eq!(actual.len(), w[83] as usize * 2, "case {cases}");
        assert_eq!(actual, w[84..], "case {cases}");
        pairs += w[83];
        cases += 1;
    }
    assert_eq!(cases, 512);
    assert!(pairs > 0);
}
