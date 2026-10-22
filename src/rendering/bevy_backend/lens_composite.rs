//! Authored pre-tone bloom/lens-dirt/flare composition, fused into the display
//! pass when its frame owner supplies all real images and parameters.
use bevy::render::render_resource::TextureView;
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_colour_output.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_lens_streak.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_lens_composite.wgsl")
);

#[derive(Clone, Copy, Debug)]
pub struct LensCompositeParameters {
    /// Resolution scale and subpixel offset.
    pub viewport: [f32; 4],
    /// Inverse target size, source view-axis sum, host radiance multiplier.
    pub pixel: [f32; 4],
    /// Veil strength, lens-dirt strength, blend fraction, white level.
    pub controls: [f32; 4],
    /// Streak radius, random offset, width, opacity.
    pub streak: [f32; 4],
    /// Black-and-white tint; W is unused.
    pub tint: [f32; 4],
    /// Bloom enabled, flares enabled, default dirt, black-and-white program.
    pub flags: [u32; 4],
    /// HDR mode (0: uncompressed, 1: compressed, 3: compressed with host source bridge),
    /// BW runtime, BW exclusion, embedded-cube debug mode.
    pub options: [u32; 4],
}
impl LensCompositeParameters {
    pub fn uniform_bytes(&self) -> [u8; 112] {
        let mut bytes = [0; 112];
        for (i, word) in self
            .viewport
            .into_iter()
            .chain(self.pixel)
            .chain(self.controls)
            .chain(self.streak)
            .chain(self.tint)
            .map(f32::to_bits)
            .chain(self.flags)
            .chain(self.options)
            .enumerate()
        {
            bytes[4 * i..4 * i + 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }
}
pub struct LensCompositeInputs<'a> {
    pub parameters: &'a LensCompositeParameters,
    pub bloom: &'a TextureView,
    pub dirt_from: &'a TextureView,
    pub dirt_to: &'a TextureView,
    pub flares: &'a TextureView,
}
#[cfg(test)]
#[path = "lens_composite/tests.rs"]
mod tests;
