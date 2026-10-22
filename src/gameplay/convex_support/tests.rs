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
