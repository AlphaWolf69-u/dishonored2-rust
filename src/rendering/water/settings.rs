//! Authored optical inputs shared by arkWater and arkOcean colour programs.
use crate::assets::environment::ParameterSource;

#[derive(Clone, Debug)]
pub struct CausticSettings {
    pub image: String,
    pub speed: f32,
    pub tiling: f32,
    pub fade: f32,
    pub chromatic_aberration: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct WaterWaveSettings {
    pub maximum_octaves: f32,
    pub anti_alias: f32,
    pub frequency: f32,
    pub speed: f32,
    pub persistence: f32,
    pub amplitude: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct WaterReflectionSettings {
    /// Near multiplier/distance, far multiplier/distance, in shader order.
    pub thickness: [f32; 4],
    pub thickness_power: f32,
    pub vertical_fade: f32,
    pub border_fade: f32,
    pub normal_softening: f32,
    /// Remains a float until the selected shader's integer conversion.
    pub maximum_steps: f32,
}

impl WaterReflectionSettings {
    /// Cached ocean PS14070 converts this float with FTOU. Rust's saturating
    /// cast also matches its negative, overflow, infinity and NaN results.
    pub fn maximum_step_count(&self) -> u32 {
        self.maximum_steps as u32
    }
}

#[derive(Clone, Debug)]
pub struct WaterSettings {
    pub waves: WaterWaveSettings,
    pub colour: [f32; 3],
    pub extinction: f32,
    pub inscattering: f32,
    pub refraction: f32,
    pub specular_power: f32,
    pub shore_foam_luminance: f32,
    pub underwater_visibility_boost: f32,
    pub sky_colour: [f32; 3],
    pub sky_cube_radius: f32,
    pub sky_cube: String,
    pub caustics: CausticSettings,
    pub reflection: WaterReflectionSettings,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflection_step_limits_match_executed_shader_conversion() {
        let mut count = 0;
        for row in include_str!("../../../tests/fixtures/water_step_conversion.txt").lines() {
            let words = row
                .split_whitespace()
                .map(|word| u32::from_str_radix(word, 16).unwrap())
                .collect::<Vec<_>>();
            let settings = WaterReflectionSettings {
                thickness: [0.; 4],
                thickness_power: 0.,
                vertical_fade: 0.,
                border_fade: 0.,
                normal_softening: 0.,
                maximum_steps: f32::from_bits(words[0]),
            };
            assert_eq!(
                settings.maximum_step_count(),
                words[1],
                "float bits {:08x}",
                words[0]
            );
            count += 1;
        }
        assert_eq!(count, 52);
    }
}

impl WaterSettings {
    /// Read the effective material/environment, preserving registered defaults
    /// through ParameterSource. Program options and dynamic SSR admission belong
    /// to their respective owners, rather than this uniform-value resolver.
    pub fn resolve(read: &impl ParameterSource) -> Result<Self, String> {
        let colour = read.vector("water/watercolor")?;
        let sky = read.vector("water/skycolor")?;
        Ok(Self {
            waves: WaterWaveSettings {
                maximum_octaves: read.scalar("ocean/perlinmaxoctaves")?,
                anti_alias: read.scalar("ocean/perlinantialias")?,
                frequency: read.scalar("ocean/perlinfrequency")?,
                speed: read.scalar("ocean/perlinspeed")?,
                persistence: read.scalar("ocean/perlinpersistence")?,
                amplitude: read.scalar("ocean/perlinamplitude")?,
            },
            colour: [colour[0], colour[1], colour[2]],
            extinction: read.scalar("water/extinction")?,
            inscattering: read.scalar("water/inscattering")?,
            refraction: read.scalar("water/refraction")?,
            specular_power: read.scalar("water/specularpower")?,
            shore_foam_luminance: read.scalar("water/shorefoamluminance")?,
            underwater_visibility_boost: read.scalar("water/underwatervisibilityboost")?,
            sky_colour: [sky[0], sky[1], sky[2]],
            sky_cube_radius: read.scalar("water/skycubemapsphereradius")?,
            sky_cube: read.image_source("water/skycubemap")?,
            caustics: CausticSettings {
                image: read.image_source("water/causticstex")?,
                speed: read.scalar("water/causticsspeed")?,
                tiling: read.scalar("water/causticstiling")?,
                fade: read.scalar("water/causticsfade")?,
                chromatic_aberration: read.scalar("water/causticschromaticaberration")?,
            },
            reflection: WaterReflectionSettings {
                thickness: [
                    read.scalar("water/screenspacereflectionzthicknessmulnear")?,
                    read.scalar("water/screenspacereflectionzthicknessdistancenear")?,
                    read.scalar("water/screenspacereflectionzthicknessmulfar")?,
                    read.scalar("water/screenspacereflectionzthicknessdistancefar")?,
                ],
                thickness_power: read.scalar("water/screenspacereflectionzthicknesspow")?,
                vertical_fade: read.scalar("water/screenspacereflectionverticalfade")?,
                border_fade: read.scalar("water/screenspacereflectionborderfade")?,
                normal_softening: read.scalar("water/screenspacereflectionsoften")?,
                maximum_steps: read.scalar("water/screenspacereflectionmaxstepscount")?,
            },
        })
    }
}
