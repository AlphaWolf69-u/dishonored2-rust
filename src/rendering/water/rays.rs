//! Authored admission and intensity for water/ComputeGodRays.
use crate::assets::environment::ParameterSource;

pub const FLOAT_PARAMETERS: [&str; 1] = ["water/godraysintensity"];
pub const BOOLEAN_PARAMETERS: [&str; 2] = ["water/enablegodrays", "water/enablegodraysshadowing"];

#[derive(Clone, Copy)]
pub struct WaterRaySettings {
    pub enabled: bool,
    pub shadowing: bool,
    pub intensity: f32,
}
impl WaterRaySettings {
    pub fn resolve(read: &impl ParameterSource) -> Result<Self, String> {
        Ok(Self {
            enabled: read.boolean("water/enablegodrays")?,
            shadowing: read.boolean("water/enablegodraysshadowing")?,
            intensity: read.scalar("water/godraysintensity")?,
        })
    }
}
