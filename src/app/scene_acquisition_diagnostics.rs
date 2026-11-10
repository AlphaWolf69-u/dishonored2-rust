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
    inspect_scene_resources(base, levels, Mode::Prepare, false)
}

pub fn inspect_scene_publication(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    inspect_scene_resources(base, levels, Mode::Publish, false)
}

pub fn inspect_scene_gpu_resources(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some((flag, paths)) = levels.split_first() {
        if flag == "--worker-images" {
            return inspect_worker_images(base, paths);
        }
        if flag == "--runtime-fx" {
            let (level, effects) = paths.split_first().filter(|(_, effects)| !effects.is_empty())
                .ok_or("--runtime-fx requires a level and at least one installed effect")?;
            return inspect_scene_resources(base, std::slice::from_ref(level), Mode::RuntimeFx(effects), false);
        }
    }
    let reuse_host = levels.first().is_some_and(|arg| arg == "--reuse-host");
    inspect_scene_resources(base, &levels[usize::from(reuse_host)..], Mode::Upload, reuse_host)
}

/// Run the worker's installed-image reader and the renderer's normal upload
/// mailbox without a gameplay frame or visibility producer.
fn inspect_worker_images(
    base: &std::path::Path,
    paths: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    use super::texture_streaming::{GpuMipCopy, MipPayload, PendingGpuImageUpload, TextureReadTarget};
    if paths.is_empty() { return Err("--worker-images requires installed image paths".into()); }
    let catalog = archive::Catalog::open(base)?;
    let mut app = scene_host(true);
    let mailbox = PreparedImageMailbox::default();
    app.sub_app_mut(RenderApp)
        .insert_resource(mailbox.clone())
        .add_systems(Render, report_prepared_diffuse_images.after(RenderSystems::PrepareAssets));
    let mut handles = Vec::with_capacity(paths.len());
    let mut archive_members = Vec::new();
    for requested in paths {
        // A compiled separate-mip path selects that suffix as the initial
        // worker target. A base path still requests the inline chain.
        let (path, preferred) = if let Some((stem, suffix)) = requested.rsplit_once(".bimage7_mip") {
            let mip = suffix.parse::<u8>()?;
            if mip > 15 { return Err(format!("invalid worker mip suffix in {requested}").into()); }
            (&requested[..stem.len() + ".bimage7".len()], Some(mip))
        } else {
            (requested.as_str(), None)
        };
        let start = crate::loading::thread_cycles();
        let streamed = super::texture_streaming::read_streamed_diffuse_at_target(
            &catalog, path, TextureReadTarget::Initial(preferred),
        )?.ok_or_else(|| format!("missing worker image {path}"))?;
        let cycles = start.zip(crate::loading::thread_cycles())
            .map_or(0, |(start, end)| end.saturating_sub(start));
        let mut crc = flate2::Crc::new();
        let mut bytes = 0;
        let mut owned_capacity = 0;
        for mip in &streamed.uploads {
            crc.update(&mip.payload);
            bytes += mip.payload.len();
            if let MipPayload::Owned(payload) = &mip.payload { owned_capacity += payload.capacity(); }
        }
        let member_capacity = match streamed.uploads.first().map(|mip| &mip.payload) {
            Some(MipPayload::Member(member, _)) => {
                archive_members.push(Arc::downgrade(member));
                member.capacity()
            }
            _ => 0,
        };
        eprintln!("RESOURCE_WORKER_IMAGE path={requested} bytes={bytes} extent={}x{} mips={} format={:?} next={:?} crc32={:08x} cycles={cycles} owned_capacity={owned_capacity} member_capacity={member_capacity}",
            streamed.level.width, streamed.level.height, streamed.uploads.len(), streamed.image.texture_descriptor.format,
            streamed.level.next_separate_suffix, crc.sum());
        if preferred.is_some() {
            let mut resident = super::texture_streaming::read_resident_diffuse_image(&catalog, path, preferred)?
                .ok_or_else(|| format!("missing resident image {path}"))?;
            resident.pack_uploads();
            let data = resident.image.data.as_deref().ok_or("resident chain has no payload")?;
            let mut resident_crc = flate2::Crc::new();
            resident_crc.update(data);
            let desc = &resident.image.texture_descriptor;
            let worker = &streamed.image.texture_descriptor;
            let matched = crc.sum() == resident_crc.sum() && bytes == data.len()
                && streamed.level == resident.level && streamed.level.resource_mip == preferred
                && worker.size == desc.size && worker.format == desc.format
                && worker.mip_level_count == desc.mip_level_count;
            eprintln!("RESOURCE_WORKER_CHAIN path={requested} matched_resident={matched} selected={preferred:?}");
            if !matched { return Err(format!("worker initial chain differs from resident view {requested}").into()); }
        }
        let handle = app.world_mut().resource_mut::<Assets<Image>>().add(streamed.image);
        mailbox.0.lock().unwrap().pending.push(PendingGpuImageUpload {
            id: handle.id(), previous: None, copy: GpuMipCopy::None, uploads: streamed.uploads,
        });
        handles.push(handle);
    }
    let started = std::time::Instant::now();
    let mut completed = std::collections::HashSet::new();
    while completed.len() < handles.len() {
        render_epoch(&mut app)?;
        let mut state = mailbox.0.lock().unwrap();
        if let Some((id, error)) = state.failed.first() {
            return Err(format!("worker upload {id:?} failed: {error}").into());
        }
        completed.extend(state.completed.drain(..));
        if started.elapsed() > std::time::Duration::from_secs(120) {
            return Err(format!("{} worker uploads remain pending", handles.len() - completed.len()).into());
        }
    }
    let ids: Vec<_> = handles.iter().map(Handle::id).collect();
    drop(handles);
    track_assets::<Image>(app.world_mut())?;
    for _ in 0..3 { render_epoch(&mut app)?; }
    let gpu = app.sub_app(RenderApp).world().resource::<RenderAssets<GpuImage>>();
    let remaining = ids.iter().filter(|id| gpu.get(**id).is_some()).count();
    let retained_members = archive_members.iter().filter(|member| member.strong_count() != 0).count();
    eprintln!("RESOURCE_WORKER_IMAGE_LIFETIME uploaded={} retained={remaining} retained_members={retained_members}", completed.len());
    if remaining != 0 { return Err(format!("{remaining} worker images remain resident").into()); }
    if retained_members != 0 { return Err(format!("{retained_members} worker members remain owned after upload").into()); }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Mode<'a> { Prepare, Publish, Upload, RuntimeFx(&'a [String]) }

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
        let atlases = app.world().resource::<bevy::text::FontAtlasSet>();
        let gpu_images = world.resource::<RenderAssets<GpuImage>>();
        for (font, pages) in atlases.iter() {
            for atlas in pages {
                let id = atlas.texture.id();
                let gpu = gpu_images.get(id).map(|image| {
                    (image.texture.id(), image.texture_descriptor.size)
                });
                eprintln!("RESOURCE_SCENE_OVERLAY_ATLAS step={step} phase={phase} font_px={} asset={id:?} gpu={gpu:?}",
                    f32::from_bits(font.font_size_bits));
            }
        }
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

/// Observe real runtime material and texture publication, with no view or
/// effect simulation. The requested definitions come from installed assets.
fn inspect_runtime_fx(app: &mut App, catalog: &archive::Catalog, effects: &[String])
    -> Result<(), Box<dyn std::error::Error>>
{
    use super::{runtime_fx_images, runtime_materials, usable_effects::{DynamicDecalMaterials, DynamicWorldMaterials, ParticleResourceOwner}};
    use crate::rendering::{bevy_backend::{projected_decals::DynamicProxyReadiness, world_effects::DynamicMaterialPrewarm}, particles::world_sources::WorldFxSources};
    use std::sync::{Arc, atomic::Ordering};

    let mailbox = PreparedImageMailbox::default();
    app.insert_resource(mailbox.clone())
        .init_resource::<DynamicProxyReadiness>().init_resource::<DynamicMaterialPrewarm>();
    app.sub_app_mut(RenderApp).insert_resource(mailbox)
        .add_systems(Render, report_prepared_diffuse_images.after(RenderSystems::PrepareAssets));
    let world = app.world_mut();
    let sources = {
        let mut programs = world.resource::<ProgramCacheSource>().0.lock().unwrap_or_else(|error| error.into_inner());
        let mut particles = world.resource::<ParticleResourceOwner>().0.lock().unwrap_or_else(|error| error.into_inner());
        Arc::new(WorldFxSources::load_named_shared(effects.to_vec(), catalog,
            programs.as_mut().map_err(|error| error.clone())?, &mut particles))
    };
    let meshes = sources.world_meshes().len();
    let decals = sources.decal_materials().len();
    if meshes == 0 && decals == 0 { return Err("installed FX has no admitted world meshes or decals".into()); }
    eprintln!("RESOURCE_RUNTIME_FX_REQUEST meshes={meshes} decals={decals}");
    let baseline_slots = world.resource::<SceneRenderQueue>().diffuse_image_paths.len();
    let baseline_meshes: std::collections::HashSet<_> = world.resource::<Assets<Mesh>>().ids().collect();
    let baseline_decals: std::collections::HashSet<_> = world.resource::<Assets<DecalMaterial>>().ids().collect();

    let request = |world: &mut World, generation, sources: Arc<WorldFxSources>, leases: Arc<Vec<std::sync::Weak<WorldFxSources>>>| {
        let mut materials = world.resource_mut::<DynamicWorldMaterials>();
        materials.generation = generation;
        materials.sources = sources.clone();
        materials.leases = leases.clone();
        let mut decals = world.resource_mut::<DynamicDecalMaterials>();
        decals.generation = generation;
        decals.sources = sources;
        decals.leases = leases;
    };
    let epoch = |app: &mut App| -> Result<(), Box<dyn std::error::Error>> {
        let world = app.world_mut();
        world.run_system_once(super::texture_streaming::stream_visible_textures)
            .map_err(|error| format!("FX texture worker: {error:?}"))?;
        world.run_system_once(runtime_fx_images::synchronize)
            .map_err(|error| format!("FX image bindings: {error:?}"))?;
        runtime_materials::coordinate(world);
        render_epoch(app)
    };
    let leases = Arc::new(vec![Arc::downgrade(&sources)]);
    request(world, 1, sources, leases);
    let started = std::time::Instant::now();
    loop {
        epoch(app)?;
        let world = app.world();
        let materials = world.resource::<DynamicWorldMaterials>();
        let prepared = world.resource::<DynamicDecalMaterials>();
        if materials.failed.load(Ordering::Acquire) == 1 || prepared.failed.load(Ordering::Acquire) == 1 {
            return Err("runtime FX publication failed; see material and texture diagnostics".into());
        }
        if prepared.prepared_generation == 1 && materials.images_ready == materials.image_revision {
            break;
        }
        if started.elapsed() > std::time::Duration::from_secs(120) {
            return Err("runtime FX publication observation timed out".into());
        }
    }
    let published = gpu_inputs(app.world());
    upload_assets(app, &published)?;
    let world = app.world();
    let proxies = &world.resource::<DynamicProxyReadiness>().world_meshes.proxies;
    let queue = world.resource::<SceneRenderQueue>();
    let slots = world.resource::<runtime_fx_images::RuntimeFxImages>();
    let images: std::collections::HashSet<_> = world.resource::<DynamicWorldMaterials>().extra_images.iter()
        .filter_map(|image| slots.slot(&image.path)).filter(|&slot| slot >= baseline_slots)
        .filter_map(|slot| queue.diffuse_image_handles[slot].as_ref().map(Handle::id)).collect();
    let meshes: std::collections::HashSet<_> = proxies.iter().filter_map(|&entity| world.get::<Mesh3d>(entity))
        .map(|mesh| mesh.0.id()).filter(|id| !baseline_meshes.contains(id)).collect();
    let decal_materials: std::collections::HashSet<_> = world.resource::<DynamicDecalMaterials>().prepared.iter()
        .map(|decal| decal.handle.id()).filter(|id| !baseline_decals.contains(id)).collect();
    eprintln!("RESOURCE_RUNTIME_FX_PUBLISH proxies={} images={} meshes={} decal_materials={}",
        proxies.len(), images.len(), meshes.len(), decal_materials.len());
    if images.is_empty() && meshes.is_empty() && decal_materials.is_empty() {
        return Err("runtime FX reused startup assets; select a level without those resources".into());
    }

    request(app.world_mut(), 2, Arc::default(), Arc::default());
    let started = std::time::Instant::now();
    loop {
        epoch(app)?;
        let world = app.world_mut();
        track_assets::<ProbeMaterial>(world)?;
        track_assets::<DecalMaterial>(world)?;
        track_assets::<Mesh>(world)?;
        track_assets::<Image>(world)?;
        let decals_left = decal_materials.iter().filter(|id| world.resource::<Assets<DecalMaterial>>().get(**id).is_some()).count();
        let render = app.sub_app(RenderApp).world();
        let images_left = images.iter().filter(|id| render.resource::<RenderAssets<GpuImage>>().get(**id).is_some()).count();
        let meshes_left = meshes.iter().filter(|id| render.resource::<RenderAssets<RenderMesh>>().get(**id).is_some()).count();
        if (images_left, meshes_left, decals_left) == (0, 0, 0) { break; }
        if started.elapsed() > std::time::Duration::from_secs(120) {
            return Err(format!("runtime FX release timed out: images={images_left} meshes={meshes_left} decals={decals_left}").into());
        }
    }
    eprintln!("RESOURCE_RUNTIME_FX_RELEASE images=0 meshes=0 decal_materials=0");
    let (free, requests, requested_free) = super::texture_streaming::inspect_retired_slot_demand(
        app.world().resource::<SceneRenderQueue>(),
    );
    eprintln!("RESOURCE_RUNTIME_FX_RETIRED_SLOTS free={free} requests={requests} requested_free={requested_free}");
    Ok(())
}

fn resource_snapshot(world: &World, step: usize, phase: &str) {
    eprintln!("RESOURCE_SCENE_FX step={step} phase={phase} simulation={}",
        world.contains_resource::<crate::rendering::bevy_backend::camera_scene::FxSimulation>());
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

fn scene_host(first_host: bool) -> App {
    use bevy::render::settings::MemoryHints;
    let memory_hints = if std::env::var_os("DH2_DEBUG_GPU_SMALL_HEAPS").is_some_and(|v| v == "1") {
        MemoryHints::MemoryUsage
    } else {
        MemoryHints::default()
    };
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
                        memory_hints,
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
    if !first_host { plugins = plugins.disable::<bevy::log::LogPlugin>(); }
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
    // Give UI layout a target size without allocating a window or drawing a view.
    app.world_mut().spawn((
        Camera3d::default(),
        Camera { is_active: false, ..default() },
        bevy::camera::RenderTarget::None { size: UVec2::new(1366, 768) },
        bevy::ui::IsDefaultUiCamera,
    ));
    app.world_mut()
        .run_system_once(super::performance::bootstrap_overlay)
        .expect("prepare the diagnostic host overlay");
    let resident = app
        .world()
        .resource::<super::resident_images::ResidentImages>()
        .clone();
    app.insert_resource(super::resident_images::UiImages::new(resident));
    app
}

fn inspect_scene_resources(
    base: &std::path::Path,
    levels: &[String],
    mode: Mode<'_>,
    reuse_host: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if levels.is_empty() {
        return Err("inspect-scene-preparation requires level names".into());
    }
    let catalog = archive::Catalog::shared(base)?;
    let catalog_session = super::launch::CatalogSession(catalog.identity());
    let session = super::audio::AudioSession::begin()?;
    let mut host = None;
    for (step, name) in levels.iter().enumerate() {
        let level = crate::assets::level::LevelResources::resolve(&catalog, name)?;
        if let Some(map) = level.entities.strip_suffix(".entities") {
            crate::gameplay::map_info::set_current_map(&format!("{map}.map"));
        }
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=begin");
        // Fresh hosts isolate each sample by default. Reuse mode carries the
        // asset storage and render device across actual map retirement.
        let new_host = host.is_none();
        let mut app = host.take().unwrap_or_else(|| scene_host(step == 0));
        if matches!(mode, Mode::Upload | Mode::RuntimeFx(_)) {
            if new_host {
                let resident = app.world().resource::<super::resident_images::ResidentImages>().clone();
                resident.prepare_intrinsics(&catalog, &mut *app.world_mut().resource_mut::<Assets<Image>>())?;
            }
            if new_host {
                // UI layout creates the glyph image; include it in the real upload set.
                render_epoch(&mut app)?;
            }
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
            |_| false,
        )?;
        super::scene_preparation::ScenePreparation::new(app.world_mut()).prepare(&mut prepared, || false)?;
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
            if matches!(mode, Mode::Upload | Mode::RuntimeFx(_)) {
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
            if let Mode::RuntimeFx(effects) = mode {
                inspect_runtime_fx(&mut app, &catalog, effects)?;
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
            if matches!(mode, Mode::Upload | Mode::RuntimeFx(_)) {
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
        if reuse_host {
            host = Some(app);
        } else {
            drop(app);
            for (tag, bytes) in crate::platform::allocation_tags::report() {
                eprintln!(
                    "RESOURCE_SCENE_HEAP step={step} phase=host-released tag={tag:?} bytes={bytes}"
                );
            }
        }
        catalog.release_level_resources();
        eprintln!("RESOURCE_SCENE_PREPARATION step={step} level={name} phase=released");
    }
    drop(host);
    drop(session);
    drop(catalog);
    drop(catalog_session);
    for (tag, bytes) in crate::platform::allocation_tags::report() {
        eprintln!("RESOURCE_SCENE_HEAP phase=session-released tag={tag:?} bytes={bytes}");
    }
    Ok(())
}
