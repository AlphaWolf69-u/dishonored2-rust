//! Standard final colour transform; the view owner supplies linear HDR,
//! actual exposure and the prepared colour cube. No fog filter is added.
pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_tone_mapping.wgsl");

#[path = "tone_mapping/pass.rs"]
mod pass;
pub use pass::ToneMappingPass;

#[derive(Clone, Copy, Debug)]
pub struct ToneMappingParameters {
    pub curve: crate::rendering::tone_mapping::ToneCurve,
    pub gamma: f32,
    pub brightness: f32,
    pub exposure: f32,
    pub use_curve: bool,
    pub use_colour_cube: bool,
}
impl ToneMappingParameters {
    pub fn uniform_bytes(&self) -> [u8; 80] {
        let mut bytes = [0; 80];
        for (i, value) in self
            .curve
            .junctions
            .into_iter()
            .chain(self.curve.toe)
            .chain(self.curve.shoulder)
            .chain([self.gamma, self.brightness, self.exposure, 0.])
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (i, value) in [
            u32::from(self.use_curve),
            u32::from(self.use_colour_cube),
            0,
            0,
        ]
        .into_iter()
        .enumerate()
        {
            bytes[64 + i * 4..68 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
#[path = "tone_mapping/tests.rs"]
mod tests;
