use super::*;
fn words(line: &str) -> Vec<u32> {
    line.split_whitespace()
        .map(|w| u32::from_str_radix(w, 16).unwrap())
        .collect()
}
#[test]
fn material_filter_matches_executed_edits() {
    for (case, line) in include_str!("../../../tests/fixtures/material_filter.txt")
        .lines()
        .enumerate()
    {
        let w = words(line);
        assert_eq!(material_filter(w[0], w[1] as u16), w[2], "case {case}");
    }
}
#[test]
fn shape_tags_match_executed_decoder() {
    struct Source {
        index: usize,
        material: u16,
        flags: u16,
        mapped: bool,
    }
    impl ShapeTagSource for Source {
        fn mapped_material(&self, index: usize) -> Result<u16, String> {
            assert!(self.mapped, "direct/invalid tag read the material map");
            assert_eq!(index, self.index);
            Ok(self.material)
        }
        fn material_flags(&self, material: u16) -> Result<u16, String> {
            assert!(self.mapped, "direct/invalid tag read material flags");
            assert_eq!(material, self.material);
            Ok(self.flags)
        }
    }
    for (case, line) in include_str!("../../../tests/fixtures/shape_tags.txt")
        .lines()
        .enumerate()
    {
        let w = words(line);
        let tag = w[0] as u16;
        let source = Source {
            index: w[2] as usize + usize::from(tag & 0x7fff),
            material: w[4] as u16,
            flags: w[5] as u16,
            mapped: tag < 0x8000,
        };
        let mut hit = ShapeHitInfo {
            body_id: Some(23),
            material_id: w[2] as u16,
            shape_key: 0x12345678,
            filter_info: w[3],
            user_data: 0x123456789abcdef0,
        };
        decode_shape_tag(tag, (w[1] != 0).then_some(&source), &mut hit).unwrap();
        assert_eq!(
            (hit.material_id, hit.filter_info),
            (w[6] as u16, w[7]),
            "case {case}"
        );
        assert_eq!(
            (hit.body_id, hit.shape_key, hit.user_data),
            (Some(23), 0x12345678, 0x123456789abcdef0)
        );
    }
}
