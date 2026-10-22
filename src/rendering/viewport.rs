//! 310670's viewport-derived shader constants, called by 2bc610.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportParameters {
    pub position_to_view_texture: [f32; 4],
    pub viewport: [f32; 4],
}

pub fn parameters(
    origin: [i32; 2],
    coordinate_extent: [i32; 2],
    viewport_extent: [i32; 2],
) -> ViewportParameters {
    let [x, y] = origin.map(|v| v as f32);
    let [width, height] = coordinate_extent.map(|v| v as f32);
    ViewportParameters {
        position_to_view_texture: [x / width, y / height, 1.0 / width, 1.0 / height],
        viewport: [x, y, viewport_extent[0] as f32, viewport_extent[1] as f32],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_constants_match_installed_writer() {
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/viewport_parameters.txt").lines() {
            let words: Vec<_> = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let p = parameters(
                [words[0] as i32, words[1] as i32],
                [words[2] as i32, words[3] as i32],
                [words[4] as i32, words[5] as i32],
            );
            let result: Vec<_> = p
                .position_to_view_texture
                .into_iter()
                .chain(p.viewport)
                .map(f32::to_bits)
                .collect();
            assert_eq!(result, &words[6..], "{line}");
            count += 1;
        }
        assert_eq!(count, 128);
    }
}
