//! Level preparation and camera-area publication for the dedicated water owner.
use crate::{
    archive,
    entities::EnvironmentRenderParameters,
    rendering::{
        bevy_backend::water_scene::{
            WaterFrame, WaterImageSources, WaterMaterials, WaterSceneFrame,
        },
        environment::AreaEnvironment,
        view_history::{ViewMatrices, ViewMatrixFrame, ViewMatrixHistory},
        water::parameters::{PreparedWaterValues, WaterTracks},
    },
};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Resource)]
pub(super) struct WaterScene {
    ocean_render_settings: crate::platform::game_settings::OceanRenderSettings,
    ocean_environment: crate::rendering::ocean::environment::OceanEnvironment,
    world: PreparedWaterValues,
    areas: Vec<(String, PreparedWaterValues)>,
    tracks: WaterTracks,
    camera: Option<Entity>,
    history: ViewMatrixHistory,
}
impl WaterScene {
    pub(super) fn load(
        catalog: &archive::Catalog,
        world: &EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
        materials: &WaterMaterials,
    ) -> Result<(Self, WaterImageSources), String> {
        let (_, ocean_render_settings) =
            crate::platform::game_settings::read_ocean_render_settings()?;
        let ocean_environment =
            crate::rendering::ocean::environment::OceanEnvironment::load(catalog, world, areas)?;
        let defaults = PreparedWaterValues::load(
            catalog,
            &EnvironmentRenderParameters { layers: Vec::new() },
            true,
        )?;
        let world = PreparedWaterValues::load(catalog, world, false)?;
        let prepared = areas
            .iter()
            .map(|a| {
                Ok((
                    a.id.clone(),
                    PreparedWaterValues::load(catalog, &a.parameters, false)?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let tracks = WaterTracks::new(&defaults)?;
        let mut sources = WaterImageSources::default();
        let mut cubes = HashMap::new();
        let mut arrays = HashMap::new();
        let mut admit = |role: usize, name: &str| -> Result<(), String> {
            if role == 0 {
                if sources.skies.contains_key(name) {
                    return Ok(());
                }
                let key = if catalog.contains(name) {
                    name.to_owned()
                } else {
                    crate::material::compiled_image_stem(catalog, name)
                        .ok_or_else(|| format!("unresolved water sky {name}"))?
                }
                .to_ascii_lowercase();
                let chain = if let Some(chain) = cubes.get(&key) {
                    Arc::clone(chain)
                } else {
                    let loaded = crate::material::load_cube_image(catalog, name)?;
                    let chain = Arc::new(loaded.image);
                    cubes.insert(key.clone(), chain.clone());
                    chain
                };
                let table = Arc::make_mut(&mut sources.skies);
                table.insert(name.into(), chain.clone());
                table.insert(key, chain);
            } else {
                if sources.caustics.contains_key(name) {
                    return Ok(());
                }
                let key = crate::material::image_load_resource(catalog, name)
                    .ok_or_else(|| format!("unresolved water caustics {name}"))?;
                let chain = if let Some(chain) = arrays.get(&key) {
                    Arc::clone(chain)
                } else {
                    let bytes = catalog
                        .read(&key)
                        .map_err(|e| e.to_string())?
                        .ok_or_else(|| format!("missing water caustics {key}"))?;
                    let chain = Arc::new(
                        crate::texture::compressed_inline_array(&bytes)
                            .map_err(|e| format!("{key}: {e}"))?,
                    );
                    arrays.insert(key.clone(), chain.clone());
                    chain
                };
                let table = Arc::make_mut(&mut sources.caustics);
                table.insert(name.into(), chain.clone());
                table.insert(key, chain);
            }
            Ok(())
        };
        for source in std::iter::once(&defaults)
            .chain(std::iter::once(&world))
            .chain(prepared.iter().map(|v| &v.1))
        {
            for (role, name) in source.image_sources() {
                admit(role, name)?;
            }
        }
        for material in materials.0.values().filter_map(|v| v.as_ref().ok()) {
            let world = material.world_colour.as_ref().ok().and_then(Option::as_ref);
            for block in material.parameters.iter().chain(
                world
                    .into_iter()
                    .flat_map(|source| source.parameters.iter()),
            ) {
                for parameter in &block.parameters {
                    for (role, name) in crate::rendering::water::IMAGE_PARAMETER_NAMES
                        .iter()
                        .enumerate()
                    {
                        if parameter
                            .name
                            .split(|&v| v == 0)
                            .next()
                            .unwrap()
                            .eq_ignore_ascii_case(name.as_bytes())
                        {
                            let crate::render_program::SerializedParameterValue::Name(value) =
                                &parameter.value
                            else {
                                return Err(format!(
                                    "water image scope has a nonimage value: {name}"
                                ));
                            };
                            let value =
                                std::str::from_utf8(value.split(|&v| v == 0).next().unwrap())
                                    .map_err(|e| e.to_string())?;
                            admit(role, value)?;
                        }
                    }
                }
            }
        }
        Ok((
            Self {
                ocean_render_settings,
                ocean_environment,
                world,
                areas: prepared,
                tracks,
                camera: None,
                history: ViewMatrixHistory::default(),
            },
            sources,
        ))
    }
    pub(super) fn update(
        &mut self,
        areas: &[AreaEnvironment],
        eye: [f32; 3],
        world_duration: f32,
        delta: f32,
    ) -> Result<crate::rendering::water::parameters::WaterViewParameters, String> {
        if areas.len() != self.areas.len()
            || areas
                .iter()
                .zip(&self.areas)
                .any(|(a, (id, _))| &a.id != id)
        {
            return Err("water areas changed without preparing their inputs".into());
        }
        let active: Vec<_> = areas
            .iter()
            .enumerate()
            .filter(|(_, a)| a.contains(eye))
            .map(|(i, a)| (i, &self.areas[i].1, a.blend_duration))
            .collect();
        Ok(self
            .tracks
            .update(&self.world, world_duration, &active, delta))
    }

    fn camera_frame(
        &mut self,
        camera: Entity,
        frame: u32,
        matrices: ViewMatrices,
    ) -> ViewMatrixFrame {
        if self.camera != Some(camera) {
            self.camera = Some(camera);
            self.history = ViewMatrixHistory::default();
        }
        self.history.update(frame, matrices)
    }
}
pub(super) fn publish_water_frame(
    scene: Res<super::SceneData>,
    mut water: Option<ResMut<WaterScene>>,
    clock: Res<crate::gameplay::game_clock_runtime::ClockRuntime>,
    view_clock: Res<crate::gameplay::game_clock_runtime::GameplayViewClock>,
    frame_count: Res<bevy::diagnostic::FrameCount>,
    cameras: Query<(Entity, &GlobalTransform, &Projection), With<Camera3d>>,
    mut output: ResMut<WaterSceneFrame>,
    mut last_error: Local<Option<String>>,
) {
    let Some(water) = water.as_mut() else {
        output.0 = None;
        return;
    };
    if !clock.advanced() {
        return;
    }
    output.0 = None;
    let result = (|| -> Result<WaterFrame, String> {
        let clock = clock.clock().map_err(str::to_owned)?;
        let delta = clock.delta(3).ok_or("water gameplay clock unavailable")?;
        let (camera, transform, projection) = cameras.single().map_err(|e| e.to_string())?;
        let Projection::Custom(custom) = projection else {
            return Err("water camera projection unavailable".into());
        };
        let projection = custom
            .get::<super::camera_projection::GameProjection>()
            .ok_or("water camera projection type unavailable")?;
        let pose = transform.compute_transform();
        let eye = super::view_to_game(pose.translation);
        let matrices = ViewMatrices::from_camera(
            eye,
            [
                super::view_to_game(pose.rotation * Vec3::NEG_Z),
                super::view_to_game(pose.rotation * Vec3::NEG_X),
                super::view_to_game(pose.rotation * Vec3::Y),
            ],
            projection.rows(),
        );
        let matrices = water.camera_frame(camera, frame_count.0, matrices);
        let values = water.update(
            &scene.area_environments,
            eye,
            scene.environment_blend_duration,
            delta,
        )?;
        Ok(WaterFrame {
            camera,
            ocean_render_settings: water.ocean_render_settings,
            values,
            ocean_environment: water.ocean_environment.update(
                &scene.area_environments,
                eye,
                scene.environment_blend_duration,
                delta,
            )?,
            matrices,
            local_time: view_clock
                .view_shader_times(crate::game_clock::RESET_VIEW_LOCAL_TIME_FLAG)?
                .local_time,
        })
    })();
    match result {
        Ok(frame) => {
            output.0 = Some(frame);
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Water publication pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
#[cfg(test)]
#[path = "water_scene/tests.rs"]
mod tests;
