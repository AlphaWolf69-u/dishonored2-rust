//! 306840's environment inputs, retaining the original units and defaults.
use super::world::{FogRamp, WorldFog, WorldFogEffects, WorldFogInput};
use crate::assets::{
    archive::Catalog,
    entities::EnvironmentRenderParameters,
    environment::{ParameterReader, ParameterSource},
};
use bevy_math::{Vec3, Vec4};

pub struct FogSettings {
    pub shadow: FogShadowSettings,
    pub fog: WorldFog,
    pub effects: WorldFogEffects,
    pub inverse_frequency: f32,
    pub focus_distance: f32,
    pub light_canvas: LightCanvasSettings,
    pub color_lut: String,
    pub noise_map: String,
    pub rays: super::rays::RaySettings,
    pub sun_sprite: super::sun_sprite::SunSpriteSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FogShadowSettings {
    pub image: String,
    pub extent: f32,
}

/// 3110e0's second 2ff520 call uses its own environment parameters,
/// independently of the first canvas's renderer world wind.
#[derive(Debug, PartialEq)]
pub struct LightCanvasSettings {
    pub inverse_frequency: f32,
    pub focus_distance: f32,
    pub wind: Vec3,
}
impl FogSettings {
    /// Sun values are the renderer's current toward-sun direction and color,
    /// supplied by the same environment producer as clouds and parallel lights.
    pub fn load(
        catalog: &Catalog,
        parameters: &EnvironmentRenderParameters,
        sun: Option<(Vec3, Vec3)>,
        far_clip: f32,
    ) -> Result<Self, String> {
        let read = ParameterReader {
            catalog,
            parameters,
        };
        Self::resolve(&read, sun, far_clip)
    }

    pub fn resolve(
        read: &impl ParameterSource,
        sun: Option<(Vec3, Vec3)>,
        far_clip: f32,
    ) -> Result<Self, String> {
        let ramp = |prefix: &str| -> Result<FogRamp, String> {
            Ok(FogRamp {
                start: read.scalar(&format!("{prefix}/startdistance"))?,
                end: read.scalar(&format!("{prefix}/enddistance"))?,
                extinction: read.scalar(&format!("{prefix}/extinctioncoefficient"))?,
                color_start: Vec4::from_array(read.vector(&format!("{prefix}/colorstart"))?),
                color_end: Vec4::from_array(read.vector(&format!("{prefix}/colorend"))?),
                use_color_end: read.boolean(&format!("{prefix}/usecolorend"))?,
            })
        };
        // 306840 reads these inputs before selecting the actual-sun values.
        let theta = read.scalar("atm/worldfog/scatteringangletheta")?;
        let phi = read.scalar("atm/worldfog/scatteringanglephi")?;
        // 306840 reads the artist-facing colour, not the separate renderer
        // scatteringColor parameter (live metadata key 1422aebc8).
        let authored_color =
            Vec4::from_array(read.vector("atm/worldfog/artistscatteringcolor")?).truncate();
        let direction = if read.boolean("atm/worldfog/useactualsundirection")? {
            sun.ok_or("fog requests unresolved current sun direction")?
                .0
        } else {
            Vec3::from_array(crate::render_shadow::environment_sun_direction(theta, phi))
        };
        let color = if read.boolean("atm/worldfog/useactualsuncolor")? {
            sun.ok_or("fog requests unresolved current sun color")?.1
        } else {
            authored_color
        };
        let input = WorldFogInput {
            distance: ramp("atm/worlddistancefog")?,
            vertical: ramp("atm/worldverticalfog")?,
            height_start: read.scalar("atm/worldverticalfog/verticalstartdistance")?,
            height_end: read.scalar("atm/worldverticalfog/verticalenddistance")?,
            density_falloff: read.scalar("atm/worldverticalfog/densityfalloffspeed")?,
            scattering_intensity: read.scalar("atm/worldfog/scatteringintensity")?,
            scattering_phase: read.scalar("atm/worldfog/scatteringphase")?,
            scattering_direction: direction,
            scattering_color: color,
            god_rays: read.boolean("atm/worldfog/enablegodrays")?,
            turbulence: read.boolean("atm/worldfogturbulence/useturbulence")?,
        };
        Ok(Self {
            shadow: FogShadowSettings {
                image: read.image_source("ocean/projectedshadowheightmap")?,
                extent: read.scalar("ocean/projectedshadowheightmapextent")?,
            },
            fog: WorldFog::prepare(input),
            rays: super::rays::RaySettings::resolve(read, sun.map(|(direction, _)| direction))?,
            sun_sprite: super::sun_sprite::SunSpriteSettings::resolve(
                read,
                sun.map(|(direction, _)| direction),
            )?,
            effects: WorldFogEffects {
                far_clip,
                scattering_depth_start: read.scalar("atm/worldfog/scatteringdepthdist")?,
                god_ray_shadow: read.scalar("atm/worldfog/godrayshadow")?,
                turbulence_start: read.scalar("atm/worldfogturbulence/startdistance")?,
                turbulence_end: read.scalar("atm/worldfogturbulence/enddistance")?,
                turbulence_color: Vec4::from_array(read.vector("atm/worldfogturbulence/color")?)
                    .truncate(),
            },
            inverse_frequency: read.scalar("atm/worldfogturbulence/inversebasefrequency")?,
            focus_distance: read.scalar("atm/worldfogturbulence/focusdistance")?,
            light_canvas: LightCanvasSettings {
                inverse_frequency: read
                    .scalar("atm/lightscatteringturbulence/inversebasefrequency")?,
                focus_distance: read.scalar("atm/lightscatteringturbulence/focusdistance")?,
                wind: Vec4::from_array(read.vector("atm/lightscatteringturbulence/winddirection")?)
                    .truncate(),
            },
            noise_map: read.image_source("noisemap")?,
            color_lut: read
                .image_source("atm/worlddistancefog/colorlut")?
                .trim_matches('"')
                .trim_end_matches(".curvefloat")
                .to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities;
    #[test]
    #[ignore = "requires installed level environments, defaults and fog lookup images"]
    fn installed_world_fog_inputs_resolve_all_levels() {
        let catalog = Catalog::open(&crate::assets::installation::game_base().unwrap()).unwrap();
        let read = |name: &str| -> Result<Option<String>, String> {
            catalog
                .read(&format!("generated/decls/env/{name}.decl"))
                .map(|v| v.map(|v| entities::declaration_text(&v).into_owned()))
                .map_err(|e| e.to_string())
        };
        let mut count = 0;
        for path in catalog.paths_with_suffix(".entities") {
            let bytes = catalog.read(&path).unwrap().unwrap();
            let doc = entities::parse(&entities::declaration_text(&bytes)).unwrap();
            let Some(name) = entities::world_environment(&doc) else {
                continue;
            };
            let source = read(&name).unwrap().unwrap();
            let parameters =
                entities::environment_render_parameters_with_inheritance(&name, &source, read)
                    .unwrap();
            let lighting = entities::environment_lighting_with_inheritance(&source, read).unwrap();
            let sun = lighting
                .sun_theta_degrees
                .zip(lighting.sun_phi_degrees)
                .zip(lighting.sun_color)
                .map(|((theta, phi), color)| {
                    (
                        Vec3::from_array(crate::render_shadow::environment_sun_direction(
                            theta, phi,
                        )),
                        Vec3::from_array(crate::render_light::sun_ground_color(color)),
                    )
                });
            let fog = FogSettings::load(&catalog, &parameters, sun, 7000.)
                .unwrap_or_else(|e| panic!("{path}: {e}"));
            if name == "env/campaign/institute/institute_p/institut_ext01" {
                // Live 306840 output: both colour parameters coexist, but the
                // artistScatteringColor key feeds this routine. Keep their
                // different values from silently being conflated again.
                assert_eq!(
                    fog.fog.scattering_color.to_array().map(f32::to_bits),
                    [0x3ea1_13a5, 0x3ec2_d363, 0x3edc_b44a],
                    "{path}: differs from the game's completed fog record"
                );
            }
            let prepared =
                crate::rendering::environment_parameters::EnvironmentParameters::capture(
                    &catalog,
                    &parameters,
                    |read| FogSettings::resolve(read, sun, 7000.),
                )
                .unwrap();
            let resident = FogSettings::resolve(&prepared, sun, 7000.).unwrap();
            assert_eq!(format!("{:?}", fog.fog), format!("{:?}", resident.fog));
            assert_eq!(
                format!("{:?}", fog.effects),
                format!("{:?}", resident.effects)
            );
            assert_eq!(fog.color_lut, resident.color_lut);
            assert_eq!(fog.light_canvas, resident.light_canvas);
            assert_eq!(fog.shadow, resident.shadow);
            assert_eq!(fog.sun_sprite.parameters, resident.sun_sprite.parameters);
            assert_eq!(fog.sun_sprite.image, resident.sun_sprite.image);
            let noise = crate::material::compiled_image_stem(&catalog, &fog.noise_map).unwrap();
            let noise_chain =
                crate::texture::compressed_inline_chain(&catalog.read(&noise).unwrap().unwrap())
                    .unwrap();
            assert_eq!(
                noise_chain.info.format,
                crate::texture::ImageFormat::Rgba8,
                "{noise}"
            );
            let image = crate::material::compiled_image_stem(&catalog, &fog.color_lut)
                .unwrap_or_else(|| panic!("{path}: unresolved fog curve {}", fog.color_lut));
            let chain =
                crate::texture::compressed_inline_chain(&catalog.read(&image).unwrap().unwrap())
                    .unwrap();
            assert_eq!(
                chain.info.format,
                crate::texture::ImageFormat::R16Float,
                "{image}"
            );
            assert!(
                fog.fog.distance.is_finite() && fog.fog.vertical.is_finite(),
                "{path}"
            );
            println!(
                "{path}: fog flags={} curve={} turbulence_frequency={} focus={}",
                fog.fog.flags, image, fog.inverse_frequency, fog.focus_distance
            );
            count += 1;
        }
        assert!(count >= 30, "only {count} levels audited");
    }
}
