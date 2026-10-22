//! Colour-buffer downscaling selected by 34f840, separate from luminance and
//! gathered depth reduction. Discriminants are PostFX_Downscale's TYPE values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum DownscaleMode {
    AverageArgb = 0,
    MinimumArgb = 1,
    MaximumArgb = 2,
    MinimumRg = 3,
    AverageRgb = 4,
    AverageClear = 5,
    AverageRgbClamp = 6,
}
impl DownscaleMode {
    pub fn from_selector(selector: u8, paired_minimum: bool) -> Option<Self> {
        if paired_minimum {
            return Some(Self::MinimumRg);
        }
        Some(match selector {
            0 => Self::MinimumArgb,
            1 => Self::MaximumArgb,
            2 => Self::AverageArgb,
            3 => Self::AverageRgb,
            4 => Self::AverageClear,
            5 => Self::AverageRgbClamp,
            _ => return None,
        })
    }
}
#[derive(Clone, Copy, Debug)]
pub struct DownscaleInputs {
    pub texel_size: [f32; 4],
    pub colour: [f32; 4],
}
impl DownscaleInputs {
    /// DAT1422fd9f0 supplies (1,1), rather than a half-texel offset.
    pub fn new(source_extent: [u32; 2], colour: [f32; 3]) -> Result<Self, String> {
        if source_extent.contains(&0) {
            return Err("colour downscale requires a nonempty source".into());
        }
        Ok(Self {
            texel_size: [
                1.0 / source_extent[0] as f32,
                1.0 / source_extent[1] as f32,
                0.0,
                0.0,
            ],
            colour: [
                colour[0],
                colour[1],
                colour[2],
                colour[0] * colour[0] + colour[1] * colour[1] + colour[2] * colour[2],
            ],
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn downscale_parameters_match_executable() {
        let modes = [
            DownscaleMode::MinimumArgb,
            DownscaleMode::MaximumArgb,
            DownscaleMode::AverageArgb,
            DownscaleMode::AverageRgb,
            DownscaleMode::AverageClear,
            DownscaleMode::AverageRgbClamp,
            DownscaleMode::MinimumRg,
        ];
        let mut count = 0;
        for line in include_str!("../../tests/fixtures/colour_downscale_parameters.txt").lines() {
            let words: Vec<_> = line.split_whitespace().collect();
            let number = |i: usize| words[i].parse::<u32>().unwrap();
            let input = words[4];
            let bytes: Vec<_> = (0..16)
                .map(|i| u8::from_str_radix(&input[i * 2..i * 2 + 2], 16).unwrap())
                .collect();
            let floats: Vec<_> = bytes
                .chunks_exact(4)
                .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let actual =
                DownscaleInputs::new([number(0), number(1)], [floats[0], floats[1], floats[2]])
                    .unwrap();
            let mode = DownscaleMode::from_selector(number(2) as u8, number(3) != 0);
            assert_eq!(mode, number(5).checked_sub(1).map(|i| modes[i as usize]));
            let expected: Vec<_> = words[6..]
                .iter()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            assert_eq!(
                actual
                    .texel_size
                    .into_iter()
                    .chain(actual.colour)
                    .map(f32::to_bits)
                    .collect::<Vec<_>>(),
                expected
            );
            count += 1;
        }
        assert_eq!(count, 64);
    }
}
