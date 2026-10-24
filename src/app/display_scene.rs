//! Prepared level display inputs. Gameplay updates consume typed values only;
//! images and registered defaults are read from archives once during setup.
use crate::{
    assets::{archive::Catalog, environment::ParameterReader},
    entities::EnvironmentRenderParameters,
    rendering::{
        bevy_backend::{
            auto_exposure::ExposureParameters,
            display_scene::{DisplayFrame, DisplaySceneFrame},
            tone_mapping::ToneMappingParameters,
        },
        environment::{AreaEnvironment, EnvironmentTrack},
        tone_mapping::{ToneCurve, ToneSettings},
    },
};
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use std::collections::HashMap;

const NAMES: [&str; 18] = [
    "env/tonemapping/overbright_level",
    "env/tonemapping/black_level",
    "env/tonemapping/white_level",
    "env/tonemapping/junction_point",
    "env/tonemapping/toe_strength",
    "env/tonemapping/shoulder_strength",
    "env/tonemapping/gamma_brightness",
    "env/autoexp/adapt_min_luminance",
    "env/autoexp/adapt_max_luminance",
    "env/autoexp/clip_shadows",
    "env/autoexp/clip_highlights",
    "env/autoexp/ev",
    "postfx/luminance/white_level",
    "env/autoexp/adapt_speed_up",
    "env/autoexp/adapt_speed_down",
    "env/autoexp/reference_camera_fps",
    "env/autoexp/fstop_bias",
    "postfx/luminance/customevbias",
];
struct AreaInputs {
    values: [Option<Vec4>; 18],
    cube: Option<Handle<Image>>,
}
#[derive(Resource)]
pub(super) struct DisplayScene {
    /// The pause screen FX's override environment (`pause_fx`).
    screen_fx: Option<AreaInputs>,
    world: [Vec4; 18],
    world_cube: Handle<Image>,
    areas: Vec<AreaInputs>,
    tracks: [EnvironmentTrack<Vec4>; 18],
    cube_track: EnvironmentTrack<(Handle<Image>, f32)>,
    lens: super::lens_scene::LensScene,
    camera_effects: super::camera_effects::CameraScene,
}
impl DisplayScene {
    pub(super) fn load(
        catalog: &Catalog,
        world: &EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
        screen_fx: Option<&EnvironmentRenderParameters>,
        images: &mut Assets<Image>,
    ) -> Result<Self, String> {
        let empty = EnvironmentRenderParameters { layers: Vec::new() };
        let default_reader = ParameterReader {
            catalog,
            parameters: &empty,
        };
        let reader = ParameterReader {
            catalog,
            parameters: world,
        };
        let read_values = |read: &ParameterReader<'_>| -> Result<[Vec4; 18], String> {
            NAMES
                .iter()
                .map(|name| read.vector(name).map(Vec4::from_array))
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .map_err(|_| "display parameter count changed".into())
        };
        let defaults = read_values(&default_reader)?;
        let values = read_values(&reader)?;
        let mut resident = HashMap::new();
        let default_cube = load_cube(
            catalog,
            &default_reader.image_source("env/tonemapping/colorcube")?,
            images,
            &mut resident,
        )?;
        let world_cube = load_cube(
            catalog,
            &reader.image_source("env/tonemapping/colorcube")?,
            images,
            &mut resident,
        )?;
        // The parameters an environment sets, and its colour cube.
        let mut prepare = |parameters: &EnvironmentRenderParameters| -> Result<AreaInputs, String> {
            let read = ParameterReader {
                catalog,
                parameters,
            };
            let mut values = [None; 18];
            for (index, name) in NAMES.iter().enumerate() {
                if parameters.parameter(name).is_some() {
                    values[index] = Some(Vec4::from_array(read.vector(name)?));
                }
            }
            let cube = if parameters.parameter("env/tonemapping/colorcube").is_some() {
                Some(load_cube(
                    catalog,
                    &read.image_source("env/tonemapping/colorcube")?,
                    images,
                    &mut resident,
                )?)
            } else {
                None
            };
            Ok(AreaInputs { values, cube })
        };
        let mut prepared = Vec::with_capacity(areas.len());
        for area in areas {
            prepared.push(prepare(&area.parameters)?);
        }
        let screen_fx_inputs = screen_fx.map(&mut prepare).transpose()?;
        Ok(Self {
            screen_fx: screen_fx_inputs,
            world: values,
            world_cube,
            areas: prepared,
            tracks: std::array::from_fn(|i| EnvironmentTrack::new(defaults[i])),
            cube_track: EnvironmentTrack::new((default_cube, 1.)),
            lens: super::lens_scene::LensScene::load(catalog, world, areas, images)?,
            camera_effects: super::camera_effects::CameraScene::load(catalog, world, areas, screen_fx)?,
        })
    }
    /// `screen_fx` is the pause screen FX's weight in the override slots.
    fn update(
        &mut self,
        areas: &[AreaEnvironment],
        screen_fx: f32,
        position: [f32; 3],
        world_duration: f32,
        delta: f32,
        camera_axis: f32,
    ) -> Result<
        (
            ToneMappingParameters,
            Handle<Image>,
            f32,
            ExposureParameters,
            crate::rendering::bevy_backend::lens_scene::LensFrame,
        ),
        String,
    > {
        if areas.len() != self.areas.len() {
            return Err("display areas changed without preparing their inputs".into());
        }
        let active: Vec<_> = areas
            .iter()
            .enumerate()
            .filter(|(_, area)| area.contains(position))
            .collect();
        let mut values = [Vec4::ZERO; 18];
        for i in 0..18 {
            let (mut source, mut target, mut duration) = (None, self.world[i], world_duration);
            for (index, area) in &active {
                if let Some(value) = self.areas[*index].values[i] {
                    source = Some(*index);
                    target = value;
                    duration = area.blend_duration;
                }
            }
            values[i] =
                self.tracks[i].advance_with(source, target, duration, delta, |start, target, t| {
                    (*target - *start) * t + *start
                });
            // 88e220's override stage, after the area/world blend: a slot's
            // value v with weight w (influence 1) gives base + (v - base)·w.
            if let Some(v) = self.screen_fx.as_ref().and_then(|s| s.values[i]).filter(|_| screen_fx > 0.) {
                values[i] += (v - values[i]) * screen_fx.clamp(0., 1.);
            }
        }
        let (mut source, mut target, mut duration) =
            (None, self.world_cube.clone(), world_duration);
        for (index, area) in &active {
            if let Some(cube) = &self.areas[*index].cube {
                source = Some(*index);
                target = cube.clone();
                duration = area.blend_duration;
            }
        }
        // 88e220 publishes the target and interpolation fraction for this
        // special image parameter; generic images' halfway switch is separate.
        let (cube, fraction) =
            self.cube_track
                .advance_with(source, (target, 1.), duration, delta, |_, target, t| {
                    (target.0.clone(), t)
                });
        let post_process = self.lens.settings();
        let hdr_white_level = post_process.hdr_white_level(values[2].x);
        // Live cvars: r_gamma, r_brightness, r_skipPostProcess.
        let cvars = {
            let c = crate::platform::console::shared().read();
            (
                c.cvars.float("r_gamma").unwrap_or(2.2),
                c.cvars.float("r_brightness").unwrap_or(1.0),
                c.cvars.int("r_skipPostProcess").unwrap_or(0) != 0,
            )
        };
        let exposure = ExposureParameters {
            range: Vec4::new(values[7].x, values[8].x, values[9].x, values[10].x),
            adaptation: Vec4::new(values[11].x, hdr_white_level, values[13].x, values[14].x),
            camera: Vec4::new(values[15].x, values[16].x, values[17].x, 0.),
        }
        .validate()?;
        let parameters = ToneMappingParameters {
            curve: ToneCurve::new(ToneSettings {
                overbright: values[0].x,
                black: values[1].x,
                white: values[2].x,
                junction: values[3].x,
                toe: values[4].x,
                shoulder: values[5].x,
            }),
            // FUN_140326af0 overwrites env/tonemapping/gamma_brightness with
            // (1 / (r_gamma / 2.2), r_brightness) before the tone-mapping draw.
            gamma: 1.0 / (cvars.0 / 2.2),
            brightness: cvars.1,
            exposure: 1.,
            // The ToneMapping1 variant needs neither r_skipToneMapping nor
            // r_skipPostProcess (notes-gamma.md).
            use_curve: !post_process.skip_tone_mapping && !cvars.2,
            use_colour_cube: true,
        };
        let lens = self.lens.update(
            areas,
            position,
            world_duration,
            delta,
            camera_axis,
            hdr_white_level,
        )?;
        Ok((parameters, cube, fraction, exposure, lens))
    }

    /// The override colour cubes (`m_overrideColorCubes`, 88e220): the pause
    /// screen FX's cube with its weight (weights summing above one are
    /// scaled down to one; a single slot never is).
    fn cube_overrides(&self, screen_fx: f32) -> Option<crate::rendering::bevy_backend::display_scene::DisplayCubeOverrides> {
        let cube = self.screen_fx.as_ref()?.cube.clone()?;
        (screen_fx > 0.).then(|| crate::rendering::bevy_backend::display_scene::DisplayCubeOverrides {
            images: [cube.clone(), cube.clone(), cube.clone(), cube],
            weights: Vec4::new(screen_fx.min(1.), 0., 0., 0.),
        })
    }
}

fn load_cube(
    catalog: &Catalog,
    name: &str,
    images: &mut Assets<Image>,
    resident: &mut HashMap<String, Handle<Image>>,
) -> Result<Handle<Image>, String> {
    let key = if name.eq_ignore_ascii_case("_defaultColorCube") {
        "_defaultColorCube".into()
    } else {
        crate::material::compiled_image_stem(catalog, name)
            .ok_or_else(|| format!("unresolved display colour cube {name}"))?
    };
    if let Some(image) = resident.get(&key) {
        return Ok(image.clone());
    }
    let (pixels, format) = if key == "_defaultColorCube" {
        (
            crate::rendering::bevy_backend::colour_cube::identity_pixels(),
            TextureFormat::Rgba8Unorm,
        )
    } else {
        let bytes = catalog
            .read(&key)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("missing colour cube {key}"))?;
        let volume = crate::texture::compressed_inline_volume(&bytes)?;
        if (
            volume.info.width,
            volume.info.height,
            volume.depth,
            volume.info.mip_count,
        ) != (32, 32, 32, 1)
        {
            return Err(format!(
                "display colour cube requires32^3 with one mip: {key}"
            ));
        }
        let format = match volume.info.format {
            crate::texture::ImageFormat::Rgba8 => TextureFormat::Rgba8Unorm,
            crate::texture::ImageFormat::Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
            other => return Err(format!("unsupported colour-cube format {other:?}: {key}")),
        };
        (volume.payload, format)
    };
    let handle = images.add(Image::new(
        Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 32,
        },
        TextureDimension::D3,
        pixels,
        format,
        RenderAssetUsages::RENDER_WORLD,
    ));
    resident.insert(key, handle.clone());
    Ok(handle)
}

pub(super) fn publish_display_frame(
    scene: Res<super::SceneData>,
    mut display: Option<ResMut<DisplayScene>>,
    clock: Res<crate::gameplay::game_clock_runtime::ClockRuntime>,
    screen_fx: Option<Res<super::pause_fx::PauseScreenFx>>,
    mut published_screen_fx: Local<f32>,
    mut cameras: Query<(Entity, &GlobalTransform, &mut ExposureParameters), With<Camera3d>>,
    mut output: ResMut<DisplaySceneFrame>,
    mut last_error: Local<Option<String>>,
) {
    let Some(display) = display.as_mut() else {
        output.0 = None;
        return;
    };
    // A paused frame keeps the environment (its tracks do not advance), but
    // the override slots still apply: the pause screen FX blends in.
    let weight = screen_fx.as_ref().map_or(0., |f| f.weight);
    if !clock.advanced() && weight == *published_screen_fx {
        return;
    }
    *published_screen_fx = weight;
    let advanced = clock.advanced();
    output.0 = None;
    let result = (|| -> Result<DisplayFrame, String> {
        let clock = clock.clock().map_err(str::to_owned)?;
        let delta = if advanced { clock.delta(3).ok_or("display gameplay clock unavailable")? } else { 0. };
        let (camera, transform, mut exposure) = match cameras.single_mut() {
            Ok(camera) => camera,
            Err(_) => return Err("display requires one exposure-controlled gameplay camera".into()),
        };
        let pose = transform.compute_transform();
        let view = crate::render_shadow::light_view(
            super::view_to_game(pose.translation),
            [
                super::view_to_game(pose.rotation * Vec3::NEG_Z),
                super::view_to_game(pose.rotation * Vec3::NEG_X),
                super::view_to_game(pose.rotation * Vec3::Y),
            ],
        );
        let (parameters, cube, cube_weight, frame_exposure, lens) = display.update(
            &scene.area_environments,
            weight,
            super::view_to_game(transform.translation()),
            scene.environment_blend_duration,
            delta,
            view[0][2] + view[2][1],
        )?;
        *exposure = frame_exposure;
        let camera_effects = display.camera_effects.update(
            &scene.area_environments,
            weight,
            super::view_to_game(transform.translation()),
            scene.environment_blend_duration,
            delta,
        )?;
        Ok(DisplayFrame {
            camera,
            camera_effects: Some(camera_effects),
            camera_blur: Some(display.camera_effects.blur_resources().clone()),
            parameters,
            cube,
            weight: cube_weight,
            custom_ev: frame_exposure.camera.z,
            overrides: display.cube_overrides(weight),
            lens: Some(lens),
        })
    })();
    match result {
        Ok(frame) => {
            output.0 = Some(frame);
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Display publication pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}

#[cfg(test)]
#[path = "display_scene/tests.rs"]
mod tests;
