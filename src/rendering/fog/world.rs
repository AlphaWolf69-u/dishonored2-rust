//! World distance/height fog parameters and shader calculations.
//! Evidence: FUN_140306840, FUN_140306150, and installed fogHelpers/renderWorld.

use bevy_math::{Vec3, Vec4};

fn saturate(value: f32) -> f32 {
    if value > 0. {
        value.min(1.)
    } else {
        0.
    }
}

/// Inputs after registered defaults and environment overrides have been resolved.
#[derive(Clone, Copy, Debug, Default)]
pub struct FogRamp {
    pub start: f32,
    pub end: f32,
    pub extinction: f32,
    pub color_start: Vec4,
    pub color_end: Vec4,
    pub use_color_end: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldFogInput {
    pub distance: FogRamp,
    pub vertical: FogRamp,
    pub height_start: f32,
    pub height_end: f32,
    pub density_falloff: f32,
    pub scattering_intensity: f32,
    pub scattering_phase: f32,
    pub scattering_direction: Vec3,
    pub scattering_color: Vec3,
    pub god_rays: bool,
    pub turbulence: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldFog {
    pub flags: u32,
    pub distance: Vec4,
    pub distance_start: Vec4,
    pub distance_end: Vec4,
    pub vertical: Vec4,
    pub vertical_start: Vec4,
    pub vertical_end: Vec4,
    pub height_start: f32,
    pub height_end: f32,
    pub density_falloff: f32,
    pub scattering_intensity: f32,
    pub scattering_phase: f32,
    pub scattering_direction: Vec3,
    pub scattering_color: Vec3,
}

impl WorldFog {
    /// FUN_140306840: enable ramps only for positive extinction, and retain
    /// authored units. Global volumetric-fog scale does not scale these ramps.
    pub fn prepare(input: WorldFogInput) -> Self {
        let mut output = Self {
            scattering_intensity: input.scattering_intensity,
            scattering_phase: input.scattering_phase,
            scattering_direction: input.scattering_direction,
            scattering_color: input.scattering_color,
            ..Self::default()
        };
        let ramp = |value: FogRamp| {
            let interval = (value.end - value.start).max(f32::from_bits(0x3400_0000));
            (
                Vec4::new(value.start, 1. / interval, value.extinction, 0.),
                value.color_start,
                if value.use_color_end {
                    value.color_end
                } else {
                    value.color_start
                },
            )
        };
        if input.distance.extinction > 0. {
            output.flags |= 8;
            (output.distance, output.distance_start, output.distance_end) = ramp(input.distance);
        }
        if input.vertical.extinction > 0. {
            output.flags |= 16;
            output.height_start = input.height_start;
            output.height_end = input.height_end;
            output.density_falloff = input.density_falloff;
            (output.vertical, output.vertical_start, output.vertical_end) = ramp(input.vertical);
        }
        if input.scattering_intensity > 0. {
            output.flags |= 32;
        }
        if input.god_rays {
            output.flags |= 64;
        }
        if input.turbulence {
            output.flags |= 128;
        }
        output
    }

    /// FUN_140306150's alphaFog/worldType upload. The turbulence bit is masked
    /// out; volume-only and unsupported combinations upload zero.
    pub fn alpha_world_type(flags: u32) -> Vec4 {
        match flags & !128 {
            8 | 16 | 24 | 40 | 48 | 56 | 72 | 80 | 88 | 104 | 112 | 120 => Vec4::new(
                u8::from(flags & 8 != 0) as f32,
                u8::from(flags & 32 != 0) as f32,
                u8::from(flags & 16 != 0) as f32,
                if flags & 64 != 0 { 2. } else { 0. },
            ),
            _ => Vec4::ZERO,
        }
    }

    /// Companion alphaFog/volumeType upload from the same routine.
    pub fn alpha_volume_type(flags: u32) -> Vec3 {
        match flags & !128 {
            1 | 3 | 5 | 7 => Vec3::new(
                1.,
                u8::from(flags & 2 != 0) as f32,
                u8::from(flags & 4 != 0) as f32,
            ),
            _ => Vec3::ZERO,
        }
    }

    /// fogHelpers: the sampled curve supplies opacity, independently of RGB.
    pub fn distance_color(&self, normalized_distance: f32, curve: f32) -> Vec4 {
        let color = self
            .distance_start
            .lerp(self.distance_end, normalized_distance);
        color
            .truncate()
            .extend(1. - (-self.distance.z * curve).exp())
    }

    /// fogHelpers' exponential height integral, in the game's Z-up coordinates.
    pub fn vertical_color(
        &self,
        view_z: f32,
        camera_z: f32,
        distance: f32,
        normalized_distance: f32,
    ) -> Vec4 {
        let altitude = camera_z - self.height_start;
        let dy = view_z + if view_z > 0. { 0.00001 } else { -0.00001 };
        let hit = altitude + distance * dy;
        let density = ((-self.density_falloff * altitude).exp()
            - (-self.density_falloff * hit).exp())
            / (self.density_falloff * dy);
        // FXC lowers saturate(exp(...)) to min(exp(...), 1), omitting the
        // redundant lower bound. DXBC min chooses 1 for a NaN exponential;
        // f32::min preserves that observed zero-fog result at zero falloff.
        let extinction = 1. - (-density).exp().min(1.);
        // This second subtraction of height_start is present in the shader.
        let height = saturate((hit - self.height_start) / (self.height_end - self.height_start));
        let color = self
            .vertical_start
            .lerp(self.vertical_end, height)
            .truncate()
            * extinction;
        color.extend((1. - (-self.vertical.z * normalized_distance).exp()) * extinction)
    }
}

/// FUN_140306150's optional monochrome conversion is applied to fog colors
/// at upload time. The caller supplies the engine's color-conversion weights.
pub fn monochrome_color(color: Vec4, weights: Vec3) -> Vec4 {
    Vec3::splat(color.truncate().dot(weights)).extend(1.)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_and_height_colors_match_installed_hlsl() {
        let expected = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fog_world_cases.txt"
        ))
        .split_whitespace()
        .map(|word| f32::from_bits(u32::from_str_radix(word, 16).unwrap()))
        .collect::<Vec<_>>();
        assert_eq!(expected.len(), 64 * 8);
        for id in 0..64 {
            let normalized = (id % 9) as f32 / 8.;
            let height_start = (id % 3) as f32 - 1.;
            let fog = WorldFog {
                distance: Vec4::new(10., 1. / 90., 3.5, 0.),
                distance_start: Vec4::new(0.1, 0.3, 0.8, 0.2),
                distance_end: Vec4::new(1.1, 0.8, 0.2, 0.7),
                vertical: Vec4::new(2., 1. / 70., 0.4, 0.),
                vertical_start: Vec4::new(0.2, 0.7, 1.1, 0.3),
                vertical_end: Vec4::new(1.8, 0.4, 0.1, 0.8),
                height_start,
                height_end: height_start + if id & 8 != 0 { 0. } else { 12. },
                density_falloff: if id & 4 != 0 { 0. } else { 0.03 },
                ..Default::default()
            };
            let colors = [
                fog.distance_color(normalized, (id % 7) as f32 / 6.),
                fog.vertical_color(
                    ((id % 5) as f32 - 2.) * 0.25,
                    (id % 11) as f32 - 5.,
                    (id % 13) as f32 * 2.,
                    normalized,
                ),
            ];
            for (field, actual) in colors.into_iter().flat_map(|v| v.to_array()).enumerate() {
                let reference = expected[id * 8 + field];
                assert!(
                    (actual - reference).abs() < 0.0003,
                    "original fog case{id} field{field}: {actual} != {reference}"
                );
            }
        }
    }

    #[test]
    fn fog_parameter_packing_keeps_units_and_disabled_ramps_zero() {
        let base = FogRamp {
            start: 100.,
            end: 5000.,
            extinction: 3.5,
            color_start: Vec4::new(1., 2., 3., 0.4),
            color_end: Vec4::splat(10.),
            use_color_end: false,
        };
        let fog = WorldFog::prepare(WorldFogInput {
            distance: base,
            scattering_intensity: 0.3,
            god_rays: true,
            turbulence: true,
            ..Default::default()
        });
        assert_eq!(fog.flags, 8 | 32 | 64 | 128);
        assert_eq!(fog.distance, Vec4::new(100., 1. / 4900., 3.5, 0.));
        assert_eq!(fog.distance_end, base.color_start);
        assert_eq!(fog.vertical, Vec4::ZERO);
        assert_eq!(
            WorldFog::alpha_world_type(fog.flags),
            Vec4::new(1., 1., 0., 2.)
        );
        let equal = WorldFog::prepare(WorldFogInput {
            distance: FogRamp {
                end: 100.,
                use_color_end: true,
                ..base
            },
            ..Default::default()
        });
        assert_eq!(equal.distance.y, 8_388_608.);
        assert_eq!(equal.distance_end, base.color_end);
        assert_eq!(WorldFog::alpha_world_type(32), Vec4::ZERO);
        assert_eq!(WorldFog::alpha_volume_type(7 | 128), Vec3::ONE);
        assert_eq!(WorldFog::alpha_volume_type(8), Vec3::ZERO);
    }
}
