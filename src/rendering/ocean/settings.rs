//! Installed environment inputs for3014b0, arkOceanHelpers and FFT foam.
use super::OceanViewInput;
use crate::assets::{
    archive::Catalog,
    entities::EnvironmentRenderParameters,
    environment::{ParameterReader, ParameterSource},
};

#[derive(Clone, Copy, Debug)]
pub struct PerlinWaveSettings {
    pub blend_start: f32,
    pub blend_end: f32,
    pub frequency: f32,
    pub speed: f32,
    pub amplitude: f32,
    pub persistence: f32,
    pub foam_threshold: f32,
    pub foam_transition: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct ShoreWaveSettings {
    pub frequency: f32,
    pub speed: f32,
    pub amplitude: f32,
    pub foam_luminance: f32,
    pub foam_fade_distance: f32,
    pub foam_fade_length: f32,
}
#[derive(Clone, Copy, Debug)]
pub struct WaveFoamParameters {
    pub threshold: f32,
    pub spawn_rate: f32,
    pub decay_rate: f32,
}
#[derive(Clone, Debug)]
pub struct ProjectedShadowSettings {
    pub image: String,
    pub extent: f32,
    pub start_height: f32,
    pub end_height: f32,
}
#[derive(Clone, Debug)]
pub struct OceanSurfaceSettings {
    pub scale: [f32; 3],
    pub masks_extent: f32,
    pub time_scale: f32,
    pub normal_fade_start: f32,
    pub normal_fade_factor: f32,
    pub perlin: PerlinWaveSettings,
    pub shore: ShoreWaveSettings,
    pub foam_luminance: f32,
    pub foam_bubbles_luminance: f32,
    pub shore_distance_image: String,
    pub wave_amplitude_image: String,
    pub foam_image: String,
}
#[derive(Clone, Debug)]
pub struct OceanSettings {
    pub water: crate::rendering::water::WaterSettings,
    pub view: OceanViewInput,
    pub surface: OceanSurfaceSettings,
    pub foam: WaveFoamParameters,
    pub projected_shadow: ProjectedShadowSettings,
}
impl OceanSettings {
    /// Supply the effective environment, including actual area/water overrides.
    /// Resolve once during preparation; no archive IO belongs in pixel queries.
    pub fn load(
        catalog: &Catalog,
        parameters: &EnvironmentRenderParameters,
    ) -> Result<Self, String> {
        Self::resolve(&ParameterReader {
            catalog,
            parameters,
        })
    }
    pub fn resolve(read: &impl ParameterSource) -> Result<Self, String> {
        Ok(Self {
            water: crate::rendering::water::WaterSettings::resolve(read)?,
            view: OceanViewInput {
                enabled: read.boolean("ocean/enable")?,
                enable_ssr: read.boolean("water/enablessreflection")?,
                disable_ssr_on_rough_water: read
                    .boolean("water/disablessreflectiononroughwater")?,
                use_sky_cube_map: read.boolean("water/useskycubemap")?,
                water_volume: read.boolean("water/iswatervolume")?,
                surface_water_level: read.scalar("watersurface/waterlevel")?,
                ocean_water_level: read.scalar("ocean/waterlevel")?,
                max_wave_amplitude: read.scalar("water/maxwaveamplitude")?,
                wind: [
                    read.scalar("ocean/fftwinddirectionx")?,
                    read.scalar("ocean/fftwinddirectiony")?,
                ],
                foam_type: read.scalar("fftwaves/foamtype")?,
                wind_speed: read.scalar("ocean/fftwindspeed")?,
                wind_dependency: read.scalar("ocean/fftwinddependency")?,
                small_waves: read.scalar("ocean/fftsmallwaves")?,
                choppiness: read.scalar("ocean/fftchoppiness")?,
            },
            surface: OceanSurfaceSettings {
                scale: [
                    read.scalar("ocean/scalex")?,
                    read.scalar("ocean/scaley")?,
                    read.scalar("ocean/scalez")?,
                ],
                masks_extent: read.scalar("ocean/masksextent")?,
                time_scale: read.scalar("ocean/timescale")?,
                normal_fade_start: read.scalar("ocean/normalfadeoutstart")?,
                normal_fade_factor: read.scalar("ocean/normalfadeout")?,
                perlin: PerlinWaveSettings {
                    blend_start: read.scalar("ocean/perlinblendstart")?,
                    blend_end: read.scalar("ocean/perlinblendend")?,
                    frequency: read.scalar("ocean/perlinfrequency")?,
                    speed: read.scalar("ocean/perlinspeed")?,
                    amplitude: read.scalar("ocean/perlinamplitude")?,
                    persistence: read.scalar("ocean/perlinpersistence")?,
                    foam_threshold: read.scalar("ocean/perlinfoamthreshold")?,
                    foam_transition: read.scalar("ocean/perlinfoamtransition")?,
                },
                shore: ShoreWaveSettings {
                    frequency: read.scalar("ocean/shorewavesfrequency")?,
                    speed: read.scalar("ocean/shorewavesspeed")?,
                    amplitude: read.scalar("ocean/shorewavesamplitude")?,
                    foam_luminance: read.scalar("ocean/shorewavesfoamluminance")?,
                    foam_fade_distance: read.scalar("ocean/shorewavesfoamfadedistance")?,
                    foam_fade_length: read.scalar("ocean/shorewavesfoamfadelength")?,
                },
                foam_luminance: read.scalar("ocean/foamluminance")?,
                foam_bubbles_luminance: read.scalar("ocean/foambubblesluminance")?,
                shore_distance_image: read.image_source("ocean/shoredistancefieldtex")?,
                wave_amplitude_image: read.image_source("ocean/waveamplitudemask")?,
                foam_image: read.image_source("ocean/foamtex")?,
            },
            foam: WaveFoamParameters {
                threshold: read.scalar("fftwaves/foamthreshold")?,
                spawn_rate: read.scalar("fftwaves/foamspawnrate")?,
                decay_rate: read.scalar("fftwaves/foamdecayrate")?,
            },
            projected_shadow: ProjectedShadowSettings {
                image: read.image_source("ocean/projectedshadowheightmap")?,
                extent: read.scalar("ocean/projectedshadowheightmapextent")?,
                start_height: read.scalar("ocean/projectedshadowstartheight")?,
                end_height: read.scalar("ocean/projectedshadowendheight")?,
            },
        })
    }
}

#[cfg(test)]
mod tests;
