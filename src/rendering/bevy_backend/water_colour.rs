//! Placed-water colour inputs over the shared complete water shading helpers.
use super::water_normals::WaterNormalParameters;

pub const INPUT_SOURCE: &str = include_str!("../../../assets/shaders/dh2_water_colour.wgsl");
pub fn shader_source() -> String {
    format!(
        "{}\n{}\n{}\n{}\n{INPUT_SOURCE}",
        super::water_normals::SHADER_SOURCE,
        super::water_surface::SHADER_SOURCE,
        super::probe_weights::SHADER_SOURCE,
        super::water_probes::INPUT_SOURCE
    )
}

#[cfg(test)]
#[path = "water_colour/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug)]
pub struct WaterColourParameters {
    pub normals: WaterNormalParameters,
    /// Logical column-major matrix in the game's world basis.
    pub view_from_world: [f32; 16],
    pub camera_refraction: [f32; 4],
    pub water: [f32; 4],
    pub light: [f32; 4],
    pub sun: [f32; 4],
    pub horizontal: bool,
}
impl WaterColourParameters {
    pub fn from_settings(
        settings: &crate::rendering::water::WaterSettings,
        local_time: f32,
        camera: [f32; 3],
        view_from_world: [f32; 16],
        parallel: ([f32; 3], [f32; 3]),
        horizontal: bool,
    ) -> Self {
        Self {
            normals: WaterNormalParameters::from_settings(&settings.waves, local_time),
            view_from_world,
            camera_refraction: [camera[0], camera[1], camera[2], settings.refraction],
            water: [
                settings.colour[0],
                settings.colour[1],
                settings.colour[2],
                settings.extinction,
            ],
            light: [
                parallel.0[0],
                parallel.0[1],
                parallel.0[2],
                settings.inscattering,
            ],
            sun: [
                parallel.1[0],
                parallel.1[1],
                parallel.1[2],
                settings.specular_power,
            ],
            horizontal,
        }
    }
    pub fn uniform_bytes(&self) -> [u8; 176] {
        let mut bytes = [0; 176];
        bytes[..32].copy_from_slice(&self.normals.uniform_bytes());
        for (i, value) in self
            .view_from_world
            .into_iter()
            .chain(self.camera_refraction)
            .chain(self.water)
            .chain(self.light)
            .chain(self.sun)
            .enumerate()
        {
            bytes[32 + i * 4..36 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[160..164].copy_from_slice(&u32::from(self.horizontal).to_le_bytes());
        bytes
    }
}
