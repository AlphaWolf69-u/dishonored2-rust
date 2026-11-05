//! Refraction blur precedes transparent colour; final camera effects follow it.
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
    ecs::system::SystemParam,
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

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct CameraAlphaPass;

/// CPU simulation shares one particle pool across player and world effects.
/// Its ownership can move independently of render-only GPU state.
#[derive(Resource)]
pub(crate) struct FxSimulation {
    generation: u64,
    screen_state: super::screen_effects::ScreenEffectState,
    particle_systems: super::particle_systems::ParticleSystems,
    pub(super) world_particles: super::world_particle_state::WorldParticleState,
    screen_sources: Option<std::sync::Arc<super::screen_effects::ScreenEffectSources>>,
}
impl Default for FxSimulation {
    fn default() -> Self {
        static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            generation: NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            screen_state: Default::default(), particle_systems: Default::default(),
            world_particles: Default::default(), screen_sources: None,
        }
    }
}

#[derive(Resource, Default)]
pub(super) struct CameraGpu {
    current: Option<(Entity, CameraView)>,
    completed: Option<(Entity, u32)>,
    alpha_completed: Option<(Entity, u32)>,
    screen_effects: super::screen_effects::ScreenEffects,
    particles: Option<ParticleGpu>,
    particle_images: super::particle_images::ParticleImagesGpu,
    particle_programs: std::collections::HashMap<(bevy::render::render_resource::TextureFormat, bevy::render::render_resource::TextureFormat), std::sync::Arc<super::particle_pass::ParticlePrograms>>,
}
pub(super) struct ParticlePublicationFrame {
    pub camera: Entity,
    pub frame: u32,
    pub draws: Vec<super::particle_pass::ParticleDraw>,
    pub world_meshes: Vec<super::particle_system::WorldMeshDraw>,
}

/// One immutable gameplay publication is shared by the rendering consumers.
/// Particle state remains exclusively in the gameplay world.
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub(crate) struct FxPublication(pub(super) Option<std::sync::Arc<FxFrame>>);
pub(super) struct FxFrame {
    pub camera: Entity,
    pub frame: u32,
    pub generation: u64,
    pub capacity: (usize, usize),
    pub retired: u64,
    pub screen_draws: Option<Vec<super::screen_effects::QuadDraw>>,
    pub particles: Option<ParticlePublicationFrame>,
    pub decals: Vec<(u32, u64, crate::rendering::decals::CommittedDecal<super::decal_effects::DecalReferences>)>,
}

/// 1ffff0 publishes every visible system's renderers after its simulation;
/// all of this frame's consumers read the same publication.
fn publish_particles(
    systems: &mut super::particle_systems::ParticleSystems,
    water: &super::water_scene::WaterFrame,
    queries: &super::particle_systems::ParticleQueryFrame,
    fog_frame: &FogSceneFrame,
    image_sources: &super::particle_images::SceneParticleImages,
) -> Result<ParticlePublicationFrame, String> {
    let frame = fog_frame
        .0
        .as_ref()
        .filter(|f| f.camera == water.camera)
        .ok_or("particle fog awaits the gameplay camera")?;
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
        let transparency = crate::rendering::particles::transparency::ParticleTransparency::new(&console.cvars);
        let (draws, world_meshes) = systems
            .publish_draws(water.matrices.frame, |pose| {
                Ok(super::screen_effects::ParticleDrawContext {
                    view: &sprite_view,
                    probes: queries.0.as_ref()
                        .filter(|q| q.camera == water.camera && q.frame == water.matrices.frame)
                        .ok_or("particle lighting awaits the current scene publication")?
                        .probes.as_deref(),
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
                    transparency: &transparency,
                    areas: &frame.areas,
                    world_images: &image_sources.world_materials,
                })
            })?;
        drop(console);
    Ok(ParticlePublicationFrame {
        camera: water.camera,
        frame: water.matrices.frame,
        draws,
        world_meshes,
    })
}

struct ParticleGpu {
    pass: super::particle_pass::ParticlePass,
    instances: usize,
    draws: usize,
    colour_format: bevy::render::render_resource::TextureFormat,
    depth_format: bevy::render::render_resource::TextureFormat,
    source: std::sync::Arc<super::screen_effects::ScreenEffectSources>,
    /// Every level particle material has its GPU material (`PARTICLE_GATE`).
    prepared: bool,
    retirement: u64,
}

/// Level loading gate: the level's particle GPU materials exist.
pub const PARTICLE_GATE: &str = "particle materials";

pub(super) fn release_level(world: &mut World) {
    let mut gpu = world.resource_mut::<CameraGpu>();
    gpu.current = None;
    gpu.completed = None;
    gpu.alpha_completed = None;
    if let Some(particles) = gpu.particles.take() {
        gpu.particle_programs.insert((particles.colour_format, particles.depth_format), particles.pass.reusable_programs());
    }
    gpu.particle_images = Default::default();
    if std::env::var("DH2_DEBUG_RETAIN_WORLD_BINDINGS").as_deref() == Ok("1") {
        gpu.screen_effects.clear_frame();
    } else {
        gpu.screen_effects.release_level();
    }
}
impl CameraGpu {
    pub(super) fn is_complete(&self, camera: Entity, frame: u32) -> bool {
        self.completed == Some((camera, frame))
    }
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
        self.is_complete(camera, frame)
            .then(|| self.current.as_ref().map(|(_, v)| v.half_colour()))
            .flatten()
    }
}
pub(super) fn register(app: &mut App) {
    app.init_resource::<FxSimulation>().init_resource::<FxPublication>();
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<FxPublication>::default());
    app.init_resource::<super::particle_pass::ParticleMaterialSampler>();
    app.init_resource::<super::particle_images::SceneParticleImages>()
        .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<super::particle_images::SceneParticleImages>::default());
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<super::particle_pass::ParticleMaterialSampler>::default());
    app.init_resource::<super::particle_systems::ParticleQueryFrame>();
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::particle_systems::ParticleQueryFrame,
    >::default());
    app.init_resource::<super::physics_queries::PhysicsQueryFrame>();
    crate::loading::gate(PARTICLE_GATE);
    app.init_resource::<super::screen_effects::ScreenEffectMaterials>();
    app.init_resource::<super::world_effects::WorldEffectSources>();
    app.init_resource::<super::world_effects::WorldEffectDefinitions>();
    app.init_resource::<super::world_effects::WorldEffectFrames>();
    app.init_resource::<super::world_effects::DynamicEffects>();
    app.init_resource::<super::world_effects::DynamicMaterialPrewarm>();
    app.init_resource::<super::world_sounds::WorldSounds>();
    // Opt-in comparison with the former extraction only. The gameplay owner
    // remains the sole simulator in either mode.
    if std::env::var("DH2_DEBUG_EFFECT_FRAME_EXTRACTION").as_deref() == Ok("1") {
        app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            super::world_effects::WorldEffectFrames,
        >::default());
    }
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::world_effects::WorldEffectSources,
    >::default());
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::world_effects::DynamicMaterialPrewarm,
    >::default());
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::screen_effects::ScreenEffectMaterials,
    >::default());
    app.add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
        super::particle_meshes::ParticleMeshProxy,
    >::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.init_resource::<CameraGpu>().add_systems(
            Core3d,
            render_camera::<false>
                .in_set(Core3dSystems::PostProcess)
                .in_set(CameraPass)
                .before(ExposurePass),
        );
        // 351780: DOF_ALPHA produces the background consumed by ALPHA_LIT;
        // the final DOF task in turn depends on ALPHA_LIT.
        render.add_systems(Core3d, render_camera::<true>
            .in_set(Core3dSystems::MainPass)
            .in_set(CameraAlphaPass)
            .after(super::fog_scene::AtmospherePass)
            .before(render_particles)
            .before(bevy::core_pipeline::core_3d::main_transparent_pass_3d));
        render.add_systems(
            Core3d,
            render_particles
                .in_set(Core3dSystems::MainPass)
                .after(super::fog_scene::AtmospherePass)
                .before(bevy::core_pipeline::core_3d::main_transparent_pass_3d),
        );
        // FX simulation and publication precede every pass that draws their
        // surfaces: world-material particle meshes enter the Z prepass.
        render.add_systems(
            Core3d,
            prepare_ocean_queries
                .in_set(FxUpdate)
                .before(Core3dSystems::Prepass)
                .before(render_particles),
        );
        render
            .init_resource::<bevy::pbr::DirectInstanceDraws>()
            .init_resource::<super::particle_meshes::ParticleMeshBuffers>()
            .add_systems(
                Core3d,
                super::particle_meshes::prepare_particle_meshes
                    .after(FxUpdate)
                    .before(Core3dSystems::Prepass),
            );
    }
}

/// The per-frame FX update (simulation and draw publication).
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FxUpdate;

/// Canonical lighting owners borrowed by the shared particle colour pass.
#[derive(SystemParam)]
struct ParticleLightingScene<'w> {
    scene_images: Res<'w, super::particle_images::SceneParticleImages>,
    upload_budget: Res<'w, bevy::render::render_asset::RenderAssetBytesPerFrameLimiter>,
    screen: Option<Res<'w, super::screen_shadow_pass::ScreenShadowFrame>>,
    sampler: Option<Res<'w,super::particle_pass::ParticleMaterialSampler>>,
    queries: Res<'w,super::particle_systems::ParticleQueryFrame>,
    probes: Res<'w,super::water_scene::WaterLighting>,
    models: Res<'w,super::water_scene::WaterModelLighting>,
    parallel: Option<Res<'w,super::shadow_sampling::ParallelSamplingBuffer>>,
    local: Option<Res<'w,super::shadow_sampling::LocalSamplingBuffer>>,
    directional_frame: Res<'w,super::shadow_pass::ShadowSceneFrame>,
    directional_gpu: Res<'w,super::shadow_pass::ShadowSceneGpu>,
    local_frame: Option<Res<'w,super::shadow_pass::LocalShadowSceneFrame>>,
    local_completion: Option<Res<'w,super::shadow_pass::LocalShadowCompletion>>,
    lights: Res<'w,super::fog_light_scattering::LocalFogFrame>,
    world_sources: Res<'w,super::world_effects::WorldEffectSources>,
    prewarm: Res<'w,super::world_effects::DynamicMaterialPrewarm>,
}
/// Submit live sprites after atmosphere/depth and before ordinary transparency.
fn render_particles(
    lighting_scene: ParticleLightingScene,
    publication: Res<FxPublication>,
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
    let _loading_span = bevy::render::diagnostic::LoadingSpan::new("particle rendering");
    let queries=&lighting_scene.queries;
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
        let (mut instances, mut draw_capacity) = publication.0.as_ref()
            .filter(|frame| frame.camera == water.camera && frame.frame == water.matrices.frame)
            .map_or((0, 0), |frame| frame.capacity);
        let mut prewarm_only = false;
        if instances == 0 || draw_capacity == 0 {
            // A runtime request may be the level's first particle system.
            // Allocate a minimal pass for its material prewarm rather than
            // waiting for playback, which itself waits for that prewarm.
            crate::loading::complete(PARTICLE_GATE);
            let prewarm = &lighting_scene.prewarm;
            let CameraGpu { particles, particle_images, .. } = &mut *gpu;
            if let Some(particles) = particles.as_mut() {
                if particles.retirement != prewarm.retirement {
                    particles.pass.retire_unused_materials();
                    particle_images.retire_unused();
                    particles.retirement = prewarm.retirement;
                    prewarm.retired_signal.fetch_add(1, std::sync::atomic::Ordering::Release);
                }
            }
            use std::sync::atomic::Ordering;
            if prewarm.generation == 0
                || prewarm.completed.load(Ordering::Acquire) == prewarm.generation
                || prewarm.failed.load(Ordering::Acquire) == prewarm.generation { return Ok(()); }
            instances = instances.max(1);
            draw_capacity = draw_capacity.max(1);
            prewarm_only = true;
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
        let frame_publication = publication.0.as_ref()
            .filter(|frame| frame.camera == water.camera && frame.frame == water.matrices.frame)
            .ok_or("particle draws await the FX update")?;
        let draws = &frame_publication.particles.as_ref()
            .ok_or("particle draws await the FX update")?.draws;
        let lens = display
            .0
            .as_ref()
            .filter(|f| f.camera == water.camera)
            .and_then(|f| f.lens.as_ref())
            .ok_or("particle HDR settings await the current view")?;
        let colour_format = target.main_texture_format();
        let depth_format = depth.alpha.full.format();
        // A larger live capacity only grows the instance/draw storage; the
        // pass and its level materials are rebuilt only for new targets.
        if let Some(particles) = gpu.particles.as_mut().filter(|p| {
            p.colour_format == colour_format && p.depth_format == depth_format
                && (p.instances < instances || p.draws < draw_capacity)
        }) {
            particles.pass.grow(&device, instances, draw_capacity);
            particles.instances = particles.instances.max(instances);
            particles.draws = particles.draws.max(draw_capacity);
        }
        if gpu.particles.as_ref().is_none_or(|p| {
            p.instances < instances
                || p.draws < draw_capacity
                || p.colour_format != colour_format
                || p.depth_format != depth_format
        }) {
            if let Some(particles) = gpu.particles.take() {
                gpu.particle_programs.insert((particles.colour_format, particles.depth_format), particles.pass.reusable_programs());
            }
            let cached = gpu.particle_programs.remove(&(colour_format, depth_format));
            let cached = if std::env::var("DH2_DEBUG_RECREATE_PARTICLE_PROGRAMS").as_deref() == Ok("1") {None} else {cached};
            let pass = super::particle_pass::ParticlePass::new_with_programs(
                &device, &queue, instances, draw_capacity, colour_format, depth_format, cached,
            )?;
            gpu.particles = Some(ParticleGpu {
                pass,
                instances,
                draws: draw_capacity,
                colour_format,
                depth_format,
                source: sources.clone(),
                prepared: prewarm_only,
                retirement: lighting_scene.prewarm.retirement,
            });
            info!(instances, draws=draw_capacity, "Particle colour resources prepared");
        }
        // The level's particle materials are created before play, not at
        // their first draw; a sampler change rebuilds them all here.
        if let Some(sampler) = lighting_scene.sampler.as_ref().and_then(|sampler| images.get(&sampler.0)).map(|image| &image.sampler) {
            let CameraGpu { particles, particle_images, .. } = &mut *gpu;
            let particles = particles.as_mut().unwrap();
            if particle_images.sync_scene(&lighting_scene.scene_images, &images,
                particles.pass.sampler_changed(sampler))? {
                particles.pass.refresh_scene_bindings(particle_images, &device, &queue, sampler)?;
            }
            if !particles.prepared || particles.pass.sampler_changed(sampler) {
                let console = crate::platform::console::shared().read();
                let materials = lighting_scene.world_sources.0.particle_materials().chain(sources.particle_materials());
                let bounded = !particles.prepared && lighting_scene.upload_budget.max_bytes.is_some()
                    && !bevy::render::render_asset::unbounded_loading_acquisition();
                let (created, complete) = particles.pass.prepare_materials_bounded(
                    particle_images, &device, &queue, materials, &console.cvars, sampler, bounded.then_some(1))?;
                if !complete {
                    return Err("particle materials are preparing".into());
                }
                if !particles.prepared {
                    info!(created, "Level particle materials prepared");
                    crate::loading::complete(PARTICLE_GATE);
                }
                particles.prepared = true;
            }
            let prewarm = &lighting_scene.prewarm;
            if particles.retirement != prewarm.retirement {
                particles.pass.retire_unused_materials();
                particle_images.retire_unused();
                particles.retirement = prewarm.retirement;
                prewarm.retired_signal.fetch_add(1, std::sync::atomic::Ordering::Release);
            }
            use std::sync::atomic::Ordering;
            if prewarm.generation != 0
                && prewarm.completed.load(Ordering::Acquire) != prewarm.generation
                && prewarm.failed.load(Ordering::Acquire) != prewarm.generation
                && prewarm.sources.particle_materials().flat_map(|material| material.images.values())
                    .filter(|image| image.scene_owned)
                    .all(|image| lighting_scene.scene_images.handles.contains_key(&image.path)) {
                let console = crate::platform::console::shared().read();
                let started = super::world_effects::trace_lifecycle().then(std::time::Instant::now);
                match particles.pass.prepare_materials_bounded(particle_images, &device, &queue,
                    prewarm.sources.particle_materials(), &console.cvars, sampler, Some(1)) {
                    Ok((created, complete)) => {
                        if let Some(started) = started {
                            eprintln!("FX_LIFECYCLE gpu_prepare generation={} created={created} complete={complete} cpu_ms={:.3}",
                                prewarm.generation, started.elapsed().as_secs_f64() * 1000.0);
                        }
                        if complete { prewarm.completed.store(prewarm.generation, Ordering::Release); }
                    }
                    Err(error) if error.contains("not scene-resident") => {},
                    Err(error) => {
                        warn!("Dynamic particle material unavailable: {error}");
                        prewarm.failed.store(prewarm.generation, Ordering::Release);
                    }
                }
            }
        }
        // Prepare the shared particle pipeline once the live pool exists,
        // independently of whether this frame sees any of its sprites.
        if draws.is_empty() {
            return Ok(());
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
        let CameraGpu { particles, particle_images, .. } = &mut *gpu;
        particles.as_mut().unwrap().pass.record(
            particle_images, &device,
            &queue,
            context.command_encoder(),
            super::particle_pass::ParticleViewInputs {
                distortion_sources: sources,
                surface_sampler: lighting_scene.sampler.as_ref().and_then(|sampler|
                    images.get(&sampler.0)).map(|image| &image.sampler),
                direct: if draws.iter().any(|d| !d.material.distortion && d.material.option("fx/opt/use_lighting").is_ok_and(|v| v==1 || v>=3)) {
                    let tiled=fog.tiled_lighting(water.camera,water.matrices.frame)
                        .ok_or("particle tiled lighting awaits the current view")?;
                    let sampling=lighting_scene.parallel.as_ref().ok_or("particle directional sampling owner unavailable")?;
                    let directional_image=match &lighting_scene.directional_frame.state {
                        super::shadow_pass::ShadowSceneState::Ready {target_image,..} => {
                            if !lighting_scene.directional_gpu.completed_revision(lighting_scene.directional_frame.revision) {
                                return Err("particle directional shadows await current atlas submission".into());
                            }
                            target_image
                        },
                        super::shadow_pass::ShadowSceneState::Disabled => &sampling.unavailable_depth,
                        _ => return Err("particle directional atlas owner is not ready".into()),
                    };
                    let local=if lighting_scene.lights.lights.iter().any(|l| l[24]!=u32::MAX) {
                        let owner=lighting_scene.local_frame.as_ref().and_then(|v| v.0.as_ref())
                            .ok_or("particle local shadow owner unavailable")?;
                        if lighting_scene.lights.shadow_revision!=Some(owner.revision) {
                            return Err("particle local shadows belong to another frame".into());
                        }
                        let ready=lighting_scene.local_completion.as_ref().and_then(|v| v.0.lock().ok()).is_some_and(|v|
                            v.as_ref().is_some_and(|(revision,atlas,cache,_)|
                                *revision==owner.revision && *atlas==owner.atlas.id() && *cache==owner.cache.id()));
                        if !ready {return Err("particle local shadows await atlas submission".into());}
                        let sampling=lighting_scene.local.as_ref().ok_or("particle local sampling owner unavailable")?;
                        Some((&buffers.get(&sampling.buffer).ok_or("particle local sampling buffer unavailable")?.buffer,
                            &images.get(&owner.atlas).ok_or("particle local atlas unavailable")?.texture_view))
                    } else {None};
                    Some(super::particle_pass::ParticleDirectLightingInputs {
                        pixel: if draws.iter().any(|draw| draw.material.lighting.as_ref().is_some_and(|light| light.pixel.is_some())) {
                            let screen = lighting_scene.screen.as_ref().ok_or("particle cloud owner unavailable")?;
                            let cloud = images.get(&screen.cloud).ok_or("particle cloud image awaits GPU preparation")?;
                            if lighting_scene.lights.area_images.iter().any(Option::is_some) && tiled.area_pool.is_none() {
                                return Err("particle area-light images await the shared pool".into());
                            }
                            Some(super::particle_pass::ParticlePixelLightingInputs {
                                screen: &images.get(&screen.output).ok_or("particle screen binding awaits GPU preparation")?.texture_view,
                                cloud: &cloud.texture_view, cloud_sampler: &cloud.sampler,
                                area: tiled.area_pool.as_ref(),
                            })
                        } else { None },
                        lights:&tiled.lights,pool:&tiled.buffers.pool,heads:&tiled.normal_heads,shadows:&tiled.shadows,
                        parallel:&buffers.get(&sampling.buffer).ok_or("particle parallel sampling buffer unavailable")?.buffer,
                        parallel_depth:&images.get(directional_image).ok_or("particle directional atlas unavailable")?.texture_view,
                        local,grid:tiled.grid(),sun:water.parallel,
                    })
                } else {None},
                lighting: if draws.iter().any(|d| d.lighting.is_some()) {
                    let resources=lighting_scene.probes.0.as_ref().map_err(Clone::clone)?
                        .as_ref().ok_or("particle shared probe resources absent")?;
                    if resources.cube_count==0 { return Err("particle reflection cube array is empty".into()); }
                    Some(super::particle_pass::ParticleLightingInputs {
                        probes:&buffers.get(&lighting_scene.models.records)
                            .ok_or("particle primary SH buffer unavailable")?.buffer,
                        words:&buffers.get(&resources.vertex_references)
                            .ok_or("particle tetrahedron buffer unavailable")?.buffer,
                        planes:&buffers.get(&resources.planes)
                            .ok_or("particle reflection planes unavailable")?.buffer,
                        cubes:&images.get(&resources.cubes)
                            .ok_or("particle reflection cube array unavailable")?.texture_view,
                        sampler:&images.get(&resources.cubes)
                            .ok_or("particle reflection sampler unavailable")?.sampler,
                    })
                } else {None},
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
            draws,
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
/// World FX inputs and the pre-pass water state read by the FX update.
#[derive(SystemParam)]
pub(crate) struct FxUpdateInputs<'w> {
    scene_images: Res<'w, super::particle_images::SceneParticleImages>,
    world_sources: Res<'w, super::world_effects::WorldEffectSources>,
    world_graphs: Res<'w, super::world_effects::WorldEffectDefinitions>,
    world_frames: Res<'w, super::world_effects::WorldEffectFrames>,
    dynamic: Res<'w, super::world_effects::DynamicEffects>,
    world_sounds: Res<'w, super::world_sounds::WorldSounds>,
    decals: Res<'w, super::projected_decals::DecalScene>,
    swimming_surface: Res<'w, super::water_scene::SwimmingSurfaceMailbox>,
}

/// Gameplay owns the sole world/player FX simulation and particle pool. The
/// render world receives only this frame's immutable draws and decal records.
pub(crate) fn simulate_effects(
    sources: Res<super::screen_effects::ScreenEffectMaterials>,
    fx: FxUpdateInputs,
    water: Res<WaterSceneFrame>,
    queries: Res<super::physics_queries::PhysicsQueryFrame>,
    particle_view: Res<super::particle_systems::ParticleQueryFrame>,
    fog_frame: Res<FogSceneFrame>,
    mut simulation: ResMut<FxSimulation>,
    mut published: ResMut<FxPublication>,
    mut last_error: Local<Option<String>>,
    mut particle_error: Local<Option<String>>,
    mut world_error: Local<Option<String>>,
    mut decal_error: Local<Option<String>>,
) {
    let Some(water) = water.0.as_ref() else {
        if simulation.screen_sources.is_some() { *simulation = FxSimulation::default(); }
        published.0 = None;
        return;
    };
    let Some(sources) = sources.0.as_ref() else {
        if simulation.screen_sources.is_some() { *simulation = FxSimulation::default(); }
        published.0 = None;
        return;
    };
    if simulation.screen_sources.as_ref().is_none_or(|previous| !std::sync::Arc::ptr_eq(previous, sources)) {
        *simulation = FxSimulation::default();
        simulation.screen_sources = Some(sources.clone());
    }
    let trace = super::world_effects::trace_lifecycle();
    let started = trace.then(std::time::Instant::now);
    let cycles = if trace { crate::loading::thread_cycles() } else { None };
    let (surface, prepared_ocean) = fx.swimming_surface.fx_inputs();
    let surface = surface.filter(|surface| surface.plane == water.head_water_surface);
    let head = surface.and_then(|surface| surface.head).or(water.head_water_surface);
    let ocean_height = if water.ocean_render_settings.skip { None } else {
        prepared_ocean.map(|view| view.level)
    };
    let underwater = head.is_some_and(|height| water.matrices.current.position[2] <= height);
    let camera_frame = crate::gameplay::reference_frame::ReferenceFrame {
        position: water.matrices.current.position,
        basis: [
            std::array::from_fn(|i| -water.matrices.current.view[2][i]),
            std::array::from_fn(|i| -water.matrices.current.view[0][i]),
            std::array::from_fn(|i| water.matrices.current.view[1][i]),
        ],
    };
    let FxSimulation { screen_state, particle_systems, .. } = &mut *simulation;
    let screen = screen_state.advance(sources, particle_systems, water.matrices.frame,
        water.delta_seconds, underwater, camera_frame);
    if let Some(inputs) = queries.0.as_ref().filter(|q| q.camera == water.camera && q.frame == water.matrices.frame) {
        let FxSimulation { world_particles, particle_systems, .. } = &mut *simulation;
        match world_particles.advance(&fx.world_graphs, &fx.world_sources.0, &fx.dynamic, &fx.world_frames,
            particle_systems, &fx.world_sounds, &fx.decals, water.matrices.frame, camera_frame, underwater, &inputs.world) {
            Ok(()) => *world_error = None,
            Err(error) if world_error.as_ref() != Some(&error) => {
                warn!("World particle preparation pending: {error}");
                *world_error = Some(error);
            }
            Err(_) => {}
        }
    }
    let (screen_draws, particles) = match screen {
        Ok(draws) => {
            let result = queries.0.as_ref()
                .filter(|q| q.camera == water.camera && q.frame == water.matrices.frame)
                .ok_or_else(|| "particle physics inputs await the current camera frame".to_owned())
                .and_then(|q| {
                    let view = particle_view.0.as_ref()
                        .filter(|view| view.camera == water.camera && view.frame == water.matrices.frame)
                        .ok_or("particle visibility awaits the current camera frame")?;
                    simulation.particle_systems.simulate(water.matrices.frame, &q.world, ocean_height,
                        &water.matrices.current, view.state.as_ref(), &view.wind)
                });
            let publication = publish_particles(&mut simulation.particle_systems, water,
                &particle_view, &fog_frame, &fx.scene_images);
            let (particles, publication_error) = match publication {
                Ok(publication) => (Some(publication), None),
                Err(error) => (None, Some(error)),
            };
            let error = result.err().or(publication_error);
            if let Some(error) = error {
                if particle_error.as_ref() != Some(&error) { warn!("Particle simulation pending: {error}"); }
                *particle_error = Some(error);
            } else { *particle_error = None; }
            *last_error = None;
            (Some(draws), particles)
        }
        Err(error) => {
            if last_error.as_ref() != Some(&error) { warn!("Player FX preparation pending: {error}"); }
            *last_error = Some(error);
            (None, None)
        }
    };
    let decals = match fx.decals.tree.as_deref() {
        None => Vec::new(),
        Some(tree) => {
            let limit = crate::platform::console::shared().read().cvars.int("r_maxDynamicDecals");
            match limit.ok_or_else(|| "decal budget absent".to_owned()).and_then(|limit|
                simulation.world_particles.snapshot_decals(tree, limit as usize)) {
                Ok(decals) => { *decal_error = None; decals }
                Err(error) => {
                    if decal_error.as_ref() != Some(&error) { warn!("Projected decals pending: {error}"); }
                    *decal_error = Some(error);
                    Vec::new()
                }
            }
        }
    };
    published.0 = Some(std::sync::Arc::new(FxFrame {
        camera: water.camera, frame: water.matrices.frame, generation: simulation.generation,
        capacity: simulation.particle_systems.capacity(), retired: simulation.world_particles.dynamic_retired(),
        screen_draws, particles, decals,
    }));
    if let Some(started) = started {
        let cpu_cycles = cycles.zip(crate::loading::thread_cycles())
            .map(|(start, end)| end.saturating_sub(start));
        eprintln!("FX_LIFECYCLE gameplay_simulation frame={} elapsed_ms={:.3} cpu_cycles={cpu_cycles:?} pose_signature={:016x}",
            water.matrices.frame, started.elapsed().as_secs_f64() * 1000.0, fx.world_frames.debug_pose_signature());
    }
}

fn prepare_ocean_queries(
    water: Res<WaterSceneFrame>,
    mut fog: ResMut<FogGpu>,
    mailbox: Res<super::water_scene::SwimmingSurfaceMailbox>,
    publication: Res<FxPublication>,
    prewarm: Res<super::world_effects::DynamicMaterialPrewarm>,
    mut gpu: ResMut<CameraGpu>,
    view: ViewQuery<&MainEntity>,
    mut ocean_error: Local<Option<String>>,
    mut swimming_owner: Local<Option<(usize, Entity, Option<f32>, bool, bool)>>,
    mut retirement: Local<Option<(u64, u64)>>,
) {
    let Some(water) = water.0.as_ref() else {
        if swimming_owner.take().is_some() { mailbox.publish(None); }
        *retirement = None;
        gpu.particles = None;
        return;
    };
    if view.into_inner().id() != water.camera { return; }
    let owner = (mailbox.identity(), water.camera, water.head_water_surface,
        water.ocean_render_settings.skip, water.ocean_environment.view.enabled);
    if *swimming_owner != Some(owner) {
        mailbox.publish(None);
        *swimming_owner = Some(owner);
    }
    // Register/consume the existing render query after gameplay simulation;
    // depth preparation publishes the newly completed CPU ring for next update.
    match fog.simulation_ocean_inputs(water) {
        Ok(_) => *ocean_error = None,
        Err(error) if ocean_error.as_ref() != Some(&error) => {
            warn!("Ocean query preparation pending: {error}");
            *ocean_error = Some(error);
        }
        Err(_) => {}
    }
    let Some(frame) = publication.0.as_ref()
        .filter(|frame| frame.camera == water.camera && frame.frame == water.matrices.frame) else {
        return;
    };
    let retired = retirement.as_ref().map_or(frame.retired != 0, |(generation, count)| {
        if *generation == frame.generation { *count != frame.retired }
        else { frame.retired != 0 }
    });
    if retired {
        let CameraGpu { particles, particle_images, .. } = &mut *gpu;
        if let Some(particles) = particles.as_mut() { particles.pass.retire_unused_materials(); }
        particle_images.retire_unused();
        prewarm.retired_signal.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
    *retirement = Some((frame.generation, frame.retired));
}
fn render_camera<const PREPARE_ALPHA: bool>(
    screen_sources: Res<super::screen_effects::ScreenEffectMaterials>,
    publication: Res<FxPublication>,
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
    let _loading_span = bevy::render::diagnostic::LoadingSpan::new(if PREPARE_ALPHA { "camera alpha preparation" } else { "camera composition" });
    gpu.completed = None;
    if PREPARE_ALPHA {gpu.alpha_completed = None;}
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
        if !PREPARE_ALPHA && !lens.settings.skip_depth_of_field
            && gpu.alpha_completed != Some((frame.camera, water.matrices.frame)) {
            return Err("camera colour awaits current transparent blur preparation".into());
        }
        let write = (!PREPARE_ALPHA && !lens.settings.skip_depth_of_field).then(|| target.post_process_write());
        if !lens.settings.skip_depth_of_field {
            let source = write.as_ref().map_or_else(|| target.main_texture_view(), |w| w.source);
            let destination = write.as_ref().map_or(source, |w| w.destination);
            let record = if PREPARE_ALPHA {CameraView::record_alpha} else {CameraView::record_after_alpha};
            let result = record(&mut gpu.current.as_mut().unwrap().1,
                &device,
                &queue,
                context.command_encoder(),
                CameraViewInputs {
                    effects,
                    resources,
                    colour: source,
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
                    destination,
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
            if result.is_err() && write.is_some() {
                //Recording failed before combine. Restore the readable HDR selector;
                //partial scratch writes must never become the displayed frame.
                let _ = target.post_process_write();
            }
            result?;
        } else {
            gpu.current = None;
        }
        if PREPARE_ALPHA {
            gpu.alpha_completed = Some((frame.camera, water.matrices.frame));
            return Ok(());
        }
        let colour = write
            .as_ref()
            .map_or_else(|| target.main_texture_view(), |w| w.destination);
        if let Some(sources) = screen_sources.0.as_deref() {
            let draws = publication.0
                .as_ref()
                .filter(|published| published.camera == frame.camera && published.frame == water.matrices.frame)
                .and_then(|published| published.screen_draws.as_ref())
                .ok_or("player FX commands await current atmosphere")?;
            gpu.screen_effects.record(
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
