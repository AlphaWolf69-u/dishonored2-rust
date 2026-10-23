//! HBAO+ 3 view constants used by 305620, verified against the installed SDK.
//! Shader stages and GPU ownership live in bevy_backend::ambient_occlusion.
#[derive(Clone, Debug)]
pub enum AmbientOcclusionParameters {
    Disabled,
    Hbao(HbaoParameters),
    Dwao {
        extent: [u32; 2],
        inverse_projection: [[f32; 4]; 4],
        render_scale: [f32; 2],
    },
}

impl AmbientOcclusionParameters {
    pub fn mode(&self) -> u8 {
        match self {
            Self::Disabled => 0,
            Self::Hbao(_) => 1,
            Self::Dwao { .. } => 2,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HbaoParameters {
    /// Original GlobalConstantBuffer prefix. The unused normal-matrix tail is
    /// zero because the game reconstructs normals from depth.
    pub words: [u32; 56],
    pub extent: [u32; 2],
    pub quarter_extent: [u32; 2],
}
impl HbaoParameters {
    /// Projection uses the source/D3D row-vector packing (also glam's column
    /// array). Render scale is the current full-view scale, not its pixel size.
    pub fn new(
        extent: [u32; 2],
        projection: [f32; 16],
        render_scale: [f32; 2],
    ) -> Result<Self, String> {
        if extent.contains(&0)
            || projection.iter().any(|v| !v.is_finite())
            || projection[0] == 0.0
            || projection[5] == 0.0
            || projection[14] == 0.0
            || projection[11].abs() != 1.0
            || render_scale
                .iter()
                .any(|s| !s.is_finite() || *s <= 0.0 || *s > 1.0)
        {
            return Err("invalid HBAO view inputs".into());
        }
        let quarter_extent = extent.map(|v| v.div_ceil(4));
        let mut values = [0.0f32; 56];
        values[4..8].copy_from_slice(&[
            1.0 / quarter_extent[0] as f32,
            1.0 / quarter_extent[1] as f32,
            1.0 / extent[0] as f32,
            1.0 / extent[1] as f32,
        ]);
        let tx = 1.0 / projection[0].abs();
        let ty = 1.0 / projection[5].abs();
        values[8..12].copy_from_slice(&[2.0 * tx, -2.0 * ty, -tx, ty]);
        let radius = 0.7f32;
        let bias = 0.1f32;
        let radius_pixels = radius * 0.5 / ty * extent[1] as f32;
        let r2 = radius * radius;
        values[12..16].copy_from_slice(&[radius_pixels, r2, -1.0 / r2, bias]);
        // The 3.0 SDK admits NearAO through 4 (the later SDK's SmallScaleAO
        // limit of 2 is not equivalent). 305620 compensates current render scale.
        let scale = render_scale[0].min(render_scale[1]);
        let amount = 1.0 / (1.0 - bias);
        values[16..20].copy_from_slice(&[
            (1.0 / scale).min(4.0) * amount * 2.0,
            scale * 1.75 * amount,
            1.75,
            0.0,
        ]);
        values[20..24].copy_from_slice(&[0.0, 1.0, 8.0, 8.0]);
        // Preserve the SDK's division order. Collapsing these to 1/P32 loses
        // a float step in the reverse-Z projection used by the game.
        let a = if projection[11] == 1.0 {
            -projection[10]
        } else {
            projection[10]
        };
        let near = projection[14] / a;
        let far = projection[14] / (a + 1.0);
        let inverse_near = 1.0 / near;
        let inverse_far = 1.0 / far;
        values[24..28].copy_from_slice(&[inverse_far - inverse_near, inverse_near, 1.0, 0.0]);
        values[28..32].copy_from_slice(&[0.0, 0.0, 0.0, 1.0]);
        values[32..36].copy_from_slice(&[radius_pixels / 1e-6, radius_pixels / 4.0, 0.0, 0.0]);
        let mut words = values.map(f32::to_bits);
        words[0] = 3;
        words[34] = 3;
        Ok(Self {
            words,
            extent,
            quarter_extent,
        })
    }
    pub fn uniform_bytes(&self) -> [u8; 224] {
        let mut bytes = [0; 224];
        for (chunk, word) in bytes.chunks_exact_mut(4).zip(self.words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }
}

/// Fixed HBAO+ jitter sequence (RandomTexture.cpp). Source-derived constants
/// and translations retain NVIDIA's notice; see vendor/hbao/LICENSE.txt.
/// The game uses the first sixteen Mersenne-Twister triples, without a new
/// random sequence each frame.
pub fn hbao_jitters() -> [[f32; 4]; 16] {
    const RANDOM: [f32; 48] = [
        0.463937, 0.340042, 0.223035, 0.468465, 0.322224, 0.979269, 0.031798, 0.973392, 0.778313,
        0.456168, 0.258593, 0.330083, 0.387332, 0.380117, 0.179842, 0.910755, 0.511623, 0.092933,
        0.180794, 0.620153, 0.101348, 0.556342, 0.642479, 0.442008, 0.215115, 0.475218, 0.157357,
        0.568868, 0.501241, 0.629229, 0.699218, 0.707733, 0.556725, 0.005520, 0.708315, 0.583199,
        0.236644, 0.992380, 0.981091, 0.119804, 0.510866, 0.560499, 0.961497, 0.557862, 0.539955,
        0.332871, 0.417807, 0.920779,
    ];
    std::array::from_fn(|i| {
        let angle = (2.0 * std::f32::consts::PI) * RANDOM[i * 3] / 8.0;
        [
            angle.cos(),
            angle.sin(),
            RANDOM[i * 3 + 1],
            RANDOM[i * 3 + 2],
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jitters_match_installed_sdk() {
        for (i, row) in include_str!("../../tests/fixtures/ambient_occlusion_jitters.txt")
            .lines()
            .enumerate()
        {
            let expected: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let jitter = hbao_jitters()[i];
            assert_eq!(
                jitter.map(f32::to_bits).as_slice(),
                &expected[..4],
                "slice {i}"
            );
            assert_eq!(expected[4], (i as f32 % 4.0 + 0.5).to_bits());
            assert_eq!(expected[5], ((i / 4) as f32 + 0.5).to_bits());
            assert_eq!(expected[6], (i as f32).to_bits());
            assert_eq!(expected[7], i as u32);
        }
    }
    #[test]
    fn view_constants_match_installed_hbao_sdk() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/ambient_occlusion_parameters.txt").lines() {
            let w: Vec<_> = row
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect();
            let f = |i| f32::from_bits(w[i]);
            let mut projection = [0.0; 16];
            projection[0] = f(3);
            projection[5] = f(4);
            projection[10] = f(5);
            projection[11] = -1.0;
            projection[14] = f(6);
            let actual = HbaoParameters::new([w[0], w[1]], projection, [f(2); 2]).unwrap();
            assert_eq!(&actual.words, w[7..].as_ref(), "case {count}");
            count += 1;
        }
        assert_eq!(count, 32);
    }
}
