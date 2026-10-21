//! Complete default surface composition; resources and view state are explicit.
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_texture_array_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_lighting.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_environment.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_screen_ray.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_refraction.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_reflection.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_surface.wgsl"),
);

#[derive(Clone, Copy, Debug)]
pub struct WaterSurfaceParameters {
    pub underwater_visibility_boost: f32,
    pub shore_foam_luminance: f32,
    pub use_sun: bool,
    pub disable_ssr_on_rough_water: bool,
}

impl WaterSurfaceParameters {
    pub fn uniform_bytes(&self) -> [u8; 32] {
        let mut bytes = [0; 32];
        bytes[0..4].copy_from_slice(&self.underwater_visibility_boost.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.shore_foam_luminance.to_le_bytes());
        bytes[16..20].copy_from_slice(&u32::from(self.use_sun).to_le_bytes());
        bytes[20..24].copy_from_slice(&u32::from(self.disable_ssr_on_rough_water).to_le_bytes());
        bytes
    }
}

#[cfg(test)]
#[path = "water_surface/tests.rs"]
mod tests;
