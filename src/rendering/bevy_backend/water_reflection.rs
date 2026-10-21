//! Water reflection composition over explicit current-view depth, HDR and fog.
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_environment.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_screen_ray.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_reflection.wgsl"),
);

#[derive(Clone, Copy, Debug)]
pub struct WaterReflectionParameters {
    /// Logical column-major transforms, without shadow transport transposition.
    pub view_from_world: [f32; 16],
    pub camera_to_clip: [f32; 16],
    /// -inverseProjection[2].w, inverseProjection[3][2], [3][3], unused.
    pub unproject: [f32; 4],
    /// Near multiplier/distance, far multiplier/distance.
    pub thickness: [f32; 4],
    /// Thickness exponent, vertical fade, border fade, normal softening.
    pub trace: [f32; 4],
    /// Resolution scale.xy, exposure EngineLuminanceFactor, white level.
    pub screen_hdr: [f32; 4],
    /// Material SSR option, runtime SSR switch, max steps, compressed HDR.
    pub options: [u32; 4],
}

impl WaterReflectionParameters {
    pub fn update_settings(&mut self, settings: &crate::rendering::water::WaterReflectionSettings) {
        self.thickness = settings.thickness;
        self.trace = [
            settings.thickness_power,
            settings.vertical_fade,
            settings.border_fade,
            settings.normal_softening,
        ];
        self.options[2] = settings.maximum_step_count();
    }

    pub fn uniform_bytes(&self) -> [u8; 208] {
        let mut bytes = [0; 208];
        for (i, value) in self
            .view_from_world
            .into_iter()
            .chain(self.camera_to_clip)
            .chain(self.unproject)
            .chain(self.thickness)
            .chain(self.trace)
            .chain(self.screen_hdr)
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (i, value) in self.options.into_iter().enumerate() {
            bytes[192 + i * 4..196 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[cfg(test)]
#[path = "water_reflection/tests.rs"]
mod tests;
