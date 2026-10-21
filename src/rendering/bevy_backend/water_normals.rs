//! arkWater's Perlin shading detail, shared by horizontal and oriented surfaces.
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_ocean_noise.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_normals.wgsl"),
);

#[derive(Clone, Copy, Debug)]
pub struct WaterNormalParameters {
    pub clock_octaves: [f32; 4],
    pub waves: [f32; 4],
}

#[cfg(test)]
#[path = "water_normals/tests.rs"]
mod tests;
impl WaterNormalParameters {
    pub fn from_settings(
        settings: &crate::rendering::water::WaterWaveSettings,
        local_time: f32,
    ) -> Self {
        Self {
            clock_octaves: [
                local_time,
                settings.maximum_octaves,
                settings.anti_alias,
                settings.frequency,
            ],
            waves: [settings.speed, settings.persistence, settings.amplitude, 0.],
        }
    }
    pub fn uniform_bytes(&self) -> [u8; 32] {
        let mut bytes = [0; 32];
        for (i, value) in self.clock_octaves.into_iter().chain(self.waves).enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
