//! Actual scene-worker resource acquisition without a window or game frames.
//! No application runner/update is invoked. Optional render epochs upload and
//! retire real assets with cameras disabled; gameplay and audio never run.
use super::*;
use bevy::{ecs::system::RunSystemOnce, prelude::*, render::{
    mesh::RenderMesh, render_asset::RenderAssets, renderer::RenderQueue,
    storage::GpuShaderBuffer, texture::GpuImage, RenderApp,
}};

pub fn inspect_scene_preparation(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    inspect_scene_resources(base, levels, Mode::Prepare)
}

pub fn inspect_scene_publication(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    inspect_scene_resources(base, levels, Mode::Publish)
}

pub fn inspect_scene_gpu_resources(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    inspect_scene_resources(base, levels, Mode::Upload)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode { Prepare, Publish, Upload }

fn gpu_inputs(world: &World) -> super::scene_preparation::SceneGpuInputs {
    super::scene_preparation::SceneGpuInputs {
        images: std::sync::Arc::new(world.resource::<Assets<Image>>().iter()
            .filter_map(|(id, image)| image.asset_usage.contains(RenderAssetUsages::RENDER_WORLD).then_some(id)).collect()),
        meshes: std::sync::Arc::new(world.resource::<Assets<Mesh>>().iter()
            .filter_map(|(id, mesh)| mesh.asset_usage.contains(RenderAssetUsages::RENDER_WORLD).then_some(id)).collect()),
        buffers: std::sync::Arc::new(world.resource::<Assets<ShaderBuffer>>().iter()
            .filter_map(|(id, buffer)| buffer.asset_usage.contains(RenderAssetUsages::RENDER_WORLD).then_some(id)).collect()),
    }
}

fn render_epoch(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    // First drains the render-time channel and maintains event buffers.
    // Pre/PostUpdate run only host asset/transform maintenance: this host has
    // no gameplay, sound playback or window systems installed.
    app.world_mut().run_schedule(First);
    app.world_mut().run_schedule(PreUpdate);
    app.world_mut().run_schedule(PostUpdate);
    app.update_sub_app_by_label(RenderApp);
    let submitted = app.world().resource::<RenderQueue>().submit([]);
    app.world().resource::<RenderDevice>().poll(
        bevy::render::render_resource::PollType::Wait {
            submission_index: Some(submitted), timeout: None,
        },
    )?;
    Ok(())
}

fn acquire_water_images(world: &World, step: usize) -> Result<(), String> {
    use crate::rendering::bevy_backend::water_scene::WaterImageSources;
    let Some(sources) = world.get_resource::<WaterImageSources>() else { return Ok(()); };
    let device = world.resource::<RenderDevice>();
    let queue = world.resource::<RenderQueue>();
    for (kind, aliases) in [("cube", &sources.skies), ("array", &sources.caustics)] {
        let mut images = std::collections::HashMap::new();
        let mut reuses = 0;
        for source in aliases.values() {
            let image = source.acquire(device, queue)?;
            let key = std::sync::Arc::as_ptr(source);
            match images.entry(key) {
                std::collections::hash_map::Entry::Occupied(entry) => {
                    reuses += usize::from(std::sync::Arc::ptr_eq(entry.get(), &image));
                }
                std::collections::hash_map::Entry::Vacant(entry) => { entry.insert(image); }
            }
        }
        eprintln!("RESOURCE_SCENE_LAYERED step={step} kind={kind} aliases={} sources={} reuses={reuses}", aliases.len(), images.len());
    }
    let noise = sources.noise.acquire(device, queue);
    eprintln!("RESOURCE_SCENE_NOISE step={step} texture_bytes={}", noise.texture_bytes());
    Ok(())
}

fn upload_assets(app: &mut App, inputs: &super::scene_preparation::SceneGpuInputs)
    -> Result<(), Box<dyn std::error::Error>>
{
    let started = std::time::Instant::now();
    let mut previous = None;
    loop {
        render_epoch(app)?;
        let world = app.sub_app(RenderApp).world();
        let images = world.resource::<RenderAssets<GpuImage>>();
        let meshes = world.resource::<RenderAssets<RenderMesh>>();
        let buffers = world.resource::<RenderAssets<GpuShaderBuffer>>();
        let missing = (
            inputs.images.iter().filter(|id| images.get(**id).is_none()).count(),
            inputs.meshes.iter().filter(|id| meshes.get(**id).is_none()).count(),
            inputs.buffers.iter().filter(|id| buffers.get(**id).is_none()).count(),
        );
        if previous != Some(missing) {
            eprintln!("RESOURCE_SCENE_GPU_PENDING images={} meshes={} buffers={}", missing.0, missing.1, missing.2);
            previous = Some(missing);
        }
        if missing == (0, 0, 0) { return Ok(()); }
        // Safety deadline for this observation, never a loading-ready rule.
        if started.elapsed() > std::time::Duration::from_secs(120) {
            return Err(format!("GPU acquisition observation timed out with {missing:?} pending assets").into());
        }
    }
}

fn gpu_snapshot(app: &App, step: usize, phase: &str) {
    let world = app.sub_app(RenderApp).world();
    eprintln!("RESOURCE_SCENE_GPU step={step} phase={phase} images={} meshes={} buffers={} skies={}",
        world.resource::<RenderAssets<GpuImage>>().iter().count(),
        world.resource::<RenderAssets<RenderMesh>>().iter().count(),
        world.resource::<RenderAssets<GpuShaderBuffer>>().iter().count(),
        world.resource::<RenderAssets<crate::rendering::bevy_backend::sky_mask::GpuSkyMaterial>>().iter().count());
    if matches!(phase, "before-preparation" | "after-gpu-removal") {
        let allocator = world.resource::<bevy::render::mesh::allocator::MeshAllocator>();
        for (id, mesh) in world.resource::<RenderAssets<RenderMesh>>().iter() {
            let vertex = allocator.mesh_vertex_slice(&id).map(|slice| (slice.range, slice.buffer.size()));
            let index = allocator.mesh_index_slice(&id).map(|slice| (slice.range, slice.buffer.size()));
            eprintln!("RESOURCE_SCENE_GPU_MESH step={step} phase={phase} id={id:?} vertices={} vertex={vertex:?} index={index:?}", mesh.vertex_count);
        }
        for (id, image) in world.resource::<RenderAssets<GpuImage>>().iter() {
            let descriptor = &image.texture_descriptor;
            eprintln!("RESOURCE_SCENE_GPU_IMAGE step={step} phase={phase} id={id:?} size={:?} format={:?} label={:?}",
                descriptor.size, descriptor.format, descriptor.label);
        }
    }
    super::resource_diagnostics::allocator_snapshot(app.world().resource::<RenderDevice>(), phase, step);
}

fn track_assets<A: bevy::asset::Asset>(world: &mut World) -> Result<(), std::io::Error> {
    world
        .run_system_once(Assets::<A>::track_assets)
        .map_err(|error| {
            std::io::Error::other(format!(
                "{} handle retirement: {error:?}",
                std::any::type_name::<A>()
            ))
        })
}

fn resource_snapshot(world: &World, step: usize, phase: &str) {
    eprintln!(
        "RESOURCE_SCENE_ASSETS step={step} phase={phase} meshes={} images={} probes={} decals={} skies={} buffers={}",
        world.resource::<Assets<Mesh>>().len(),
        world.resource::<Assets<Image>>().len(),
        world.resource::<Assets<ProbeMaterial>>().len(),
        world.resource::<Assets<DecalMaterial>>().len(),
        world
            .resource::<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>()
            .len(),
        world.resource::<Assets<ShaderBuffer>>().len()
    );
    for (name, bytes) in crate::platform::allocation_tags::report() {
        eprintln!("RESOURCE_SCENE_HEAP step={step} phase={phase} tag={name:?} bytes={bytes}");
    }
    super::resource_diagnostics::allocator_snapshot(world.resource::<RenderDevice>(), phase, step);
}

fn inspect_scene_resources(
    base: &std::path::Path,
    levels: &[String],
    mode: Mode,
) -> Result<(), Box<dyn std::error::Error>> {
    if levels.is_empty() {
        return Err("inspect-scene-preparation requires level names".into());
    }
    let catalog = archive::Catalog::shared(base)?;
    let catalog_session = super::launch::CatalogSession(catalog.identity());
    let session = super::audio::AudioSession::begin()?;
    for (step, name) in levels.iter().enumerate() {
        let level = crate::assets::level::LevelResources::resolve(&catalog, name)?;
        if let Some(map) = level.entities.strip_suffix(".entities") {
            crate::gameplay::map_info::set_current_map(&format!("{map}.map"));
        }
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=begin");
        // A fresh host per sample avoids claiming reuse of unpublished assets.
        // The real worker prepares its normal asset batches; dropping the
        // unpublished result observes completed-but-unpublished acquisition
        // retirement; it does not simulate mid-read cancellation or game frames.
        let mut app = App::new();
        let mut plugins = DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .set(bevy::render::RenderPlugin {
                    render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                        bevy::render::settings::WgpuSettings {
                            backends: cfg!(target_os = "windows")
                                .then_some(bevy::render::settings::Backends::DX12),
                            features: bevy::render::settings::WgpuSettings::default().features
                                | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8
                                | bevy::render::settings::WgpuFeatures::TEXTURE_COMPRESSION_BC
                                | bevy::render::settings::WgpuFeatures::TEXTURE_COMPRESSION_BC_SLICED_3D,
                            ..default()
                        },
                    )),
                    ..default()
                })
                .disable::<bevy::audio::AudioPlugin>()
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>();
        // Log/tracing subscribers are process-wide; later independent hosts
        // reuse the first host's subscriber rather than registering it again.
        if step != 0 { plugins = plugins.disable::<bevy::log::LogPlugin>(); }
        app.add_plugins(plugins);
        app.add_plugins(MaterialPlugin::<ProbeMaterial>::default())
            .add_plugins(MaterialPlugin::<DecalMaterial>::default())
            .add_plugins(crate::rendering::bevy_backend::sky_mask::SkyAssetsPlugin)
            .init_resource::<super::texture_streaming::SharedImages>()
            .init_resource::<super::resident_images::ResidentImages>();
        super::scene_preparation::SceneGpuInputs::install(&mut app);
        super::audio::initialize_resources(&mut app);
        app.finish();
        app.cleanup();
        let resident = app
            .world()
            .resource::<super::resident_images::ResidentImages>()
            .clone();
        app.insert_resource(super::resident_images::UiImages::new(resident));
        if mode == Mode::Upload {
            let inputs = gpu_inputs(app.world());
            upload_assets(&mut app, &inputs)?;
            gpu_snapshot(&app, step, "before-preparation");
        }
        resource_snapshot(app.world(), step, "before-preparation");
        let mut prepared = super::level_prepare::prepare_level(
            base.to_owned(),
            catalog.clone(),
            level,
            true,
            || false,
        )?;
        super::scene_preparation::ScenePreparation::new(app.world_mut()).prepare(&mut prepared)?;
        let inputs = prepared
            .get_resource::<super::scene_preparation::SceneGpuInputs>()
            .ok_or("scene worker did not produce its real asset inputs")?;
        eprintln!(
            "RESOURCE_SCENE_PREPARATION step={step} level={name} phase=prepared images={} meshes={} buffers={}",
            inputs.images.len(),
            inputs.meshes.len(),
            inputs.buffers.len()
        );
        resource_snapshot(app.world(), step, "prepared");
        if mode != Mode::Prepare {
            let (mut lifetime, before) =
                super::level_lifetime::LevelLifetime::begin(app.world_mut(), catalog.clone());
            prepared.publish(app.world_mut());
            lifetime.published(app.world(), &before);
            resource_snapshot(app.world(), step, "published");
            if mode == Mode::Upload {
                // Only asset upload/removal is observed. No camera produces a
                // view, and no gameplay or audio systems were registered.
                let mut cameras = app.world_mut().query::<&mut Camera>();
                for mut camera in cameras.iter_mut(app.world_mut()) {
                    camera.is_active = false;
                }
                let inputs = gpu_inputs(app.world());
                acquire_water_images(app.world(), step)?;
                upload_assets(&mut app, &inputs)?;
                gpu_snapshot(&app, step, "uploaded");
            }
            // No camera/view frame was produced. Upload mode completed its
            // asset queue work above; view-pass owners were not installed.
            // Use the actual main-world teardown and asset-removal owners.
            lifetime.clear(app.world_mut())?;
            resource_snapshot(app.world(), step, "level-cleared");
            // These are the registered asset owner's real handle-drop
            // consumers, in dependency order; no application schedule runs.
            track_assets::<ProbeMaterial>(app.world_mut())?;
            track_assets::<DecalMaterial>(app.world_mut())?;
            track_assets::<crate::rendering::bevy_backend::sky_mask::SkyMaterial>(app.world_mut())?;
            track_assets::<Mesh>(app.world_mut())?;
            track_assets::<ShaderBuffer>(app.world_mut())?;
            track_assets::<Image>(app.world_mut())?;
            resource_snapshot(app.world(), step, "asset-handles-tracked");
            if mode == Mode::Upload {
                // GPU material removal releases image handles; the following
                // tracking epoch removes those images, then consumes removal
                // events. Record actual final ownership rather than asserting
                // that every remaining asset is a map leak.
                for _ in 0..3 { render_epoch(&mut app)?; }
                gpu_snapshot(&app, step, "after-gpu-removal");
                resource_snapshot(app.world(), step, "after-gpu-removal");
            }
        } else {
            drop(prepared);
        }
        drop(app);
        for (tag, bytes) in crate::platform::allocation_tags::report() {
            eprintln!(
                "RESOURCE_SCENE_HEAP step={step} phase=host-released tag={tag:?} bytes={bytes}"
            );
        }
        catalog.release_level_resources();
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=released");
    }
    drop(session);
    drop(catalog);
    drop(catalog_session);
    for (tag, bytes) in crate::platform::allocation_tags::report() {
        eprintln!("RESOURCE_SCENE_HEAP phase=session-released tag={tag:?} bytes={bytes}");
    }
    Ok(())
}
