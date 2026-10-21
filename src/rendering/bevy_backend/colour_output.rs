//! Final shader colour transform and HDR storage conversion.
pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_colour_output.wgsl");

#[derive(Clone, Copy, Debug)]
pub struct ColourOutputParameters {
    pub tint_exposure: [f32; 4],
    pub white_level: [f32; 4],
    pub options: [u32; 4],
}

#[cfg(test)]
#[path = "colour_output/tests.rs"]
mod tests;
impl ColourOutputParameters {
    pub fn uniform_bytes(&self) -> [u8; 48] {
        let mut bytes = [0; 48];
        for (i, value) in self
            .tint_exposure
            .into_iter()
            .chain(self.white_level)
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (i, value) in self.options.into_iter().enumerate() {
            bytes[32 + i * 4..36 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
