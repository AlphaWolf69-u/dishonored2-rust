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
    ray_world: crate::rendering::environment_parameters::EnvironmentParameters,
    ray_areas: Vec<crate::rendering::environment_parameters::EnvironmentParameters>,
    ray_tracks: crate::rendering::environment_parameters::EnvironmentTracks,
    camera: Option<Entity>,
    history: ViewMatrixHistory,
    contacts: crate::rendering::environment_contacts::EnvironmentViewContacts,
    swimming_contacts:crate::rendering::environment_contacts::EnvironmentViewContacts,
}
impl WaterScene {
    pub(super) fn load(
        catalog: &archive::Catalog,
        world: &EnvironmentRenderParameters,
        areas: &[AreaEnvironment],
        materials: &WaterMaterials,
    ) -> Result<(Self, WaterImageSources), String> {
        Self::load_with_model_colours(catalog,world,areas,materials,&crate::rendering::bevy_backend::water_scene::WaterModelColours::default())
    }

    pub(super) fn load_with_model_colours(
        catalog:&archive::Catalog,world:&EnvironmentRenderParameters,areas:&[AreaEnvironment],
        materials:&WaterMaterials,models:&crate::rendering::bevy_backend::water_scene::WaterModelColours,
    )->Result<(Self,WaterImageSources),String> {
        let (_, ocean_render_settings) =
            crate::platform::game_settings::read_ocean_render_settings()?;
        let ocean_environment =
            crate::rendering::ocean::environment::OceanEnvironment::load(catalog, world, areas)?;
        let defaults = PreparedWaterValues::load(
            catalog,
            &EnvironmentRenderParameters { layers: Vec::new() },
            true,
        )?;
        use crate::rendering::environment_parameters::{EnvironmentParameters, EnvironmentTracks};
        let ray_world = EnvironmentParameters::capture(catalog, world, |read| {
            crate::rendering::water::rays::WaterRaySettings::resolve(read)
        })?;
        let ray_defaults = EnvironmentParameters::defaults(catalog, &ray_world)?;
        let ray_areas = areas.iter().map(|area| {
            EnvironmentParameters::supplied(catalog, &area.parameters, &ray_world)
        }).collect::<Result<Vec<_>, String>>()?;
        let ray_tracks = EnvironmentTracks::new(&ray_defaults);
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
        for (name,material) in materials.0.iter().filter_map(|(name,v)| v.as_ref().ok().map(|v|(name,v))) {
            let world = material.world_colour.as_ref().ok().and_then(Option::as_ref);
            let model = models.0.get(name).and_then(|v|v.as_ref().ok()).and_then(Option::as_ref);
            for block in material.parameters.iter().chain(
                world
                    .into_iter()
                    .flat_map(|source| source.parameters.iter()),
            ).chain(model.into_iter().flat_map(|source|source.parameters.iter())) {
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
        // The ocean owns additional authored masks which placed water does not
        // reference. Preserve the same archive identities as OceanEnvironment.
        for name in ocean_environment.image_sources() {
            if sources.skies.contains_key(name) || sources.caustics.contains_key(name) {
                continue;
            }
            let image = if let Some(payload) = crate::material::intrinsic_image_pixels(name) {
                let mut packed_format = 0x54a;
                crate::texture::normalize_image_format(&mut packed_format)?;
                crate::texture::InlineCompressedChain {
                    info: crate::texture::BimageInfo {
                        width: 16, height: 16, mip_count: 1, packed_format,
                        format: crate::texture::ImageFormat::Rgba8,
                    },
                    payload,
                }
            } else {
                let bytes = catalog.read(name).map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("missing ocean image {name}"))?;
                crate::texture::compressed_inline_chain(&bytes)
                    .map_err(|e| format!("{name}: {e}"))?
            };
            Arc::make_mut(&mut sources.ocean).insert(name.clone(), Arc::new(image));
        }
        Ok((
            Self {
                ocean_render_settings,
                ocean_environment,
                world,
                areas: prepared,
                tracks,
                ray_world,
                ray_areas,
                ray_tracks,
                camera: None,
                history: ViewMatrixHistory::default(),
                contacts:crate::rendering::environment_contacts::EnvironmentViewContacts::new(0.),
                swimming_contacts:crate::rendering::environment_contacts::EnvironmentViewContacts::new(0.),
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
        let mut values = self
            .tracks
            .update(&self.world, world_duration, &active, delta);
        let ray_areas = active.iter().map(|(i, _, duration)| {
            (*i, &self.ray_areas[*i], *duration)
        }).collect::<Vec<_>>();
        values.rays = self.ray_tracks.update(&self.ray_world, world_duration, &ray_areas, delta)?.clone();
        Ok(values)
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
    mut physics:Option<ResMut<super::physics_scene::PhysicsScene>>,
    volumes:Option<Res<super::state::EnvironmentVolumes>>,
    mut selection:ResMut<crate::rendering::bevy_backend::water_scene::WaterViewSelection>,
    mut last_error: Local<Option<String>>,
    player:Res<super::player::PlayerState>,
    screen_effects:Res<crate::rendering::bevy_backend::screen_effects::ScreenEffectMaterials>,
    mut queries: ResMut<crate::rendering::bevy_backend::physics_queries::PhysicsQueryFrame>,
    mut query_error: Local<Option<String>>,
) {
    let Some(water) = water.as_mut() else {
        output.0 = None;
        queries.0 = None;
        return;
    };
    if !clock.advanced() {
        return;
    }
    output.0 = None;
    queries.0 = None;
    selection.0=None;
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
        let head=Vec3::from_array(eye);
        let (body,body_axis)=if let Some(player)=player.player.as_ref().filter(|_|!player.debug_free_fly) {
            (player.physics.position(),player.physics.axis())
        } else {
            let forward=Vec3::from_array(super::view_to_game(pose.rotation*Vec3::NEG_Z));
            (head-Vec3::Z*super::controls::DEBUG_CAMERA_EYE_HEIGHT,
                crate::gameplay::player::physics::yaw_axis(forward.y.atan2(forward.x)))
        };
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
        let console=crate::platform::console::shared().read();
        let ocean_render_settings=crate::platform::game_settings::OceanRenderSettings{
            tile_size:console.cvars.get("r_oceanTileSize").ok_or("ocean tile cvar unavailable")?.float,
            skip:console.cvars.get("r_skipOcean").ok_or("ocean skip cvar unavailable")?.int!=0,
        };
        let mut ocean_environment=water.ocean_environment.update(
            &scene.area_environments,eye,scene.environment_blend_duration,delta)?;
        let completed=if let Some(physics)=physics.as_mut() {
            physics.resolve_view(&mut water.contacts,eye)?
        } else {
            water.contacts.selection.update(None);
            false
        };
        let swimming_completed=if let Some(physics)=physics.as_mut() {
            physics.resolve_view(&mut water.swimming_contacts,body.to_array())?
        } else {false};
        let publication=water.contacts.selection.water_publication(|index|{
            let volume=volumes.as_ref().and_then(|v|v.0.get(index)).ok_or("selected water profile absent")?;
            Ok(volume.surface_material.as_deref().map(Arc::<str>::from))
        })?;
        publication.apply(&mut ocean_environment.view);
        if completed {selection.0=Some(publication);}
        let follow_waves=console.cvars.get("ark_player_follow_waves").ok_or("swimming wave cvar absent")?.int!=0;
        let head_wave_points=if swimming_completed && water.swimming_contacts.head_water_is_ocean && follow_waves {
            screen_effects.0.as_ref().map(|fx| {
                fx.swimming.wave_points(head,body,body_axis)
            })
        } else {None};
        Ok(WaterFrame {
            camera,
            head_wave_points,
            head_water_surface: swimming_completed.then_some(water.swimming_contacts.head_water_surface).flatten(),
            delta_seconds: delta,
            near_far: [projection.near, projection.far],
            parallel: scene.environment_lighting.and_then(|environment| {
                Some((
                    crate::render_shadow::environment_sun_direction(
                        environment.sun_theta_degrees?, environment.sun_phi_degrees?,
                    ),
                    crate::render_light::sun_ground_color(environment.sun_color?),
                ))
            }),
            ocean_render_settings,
            values,
            ocean_environment,
            matrices,
            local_time: view_clock
                .view_shader_times(crate::game_clock::RESET_VIEW_LOCAL_TIME_FLAG)?
                .local_time,
        })
    })();
    match result {
        Ok(frame) => {
            if let Some(physics) = physics.as_ref() {
                match physics.query_snapshot() {
                    Ok(world) => {
                        queries.0 = Some(Arc::new(crate::rendering::bevy_backend::physics_queries::PhysicsQueryPublication {
                            camera: frame.camera,
                            frame: frame.matrices.frame,
                            world,
                        }));
                        *query_error = None;
                    }
                    Err(error) => {
                        if query_error.as_ref() != Some(&error) {
                            warn!("Physics query publication pending: {error}");
                            *query_error = Some(error);
                        }
                    }
                }
            }
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
