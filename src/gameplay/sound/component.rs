//! arkCpntAudio time scaling (140708960 / 140708250 / 140708b00).
use crate::assets::entities::Value;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub dilation_factor: f32,
}
impl Default for Settings {
    fn default() -> Self { Self { dilation_factor: 1.0 } } // 14000ce50
}
impl Settings {
    /// Reflected at offset 0x6c by 1415536e0.
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut settings = Self::default();
        if let Some(value) = crate::entities::scene_motion::atom(fields, "m_dilationFactor") {
            settings.dilation_factor = value.parse::<f32>().map_err(|error| format!("audio dilation factor: {error}"))?;
            if !settings.dilation_factor.is_finite() { return Err("non-finite audio dilation factor".into()); }
        }
        Ok(settings)
    }

    pub fn rate(self, clock_rate: f32) -> f32 { clock_rate * self.dilation_factor }
}
