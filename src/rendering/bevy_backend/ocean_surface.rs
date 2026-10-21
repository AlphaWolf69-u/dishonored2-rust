//! Shared surface shader and its environment/view uniform layout.
//! FFT textures and current/previous selection belong to the actual wave owner.
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_ocean_noise.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_surface.wgsl"),
);

/// Seven aligned float4s matching OceanSurfaceParameters in the shared shader.
/// Supply current resolved environment/view values; no fallback defaults.
#[derive(Clone, Copy, Debug)]
pub struct OceanSurfaceParameters {
    /// ScaleX, ScaleY, ScaleZ, MasksExtent.
    pub spatial: [f32; 4],
    /// localTime.x, TimeScale, globalViewInfos.z, globalViewInfos.w.
    pub clock_view: [f32; 4],
    /// wsCamPos.xyz, NormalFadeOutStart.
    pub camera: [f32; 4],
    /// PerlinBlendStart, PerlinFrequency, PerlinSpeed, PerlinAmplitude.
    pub perlin: [f32; 4],
    /// PerlinPersistence, PerlinFoamThreshold, PerlinFoamTransition, FoamLuminance.
    pub perlin_foam: [f32; 4],
    /// ShoreWavesFrequency, ShoreWavesSpeed, ShoreWavesAmplitude, ShoreWavesFoamLuminance.
    pub shore: [f32; 4],
    /// ShoreWavesFoamFadeDistance/Length, FoamBubblesLuminance, NormalFadeOut.
    pub foam: [f32; 4],
}

impl OceanSurfaceParameters {
    /// Current game-space camera/view/clock data, plus the effective authored
    /// environment. Native previous-wave sampling also uses this current clock.
    pub fn from_settings(
        settings: &crate::rendering::ocean::OceanSurfaceSettings,
        local_time: f32,
        camera: [f32; 3],
        view_range: [f32; 2],
    ) -> Self {
        Self {
            spatial: [
                settings.scale[0],
                settings.scale[1],
                settings.scale[2],
                settings.masks_extent,
            ],
            clock_view: [
                local_time,
                settings.time_scale,
                view_range[0],
                view_range[1],
            ],
            camera: [camera[0], camera[1], camera[2], settings.normal_fade_start],
            perlin: [
                settings.perlin.blend_start,
                settings.perlin.frequency,
                settings.perlin.speed,
                settings.perlin.amplitude,
            ],
            perlin_foam: [
                settings.perlin.persistence,
                settings.perlin.foam_threshold,
                settings.perlin.foam_transition,
                settings.foam_luminance,
            ],
            shore: [
                settings.shore.frequency,
                settings.shore.speed,
                settings.shore.amplitude,
                settings.shore.foam_luminance,
            ],
            foam: [
                settings.shore.foam_fade_distance,
                settings.shore.foam_fade_length,
                settings.foam_bubbles_luminance,
                settings.normal_fade_factor,
            ],
        }
    }

    pub fn uniform_bytes(&self) -> [u8; 112] {
        let mut bytes = [0; 112];
        let values = [
            self.spatial,
            self.clock_view,
            self.camera,
            self.perlin,
            self.perlin_foam,
            self.shore,
            self.foam,
        ];
        for (offset, value) in values.into_iter().flatten().enumerate() {
            bytes[offset * 4..offset * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

#[cfg(all(test, feature = "game"))]
#[path = "ocean_surface/tests.rs"]
mod tests;
