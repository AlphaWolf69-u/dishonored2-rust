//! Gameplay camera effects run after transparency and before exposure/bloom.
use super::{
    auto_exposure::{ExposureParameters, ExposurePass, MaterialExposure},
    camera_view::{CameraAlphaViewInputs, CameraView, CameraViewConfig, CameraViewInputs},
    display_scene::DisplaySceneFrame,
    fog::FogSceneFrame,
    fog_scene::FogGpu,
    water_scene::WaterSceneFrame,
};
use bevy::{
    core_pipeline::{Core3d, Core3dSystems},
    prelude::*,
    render::{
        camera::ExtractedCamera,
        render_asset::RenderAssets,
        render_resource::{Texture, TextureView},
        renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery},
        storage::GpuShaderBuffer,
        sync_world::MainEntity,
        texture::GpuImage,
        view::{ExtractedView, ViewTarget},
        RenderApp,
    },
};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct CameraPass;

#[derive(Resource, Default)]
pub(super) struct CameraGpu {
    current: Option<(Entity, CameraView)>,
    completed: Option<(Entity, u32)>,
    screen_effects: super::screen_effects::ScreenEffects,
    screen_state: super::screen_effects::ScreenEffectState,
    particle_systems: super::particle_systems::ParticleSystems,
    world_particles: super::world_particle_state::WorldParticleState,
    screen_draws: Option<(Entity, u32, Vec<super::screen_effects::QuadDraw>)>,
    particles: Option<ParticleGpu>,
    screen_sources: Option<std::sync::Arc<super::screen_effects::ScreenEffectSources>>,
}
struct ParticleGpu {
    pass: super::particle_pass::ParticlePass,
    instances: usize,
    draws: usize,
    colour_format: bevy::render::render_resource::TextureFormat,
    depth_format: bevy::render::render_resource::TextureFormat,
    source: std::sync::Arc<super::screen_effects::ScreenEffectSources>,
}
impl CameraGpu {
    pub(super) fn capture_planes(
        &self,
        camera: Entity,
        frame: u32,
    ) -> Vec<(&'static str, &Texture)> {
        if self.completed != Some((camera, frame)) {
            return Vec::new();
        }
        let mut planes = self
            .current
            .as_ref()
            .map_or_else(Vec::new, |(_, v)| v.capture_planes());
        planes.extend(self.screen_effects.capture_planes());
        planes
    }
    pub(super) fn half_colour(&self, camera: Entity, frame: u32) -> Option<&TextureView> {
        (self.completed == Some((camera, frame)))
            .then(|| self.current.as_ref().map(|(_, v)| v.half_colour()))
            .flatten()
    }
}
pub(super) fn register(app: &mut App) {
    app.init_resource::<super::physics_queries::PhysicsQueryFrame>();
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::physics_queries::PhysicsQueryFrame,
    >::default());
    app.init_resource::<super::screen_effects::ScreenEffectMaterials>();
    app.init_resource::<super::world_effects::WorldEffectSources>();
    app.init_resource::<super::world_effects::WorldEffectGraphs>();
    app.init_resource::<super::world_effects::WorldEffectFrames>();
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::world_effects::WorldEffectGraphs,
    >::default());
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::world_effects::WorldEffectFrames,
    >::default());
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::world_effects::WorldEffectSources,
    >::default());
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::screen_effects::ScreenEffectMaterials,
    >::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<CameraGpu>().add_systems(
            Core3d,
            render_camera
                .in_set(Core3dSystems::PostProcess)
                .in_set(CameraPass)
                .before(ExposurePass),
        );
        render.add_systems(
            Core3d,
            (prepare_screen_effects, render_particles)
                .chain()
                .in_set(Core3dSystems::MainPass)
                .after(super::fog_scene::AtmospherePass)
                .before(bevy::core_pipeline::core_3d::main_transparent_pass_3d),
        );
    }
}

/// Submit the shared FX controller's live sprites into the gameplay HDR target,
/// after atmosphere/depth preparation and before ordinary transparency.
fn render_particles(
    sources: Res<super::screen_effects::ScreenEffectMaterials>,
    display: Res<DisplaySceneFrame>,
    water: Res<WaterSceneFrame>,
    fog_frame: Res<FogSceneFrame>,
    fog: Res<FogGpu>,
    shared: Res<super::fog_material::FogMaterialResources>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    mut gpu: ResMut<CameraGpu>,
    mut context: RenderContext,
    view: ViewQuery<(
        &MainEntity,
        &ExtractedView,
        &ExtractedCamera,
        &ViewTarget,
        &MaterialExposure,
    )>,
    mut last_error: Local<Option<String>>,
    mut reported: Local<bool>,
) {
    let Some(water) = water.0.as_ref() else {
        gpu.particles = None;
        return;
    };
    let (main, view, camera, target, exposure) = view.into_inner();
    if main.id() != water.camera {
        return;
    }
    let Some(sources) = sources.0.as_ref() else {
        gpu.particles = None;
        return;
    };
    if gpu
        .particles
        .as_ref()
        .is_some_and(|p| !std::sync::Arc::ptr_eq(&p.source, sources))
    {
        gpu.particles = None;
    }
    let result = (|| -> Result<(), String> {
        let (instances, draw_capacity) = gpu.particle_systems.capacity();
        if instances == 0 || draw_capacity == 0 {
            return Ok(());
        }
        if !camera.hdr
            || target.main_texture().sample_count() != 1
            || camera.physical_viewport_size != camera.physical_target_size
            || camera
                .viewport
                .as_ref()
                .is_some_and(|v| v.physical_position != UVec2::ZERO)
        {
            return Err("particle colour requires a resolved full gameplay view".into());
        }
        let frame = fog_frame
            .0
            .as_ref()
            .filter(|f| f.camera == water.camera)
            .ok_or("particle fog awaits the gameplay camera")?;
        let (depth, _) = fog
            .camera_depth(water.camera, water.matrices.frame)
            .ok_or("particle depth awaits completed atmosphere")?;
        let matrices = &water.matrices.current;
        let axes = [
            std::array::from_fn(|i| -matrices.view[2][i]),
            std::array::from_fn(|i| -matrices.view[0][i]),
            std::array::from_fn(|i| matrices.view[1][i]),
        ];
        let sprite_view = crate::rendering::particles::SpriteView {
            camera_position: matrices.position,
            camera_axes: axes,
            custom_basis: axes,
            emitter_basis: axes,
            owner_colour: [1.; 4],
            owner_scale: 1.,
        };
        let console = crate::platform::console::shared().read();
        let draws = gpu
            .particle_systems
            .publish_draws(water.matrices.frame, |pose| {
                Ok(super::screen_effects::ParticleDrawContext {
                    view: &sprite_view,
                    // This backend publishes a single logical gameplay origin.
                    // Both view origins coincide for this supported view path.
                    camera: crate::rendering::particles::view::ParticleCamera::select(
                        pose.position,
                        matrices.position,
                        matrices.position,
                        axes,
                    ),
                    view_z: std::array::from_fn(|i| -matrices.view[2][i]),
                    query_visibility: 1.,
                    alpha_scale: 1.,
                    cvars: &console.cvars,
                    areas: &frame.areas,
                })
            })?;
        drop(console);
        if draws.is_empty() {
            return Ok(());
        }
        let lens = display
            .0
            .as_ref()
            .filter(|f| f.camera == water.camera)
            .and_then(|f| f.lens.as_ref())
            .ok_or("particle HDR settings await the current view")?;
        let colour_format = target.main_texture_format();
        let depth_format = depth.alpha.full.format();
        if gpu.particles.as_ref().is_none_or(|p| {
            p.instances < instances
                || p.draws < draw_capacity
                || p.colour_format != colour_format
                || p.depth_format != depth_format
        }) {
            gpu.particles = Some(ParticleGpu {
                pass: super::particle_pass::ParticlePass::new(
                    &device,
                    &queue,
                    instances,
                    draw_capacity,
                    colour_format,
                    depth_format,
                )?,
                instances,
                draws: draw_capacity,
                colour_format,
                depth_format,
                source: sources.clone(),
            });
        }
        let adaptation = buffers
            .get(&exposure.0)
            .ok_or("particle adaptation awaits preparation")?;
        let parameters = buffers
            .get(&shared.parameters)
            .ok_or("particle fog parameters await preparation")?;
        let image = |handle| {
            images
                .get(handle)
                .map(|image| &image.texture_view)
                .ok_or("particle fog image awaits preparation")
        };
        let inverse = view.clip_from_view.inverse();
        gpu.particles.as_mut().unwrap().pass.record(
            &device,
            &queue,
            context.command_encoder(),
            super::particle_pass::ParticleViewInputs {
                compressed_hdr: lens.settings.hdr_buffer_format != 1,
                white_level: lens.white_level,
                resolve_state: *sources
                    .transparency_resolve_state
                    .as_ref()
                    .map_err(Clone::clone)?,
                matrices,
                local_time: water.local_time[0],
                camera_exposure: camera.exposure,
                extent: [
                    target.main_texture().width(),
                    target.main_texture().height(),
                ],
                inverse_z: [inverse.z_axis.w, inverse.w_axis.w, inverse.w_axis.z],
                adaptation: &adaptation.buffer,
                colour: target.main_texture_view(),
                depth: &depth.alpha.full_attachment,
                fog: super::particle_pass::ParticleFogInputs {
                    parameters: &parameters.buffer,
                    world: image(&shared.world)?,
                    canvas: image(&shared.canvas)?,
                    curve: image(&shared.curve)?,
                    depth: image(&shared.opaque_depth)?,
                },
            },
            &draws,
        )?;
        if !*reported {
            info!(
                draws = draws.len(),
                instances = draws.iter().map(|d| d.instances.len()).sum::<usize>(),
                "Particle colour submission active"
            );
            *reported = true;
        }
        Ok(())
    })();
    match result {
        Ok(()) => *last_error = None,
        Err(error) if last_error.as_ref() != Some(&error) => {
            warn!("Particle colour pending: {error}");
            *last_error = Some(error);
        }
        Err(_) => {}
    }
}
/// The shared CPU controller advances before transparent colour submission.
/// Camera postprocessing consumes these commands without advancing it again.
fn prepare_screen_effects(
    sources: Res<super::screen_effects::ScreenEffectMaterials>,
    world_sources: Res<super::world_effects::WorldEffectSources>,
    world_graphs: Res<super::world_effects::WorldEffectGraphs>,
    world_frames: Res<super::world_effects::WorldEffectFrames>,
    water: Res<WaterSceneFrame>,
    fog: Res<FogGpu>,
    queries: Res<super::physics_queries::PhysicsQueryFrame>,
    mut gpu: ResMut<CameraGpu>,
    view: ViewQuery<&MainEntity>,
    mut last_error: Local<Option<String>>,
    mut particle_error: Local<Option<String>>,
    mut world_error: Local<Option<String>>,
) {
    let Some(water) = water.0.as_ref() else {
        gpu.screen_draws = None;
        gpu.screen_state = Default::default();
        gpu.particle_systems = Default::default();
        gpu.world_particles = Default::default();
        gpu.screen_sources = None;
        gpu.particles = None;
        return;
    };
    if view.into_inner().id() != water.camera {
        return;
    }
    gpu.screen_draws = None;
    if fog
        .camera_depth(water.camera, water.matrices.frame)
        .is_none()
    {
        return;
    }
    let Some(sources) = sources.0.as_ref() else {
        gpu.screen_state = Default::default();
        gpu.particle_systems = Default::default();
        gpu.world_particles = Default::default();
        gpu.screen_sources = None;
        gpu.particles = None;
        return;
    };
    if gpu
        .screen_sources
        .as_ref()
        .is_none_or(|previous| !std::sync::Arc::ptr_eq(previous, sources))
    {
        gpu.screen_state = Default::default();
        gpu.particle_systems = Default::default();
        gpu.world_particles = Default::default();
        gpu.particles = None;
        gpu.screen_sources = Some(sources.clone());
    }
    let underwater = fog
        .head_water_surface(water.camera, water.matrices.frame, water.head_water_surface)
        .is_some_and(|height| water.matrices.current.position[2] <= height);
    let camera_frame = crate::gameplay::reference_frame::ReferenceFrame {
        position: water.matrices.current.position,
        basis: [
            std::array::from_fn(|i| -water.matrices.current.view[2][i]),
            std::array::from_fn(|i| -water.matrices.current.view[0][i]),
            std::array::from_fn(|i| water.matrices.current.view[1][i]),
        ],
    };
    let CameraGpu { screen_state, particle_systems, .. } = &mut *gpu;
    let result = screen_state.advance(
        sources,
        particle_systems,
        water.matrices.frame,
        water.delta_seconds,
        underwater,
        camera_frame,
    );
    if let Some(inputs) = queries.0.as_ref().filter(|q| q.camera == water.camera && q.frame == water.matrices.frame) {
        let CameraGpu { world_particles, particle_systems, .. } = &mut *gpu;
        match world_particles.advance(&world_graphs,&world_sources.0,&world_frames,
            particle_systems,water.matrices.frame,camera_frame,underwater,&inputs.world) {
            Ok(()) => *world_error = None,
            Err(error) if world_error.as_ref() != Some(&error) => {
                warn!("World particle preparation pending: {error}");
                *world_error = Some(error);
            }
            Err(_) => {},
        }
    }
    match result {
        Ok(draws) => {
            let simulation = queries
                .0
                .as_ref()
                .filter(|q| q.camera == water.camera && q.frame == water.matrices.frame)
                .ok_or_else(|| "particle physics inputs await the current camera frame".to_owned())
                .and_then(|q| {
                    let ocean_height = fog
                        .ocean_capture(water.camera, water.matrices.frame)
                        .map(|ocean| {
                            ocean.prepared_water_level(
                                water.matrices.frame,
                                water.ocean_render_settings.skip,
                            )
                        })
                        .transpose()?
                        .flatten();
                    gpu.particle_systems.simulate(
                        water.matrices.frame,
                        &q.world,
                        ocean_height,
                    )
                });
            match simulation {
                Ok(()) => *particle_error = None,
                Err(error) if particle_error.as_ref() != Some(&error) => {
                    warn!("Player particle simulation pending: {error}");
                    *particle_error = Some(error);
                }
                Err(_) => {}
            }
            gpu.screen_draws = Some((water.camera, water.matrices.frame, draws));
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Player FX preparation pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
fn render_camera(
    screen_sources: Res<super::screen_effects::ScreenEffectMaterials>,
    display: Res<DisplaySceneFrame>,
    water: Res<WaterSceneFrame>,
    fog_frame: Res<FogSceneFrame>,
    fog: Res<FogGpu>,
    alpha: Res<super::alpha_depth_scene::AlphaGpu>,
    clear: Res<ClearColor>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    images: Res<RenderAssets<GpuImage>>,
    mut gpu: ResMut<CameraGpu>,
    mut context: RenderContext,
    view: ViewQuery<(
        &MainEntity,
        &ExtractedView,
        &ExtractedCamera,
        &ViewTarget,
        &MaterialExposure,
        &ExposureParameters,
    )>,
    mut last_error: Local<Option<String>>,
) {
    gpu.completed = None;
    let Some(frame) = display.0.as_ref() else {
        gpu.current = None;
        return;
    };
    let Some(lens) = frame.lens.as_ref() else {
        return;
    };
    if lens.settings.skip_post_process {
        return;
    }
    let (main, view, camera, target, exposure, exposure_parameters) = view.into_inner();
    if main.id() != frame.camera {
        return;
    }
    let result = (|| -> Result<(), String> {
        if !camera.hdr
            || target.main_texture().sample_count() != 1
            || camera.physical_viewport_size != camera.physical_target_size
            || camera
                .viewport
                .as_ref()
                .is_some_and(|v| v.physical_position != UVec2::ZERO)
        {
            return Err("camera effects require a resolved full gameplay view".into());
        }
        let full = [
            target.main_texture().width(),
            target.main_texture().height(),
        ];
        let water = water
            .0
            .as_ref()
            .filter(|w| w.camera == frame.camera)
            .ok_or("camera view time awaits preparation")?;
        let (depth, linear) = fog
            .camera_depth(frame.camera, water.matrices.frame)
            .ok_or("camera depth awaits completed atmosphere")?;
        let effects = frame
            .camera_effects
            .as_ref()
            .ok_or("camera settings await preparation")?;
        let resources = frame
            .camera_blur
            .as_deref()
            .ok_or("camera materials await preparation")?;
        let exposure = buffers
            .get(&exposure.0)
            .ok_or("camera adaptation awaits preparation")?;
        let black = images
            .get(&lens.black)
            .ok_or("camera default image awaits preparation")?;
        let config = CameraViewConfig {
            extent: full,
            reduced_extent: [depth.alpha.reduced.width(), depth.alpha.reduced.height()],
            host_format: target.main_texture_format(),
            depth_format: depth.alpha.reduced.format(),
            compressed: lens.settings.hdr_buffer_format != 1,
        };
        if !lens.settings.skip_depth_of_field
            && gpu
                .current
                .as_ref()
                .is_none_or(|(id, v)| *id != frame.camera || !v.matches(config))
        {
            gpu.current = Some((frame.camera, CameraView::new(&device, config, resources)?));
        }
        let inverse = view.clip_from_view.inverse();
        let scattering = fog_frame
            .0
            .as_ref()
            .filter(|f| f.camera == frame.camera)
            .ok_or("camera scattering parameters await preparation")?
            .parameters
            .scattering_color_intensity
            .truncate()
            .to_array();
        let write = (!lens.settings.skip_depth_of_field).then(|| target.post_process_write());
        if let Some(write) = &write {
            let result = gpu.current.as_mut().unwrap().1.record(
                &device,
                &queue,
                context.command_encoder(),
                CameraViewInputs {
                    effects,
                    resources,
                    colour: write.source,
                    depth: &depth.alpha.full_sample,
                    alpha_depth: &depth.alpha.reduced_sample,
                    alpha_attachment: &depth.alpha.reduced_attachment,
                    linear_water_depth: &linear.view,
                    linear_depth_extent: [linear.texture.width(), linear.texture.height()],
                    adaptation: &exposure.buffer,
                    black: &black.texture_view,
                    camera_exposure: camera.exposure,
                    adapt_max: exposure_parameters.range.y,
                    white_level: lens.white_level,
                    projection: view.clip_from_view.to_cols_array_2d(),
                    inverse_z: [inverse.z_axis.w, inverse.w_axis.w, inverse.w_axis.z],
                    local_time: water.local_time,
                    scattering_colour: scattering,
                    destination: write.destination,
                    alpha: alpha.opacity(frame.camera, water.matrices.frame).map(
                        |(opacity, special_alpha)| CameraAlphaViewInputs {
                            refraction: opacity,
                            full_depth_attachment: &depth.alpha.full_attachment,
                            reduction_shift: lens.settings.downsample as u32,
                            clear_colour: match camera.clear_color {
                                bevy::camera::ClearColorConfig::Custom(c) => {
                                    c.to_linear().to_f32_array()[..3].try_into().unwrap()
                                }
                                _ => clear.0.to_linear().to_f32_array()[..3].try_into().unwrap(),
                            },
                            special_alpha,
                        },
                    ),
                },
            );
            if result.is_err() {
                //Recording failed before combine. Restore the readable HDR selector;
                //partial scratch writes must never become the displayed frame.
                let _ = target.post_process_write();
            }
            result?;
        } else {
            gpu.current = None;
        }
        let colour = write
            .as_ref()
            .map_or_else(|| target.main_texture_view(), |w| w.destination);
        if let Some(sources) = screen_sources.0.as_deref() {
            let CameraGpu {
                screen_draws,
                screen_effects,
                ..
            } = &mut *gpu;
            let (_, _, draws) = screen_draws
                .as_ref()
                .filter(|(camera, marker, _)| {
                    *camera == frame.camera && *marker == water.matrices.frame
                })
                .ok_or("player FX commands await current atmosphere")?;
            screen_effects.record(
                &device,
                &queue,
                context.command_encoder(),
                sources,
                draws,
                water.local_time[0],
                colour,
                &depth.alpha.full_sample,
                full,
                lens.settings.downsample as u32,
                config.compressed,
                config.host_format,
                camera.exposure,
                lens.white_level,
                &exposure.buffer,
                [inverse.z_axis.w, inverse.w_axis.w, inverse.w_axis.z],
            )?;
        }
        gpu.completed = Some((frame.camera, water.matrices.frame));
        Ok(())
    })();
    match result {
        Ok(()) => {
            *last_error = None;
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) {
                warn!("Camera effects submission pending: {error}");
                *last_error = Some(error);
            }
        }
    }
}
