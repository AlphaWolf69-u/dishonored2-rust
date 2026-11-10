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

/// Observe a real overlay file edited by the diagnostic caller between reads.
fn inspect_editable_sound_event(base: &std::path::Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    use crate::assets::wwise::event_resources;
    let catalog = archive::Catalog::open(base)?;
    let path = format!("generated/decls/soundevent/{name}.decl");
    if catalog.kind(&path) != Some("soundevent") || !catalog.has_overlay_file(&path) {
        return Err("editable sound observation requires a mounted replacement for an installed event".into());
    }
    let acquire = || event_resources::acquire_with(&catalog, name, |path| {
        catalog.read(path).map(|bytes| bytes.map(crate::entities::owned_declaration_text))
            .map_err(|error| error.to_string())
    });
    let recorded = || crate::loading::catalog_read_recorded(catalog.identity(), "soundevent", &path);
    let next_edit = |phase| -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("EDITABLE_SOUND_OBSERVATION phase={phase}");
        if std::io::stdin().read_line(&mut String::new())? == 0 {
            return Err("editable sound observation ended before the next edit".into());
        }
        Ok(())
    };

    let error = acquire().err().ok_or("initial malformed event was accepted")?;
    if !error.contains("no engine ID") {
        return Err(format!("unexpected initial sound declaration error: {error}").into());
    }
    next_edit("repair")?;
    let first = acquire()?;
    let first_id = first.id;
    next_edit("replace")?;
    let second = acquire()?;
    let second_id = second.id;
    if first_id == second_id || first.id != first_id {
        return Err("the edited event did not publish a distinct generation".into());
    }
    catalog.release_level_resources();
    if !recorded() { return Err("live editable event lost its read history".into()); }
    drop(first);
    catalog.release_level_resources();
    if !recorded() { return Err("the older generation removed the replacement's history".into()); }
    drop(second);
    catalog.release_level_resources();
    if recorded() { return Err("editable event history outlived both generations".into()); }
    eprintln!("EDITABLE_SOUND_OBSERVATION first_id={first_id} second_id={second_id} retired=true");
    Ok(())
}

fn inspect_fx_history_scopes(base: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use crate::assets::{
        fx::{particle_declaration, particle_resources},
        parm_block::ParmBlockSources,
        particle_model,
    };
    use std::sync::Arc;

    const DECLARATION_NAME: &str = "models/effects/empty.particledt";
    const DECLARATION: &str = "generated/decls/particledt/models/effects/empty.particledt.decl";
    const MODEL: &str = "generated/staticparticlemodel/models/effects/meshes/cylinder_no_cap_1m.pmodel";
    const BLOCK: &str = "generated/decls/pblock/default.pblock.decl";
    let a = archive::Catalog::open(base)?;
    let b = archive::Catalog::open(base)?;
    let acquire_particles = |catalog: &archive::Catalog, shared| {
        let declaration = particle_resources::declaration(catalog, DECLARATION_NAME, shared, |_| {
            let fields = crate::gameplay::options::decl::read_edit(
                &mut |path| catalog.read(path).ok().flatten(), "particledt", DECLARATION_NAME,
            )?;
            particle_declaration::declaration(&fields).map(Arc::new)
        })?;
        let model = particle_resources::model(catalog, MODEL, 0, shared, || {
            let bytes = catalog.read(MODEL).map_err(|error| error.to_string())?
                .ok_or("installed particle model absent")?;
            particle_model::parse(&bytes).map(Arc::new)
        })?;
        Ok::<_, String>((declaration, model))
    };
    let first = acquire_particles(&a, false)?;
    let replacement = acquire_particles(&a, false)?;
    let other = acquire_particles(&b, true)?;
    if Arc::ptr_eq(&first.0, &replacement.0) || Arc::ptr_eq(&first.1, &replacement.1) {
        return Err("uncached particle generations share a decoded payload".into());
    }
    let mut programs_a = shader_cache::ProgramLibrary::open(base, shader_cache::ShaderCacheBackend::Pc, &a)?;
    let mut programs_b = shader_cache::ProgramLibrary::open(base, shader_cache::ShaderCacheBackend::Pc, &b)?;
    let mut blocks_a = ParmBlockSources::new(&a);
    let mut blocks_b = ParmBlockSources::new(&b);
    let first_block = blocks_a.acquire(&a, BLOCK, |name| programs_a.parameter_value_metadata(&a, name))?;
    let other_block = blocks_b.acquire(&b, BLOCK, |name| programs_b.parameter_value_metadata(&b, name))?;
    let paths = [("particledt", DECLARATION), ("staticParticleModel", MODEL), ("pblock", BLOCK)];
    let records = |catalog: &archive::Catalog| paths.map(|(kind, path)|
        crate::loading::catalog_read_recorded(catalog.identity(), kind, path));
    a.release_level_resources();
    drop(first);
    a.release_level_resources();
    if records(&a) != [true; 3] || records(&b) != [true; 3] {
        return Err("an active FX generation lost its catalog read history".into());
    }
    drop(replacement);
    drop(first_block);
    a.release_level_resources();
    if records(&a) != [false; 3] || records(&b) != [true; 3] {
        return Err("FX history retirement crossed its catalog boundary".into());
    }
    drop(other);
    drop(other_block);
    b.release_level_resources();
    if records(&b) != [false; 3] { return Err("FX history survived its final marker".into()); }
    eprintln!("FX_HISTORY_SCOPES kinds=3 independent=true separate_generations=true retired=true");
    Ok(())
}

pub fn inspect_scene_publication(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some((flag, paths)) = levels.split_first() {
        if flag == "--resident-views" {
            if paths.is_empty() { return Err("--resident-views requires installed image paths".into()); }
            let catalog = archive::Catalog::open(base)?;
            for path in paths { super::resident_images::inspect_view_replacement(&catalog, path)?; }
            return Ok(());
        }
        if flag == "--catalog-history-scopes" {
            return inspect_catalog_history_scopes(base);
        }
        if flag == "--shader-session-scopes" {
            return crate::assets::shader_cache::observe_session_scopes(base).map_err(Into::into);
        }
        if flag == "--map-info-inventory" {
            return crate::gameplay::map_info::observe_inventory(base).map_err(Into::into);
        }
    }
    if let [flag, name] = levels {
        match flag.as_str() {
            "--editable-sound-event" => return inspect_editable_sound_event(base, name),
            "--manifest-catalog-scopes" => {
                return crate::assets::map_resources::observe_catalog_manifests(base, name).map_err(Into::into);
            }
            "--map-info-catalog-scopes" => {
                return crate::gameplay::map_info::observe_catalog_scopes(base, name).map_err(Into::into);
            }
            "--physics-system-scopes" => {
                return crate::assets::physworld::physics_system::observe_resource_scopes(base, name).map_err(Into::into);
            }
            "--contact-catalog-scopes" => {
                return crate::assets::declaration_texts::observe_catalog_scopes(base, name).map_err(Into::into);
            }
            _ => {}
        }
    }
    if let [flag, level, effect] = levels {
        if flag == "--fx-requests" {
            return inspect_scene_resources(base, std::slice::from_ref(level), Mode::FxRequests(effect), false);
        }
    }
    inspect_scene_resources(base, levels, Mode::Publish, false)
}

fn inspect_catalog_history_scopes(base: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let a = Arc::new(archive::Catalog::open(base)?);
    let b = Arc::new(archive::Catalog::open(base)?);
    let leases = |catalog: &Arc<archive::Catalog>| (
        crate::bmodel::ModelHistoryLease::acquire(catalog),
        crate::material::MaterialHistoryLease::acquire(catalog),
        crate::physworld::source::ShapeSources::new(catalog),
        super::level_prepare::SkeletalHistoryScope::new(catalog),
    );
    let first = leases(&a);
    let second = leases(&b);
    const COMMON_MODEL: &str = "generated/model/models/environment/props/generic/bottle_set_01/bottle_04.bmodel";
    let common = crate::bmodel::CommonModels::shared(&a).ok_or("installed common model owner unavailable")?;
    let model = crate::bmodel::acquire(&a, COMMON_MODEL, Some(common))?.ok_or("installed common model absent")?;
    drop(model);
    let mut paths = Vec::new();
    for (kind, suffix) in [
        ("model", ".bmodel"), ("material", ".decl"), ("physhape", ".hkx"),
        ("baseModel", ".bmd6model"), ("skeleton", ".edgeskl"),
    ] {
        let path = a.paths_with_suffix(suffix).into_iter()
            .find(|path| path != COMMON_MODEL && a.kind(path) == Some(kind))
            .ok_or_else(|| format!("installed resource kind {kind} absent"))?;
        let before = a.read(&path)?.ok_or("first installed resource missing")?;
        let other = b.read(&path)?.ok_or("second installed resource missing")?;
        if before != other { return Err(format!("installed resource changed between catalogs: {path}").into()); }
        paths.push((kind, path));
    }
    drop(first);
    for (kind, path) in &paths {
        if crate::loading::catalog_read_recorded(a.identity(), kind, path)
            || !crate::loading::catalog_read_recorded(b.identity(), kind, path) {
            return Err(format!("{kind} history retired outside its catalog scope").into());
        }
    }
    if !crate::loading::catalog_read_recorded(a.identity(), "model", COMMON_MODEL) {
        return Err("live common model lost its loading credit".into());
    }
    drop(second);
    for (kind, path) in &paths {
        if crate::loading::catalog_read_recorded(b.identity(), kind, path) {
            return Err(format!("{kind} history outlived its final scope").into());
        }
    }
    use crate::assets::animation_rig::declaration::{LoadContext, resolve_model};
    const LIVE_MD6: &str = "models/characters/animals/bloodfly/bloodfly.md6";
    const RAW_MD6: &str = "generated/decls/md6def/models/characters/animals/rat/rat.md6.decl";
    let live = resolve_model(&a, LIVE_MD6, true, LoadContext::Level)?.ok_or("installed MD6 declaration absent")?;
    let live_path = &live.declaration().path;
    a.read(RAW_MD6)?.ok_or("installed compatibility MD6 source absent")?;
    b.read(RAW_MD6)?.ok_or("second compatibility MD6 source absent")?;
    a.release_md6_level();
    if !crate::loading::catalog_read_recorded(a.identity(), "md6Def", live_path)
        || crate::loading::catalog_read_recorded(a.identity(), "md6Def", RAW_MD6)
        || !crate::loading::catalog_read_recorded(b.identity(), "md6Def", RAW_MD6) {
        return Err("MD6 history lost its registered owner or retired outside its catalog".into());
    }
    let live_path = live_path.clone();
    drop(live);
    a.release_md6_level();
    b.release_md6_level();
    if crate::loading::catalog_read_recorded(a.identity(), "md6Def", &live_path)
        || crate::loading::catalog_read_recorded(b.identity(), "md6Def", RAW_MD6) {
        return Err("MD6 history outlived its final owner".into());
    }
    eprintln!("CATALOG_LEVEL_SCOPES retired={} independent=true common_model=true md6=true", paths.len());
    Ok(())
}

pub fn inspect_scene_gpu_resources(
    base: &std::path::Path,
    levels: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    if let Some((flag, paths)) = levels.split_first() {
        if flag == "--fx-history-scopes" {
            return inspect_fx_history_scopes(base);
        }
        if flag == "--ocean-readback" {
            let app = scene_host(true);
            let world = app.sub_app(RenderApp).world();
            crate::rendering::bevy_backend::ocean_height_queries::observe_readback(
                world.resource(), world.resource(),
            )?;
            return Ok(());
        }
        if flag == "--worker-images" {
            return inspect_worker_images(base, paths);
        }
        if flag == "--view-depth-retirement" {
            let mut app = scene_host(true);
            return crate::rendering::bevy_backend::coarse_depth::observe_view_retirement(
                app.sub_app_mut(RenderApp).world_mut(),
            ).map_err(Into::into);
        }
        if flag == "--area-tile-retirement" {
            let mut app = scene_host(true);
            return crate::rendering::bevy_backend::area_light_scene::observe_tile_retirement(
                app.sub_app_mut(RenderApp).world_mut(),
            ).map_err(Into::into);
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
enum Mode<'a> { Prepare, Publish, Upload, RuntimeFx(&'a [String]), FxRequests(&'a str) }

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
    let load_sources = |effects: Vec<String>| -> Result<Arc<WorldFxSources>, String> {
        let mut programs = world.resource::<ProgramCacheSource>().0.lock().unwrap_or_else(|error| error.into_inner());
        let mut particles = world.resource::<ParticleResourceOwner>().0.lock().unwrap_or_else(|error| error.into_inner());
        Ok(Arc::new(WorldFxSources::load_named_shared(effects, catalog,
            programs.as_mut().map_err(|error| error.clone())?, &mut particles)))
    };
    let sources = load_sources(effects.to_vec())?;
    let replacement = std::env::var("DH2_DEBUG_RUNTIME_FX_REPLACEMENT").ok()
        .map(|effect| load_sources(vec![effect])).transpose()?;
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
    let update_images = |world: &mut World| -> Result<(), Box<dyn std::error::Error>> {
        world.run_system_once(super::texture_streaming::stream_visible_textures)
            .map_err(|error| format!("FX texture worker: {error:?}"))?;
        world.run_system_once(runtime_fx_images::synchronize)
            .map_err(|error| format!("FX image bindings: {error:?}"))?;
        Ok(())
    };
    let epoch = |app: &mut App| -> Result<(), Box<dyn std::error::Error>> {
        let world = app.world_mut();
        update_images(world)?;
        runtime_materials::coordinate(world);
        world.run_system_once(super::projected_decals::publish_runtime)
            .map_err(|error| format!("FX decal proxies: {error:?}"))?;
        render_epoch(app)
    };
    if let Ok(path) = std::env::var("DH2_DEBUG_RUNTIME_FX_ALIAS_IMAGE") {
        observe_runtime_fx_image_aliases(app, catalog, &path, &|app| {
            update_images(app.world_mut())?;
            render_epoch(app)
        })?;
    }
    let world = app.world_mut();
    let leases = Arc::new(vec![Arc::downgrade(&sources)]);
    let generation = if std::env::var_os("DH2_DEBUG_MATERIAL_SUPERSESSION").is_some_and(|value| value == "1") {
        // Submit the real installed material batch, then replace its revision
        // before any worker output can enter the image or render owners.
        request(world, 1, sources.clone(), leases.clone());
        runtime_materials::coordinate(world);
        2
    } else { 1 };
    request(world, generation, sources, leases);
    let started = std::time::Instant::now();
    loop {
        epoch(app)?;
        let world = app.world();
        let materials = world.resource::<DynamicWorldMaterials>();
        let prepared = world.resource::<DynamicDecalMaterials>();
        if materials.failed.load(Ordering::Acquire) == generation || prepared.failed.load(Ordering::Acquire) == generation {
            return Err("runtime FX publication failed; see material and texture diagnostics".into());
        }
        if prepared.prepared_generation == Some(generation) && materials.images_ready == materials.image_revision {
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
    if std::env::var_os("DH2_DEBUG_PARTICLE_SCENE_RETIREMENT").is_some_and(|value| value == "1") {
        crate::rendering::bevy_backend::particle_images::observe_scene_retirement(
            world.resource(), app.sub_app(RenderApp).world().resource(), &images,
        )?;
    }
    if images.is_empty() && meshes.is_empty() && decal_materials.is_empty() {
        return Err("runtime FX reused startup assets; select a level without those resources".into());
    }

    let next_generation = generation + 1;
    let retirement_generation = if let Some(replacement) = replacement {
        let world = app.world_mut();
        let previous = world.resource::<DynamicProxyReadiness>().decals.proxies.clone();
        let leases = Arc::new(vec![Arc::downgrade(&replacement)]);
        request(world, next_generation, replacement, leases);
        runtime_materials::coordinate(world);
        world.run_system_once(super::projected_decals::publish_runtime)
            .map_err(|error| format!("FX decal replacement: {error:?}"))?;
        let published = world.resource::<DynamicDecalMaterials>();
        let pending = published.prepared_generation != Some(next_generation);
        let retained = previous.iter().filter(|&&entity| world.get_entity(entity).is_ok()).count();
        eprintln!("RESOURCE_RUNTIME_FX_DECAL_REPLACEMENT pending={pending} previous_proxies={} retained_proxies={retained} retained_materials={}",
            previous.len(), published.prepared.len());
        next_generation + 1
    } else { next_generation };
    request(app.world_mut(), retirement_generation, Arc::default(), Arc::default());
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

fn observe_runtime_fx_image_aliases(
    app: &mut App,
    catalog: &archive::Catalog,
    path: &str,
    epoch: &impl Fn(&mut App) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    use super::{runtime_fx_images::RuntimeFxImages, usable_effects::DynamicWorldMaterials};
    use crate::rendering::particles::{definition::ParticleResources, image_resources::ParticleImage};
    use std::sync::Arc;

    if app.world().resource::<SceneRenderQueue>().diffuse_image_paths.iter().any(|existing| existing == path) {
        return Err("FX alias observation needs an image absent from the prepared map".into());
    }
    let acquire = || -> Result<Arc<ParticleImage>, String> {
        let mut resources = ParticleResources::default();
        resources.use_scene_images();
        resources.scene_image(catalog, path)?.ok_or_else(|| format!("missing scene image {path}"))
    };
    let publish = |app: &mut App, images: Vec<Arc<ParticleImage>>| {
        let mut world = app.world_mut().resource_mut::<DynamicWorldMaterials>();
        world.extra_images = images;
        world.image_revision = world.image_revision.wrapping_add(1);
    };
    let stage = |app: &mut App| -> Result<usize, Box<dyn std::error::Error>> {
        publish(app, vec![acquire()?]);
        app.world_mut().run_system_once(super::runtime_fx_images::synchronize)
            .map_err(|error| format!("FX image bindings: {error:?}"))?;
        app.world().resource::<SceneRenderQueue>().diffuse_image_paths.iter().position(|name| name == path)
            .ok_or_else(|| "FX image did not reserve a streaming slot".into())
    };
    let stream = |app: &mut App| -> Result<(), Box<dyn std::error::Error>> {
        app.world_mut().run_system_once(super::texture_streaming::stream_visible_textures)
            .map_err(|error| format!("FX texture worker: {error:?}"))?;
        Ok(())
    };

    use super::texture_streaming::TextureReadWorker;
    let slot = stage(app)?;
    // Leave the slot's previous demand in place as runtime material
    // coordination can do after PostUpdate synchronization.
    publish(app, Vec::new());
    stream(app)?;
    if app.world().resource::<TextureReadWorker>().inflight[slot] {
        return Err("FX texture read admitted after its final owner released".into());
    }
    epoch(app)?;

    let slot = stage(app)?;
    stream(app)?;
    let bytes = {
        let mut worker = app.world_mut().resource_mut::<TextureReadWorker>();
        if !worker.inflight[slot] { return Err("FX texture worker did not admit the live request".into()); }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let completion = worker.completed.get_mut().unwrap().recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))?;
            let bytes = if completion.0 == slot {
                let image = completion.2.as_ref().map_err(Clone::clone)?.as_ref().ok_or("FX image read returned no payload")?;
                Some(image.uploads.iter().map(|mip| mip.payload.len()).sum::<usize>()
                    + image.image.data.as_ref().map_or(0, Vec::len))
            } else { None };
            worker.ready.push_back(completion);
            if let Some(bytes) = bytes { break bytes; }
        }
    };
    publish(app, Vec::new());
    stream(app)?;
    let world = app.world();
    let worker = world.resource::<TextureReadWorker>();
    let queue = world.resource::<SceneRenderQueue>();
    let discarded = bytes != 0 && !worker.inflight[slot]
        && queue.diffuse_mip_states[slot].reserved_bytes == 0
        && queue.diffuse_image_pending[slot].is_none() && queue.diffuse_image_handles[slot].is_none();
    eprintln!("RESOURCE_RUNTIME_FX_IMAGE_CANCELLATION slot={slot} read_admission_suppressed=true payload_bytes={bytes} discarded_before_upload={discarded}");
    if !discarded { return Err("FX texture completion reached the upload owner after release".into()); }
    epoch(app)?;

    let first = acquire()?;
    let second = acquire()?;
    if Arc::ptr_eq(&first, &second) || first.path != second.path {
        return Err("FX alias observation did not acquire distinct descriptors of one path".into());
    }
    let binding = |app: &App| {
        let world = app.world();
        let slot = world.resource::<RuntimeFxImages>().slot(path)?;
        world.resource::<SceneRenderQueue>().diffuse_image_handles[slot].as_ref().map(|image| (slot, image.id()))
    };
    publish(app, vec![first, second.clone()]);
    let start = std::time::Instant::now();
    let initial = loop {
        epoch(app)?;
        if let Some(binding) = binding(app) { break binding; }
        if start.elapsed() > std::time::Duration::from_secs(60) {
            return Err("FX alias image did not reach the streaming registry".into());
        }
    };
    publish(app, vec![second.clone()]);
    epoch(app)?;
    let replacement = binding(app) == Some(initial);
    publish(app, Vec::new());
    epoch(app)?;
    let tail = binding(app) == Some(initial);
    drop(second);
    epoch(app)?;
    let released = binding(app).is_none();
    eprintln!("RESOURCE_RUNTIME_FX_IMAGE_ALIAS slot={} replacement={replacement} tail={tail} released={released}", initial.0);
    if !(replacement && tail && released) {
        return Err("FX image slot ownership did not follow both source identities".into());
    }
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
    if std::env::var_os("DH2_DEBUG_OBSERVE_INITIAL_IMAGE_LOAD").is_some_and(|v| v == "1") {
        super::loading_screen::install_scene_image_requests(&mut app);
        super::loading_screen::limit_loading_uploads(app.world_mut());
    }
    {
        use crate::rendering::bevy_backend::{extract_level_resource, fog_material::FogMaterialResources};
        app.sub_app_mut(RenderApp).add_systems(bevy::render::ExtractSchedule, (
            extract_level_resource::<crate::screen_shadow_pass::ScreenShadowFrame>,
            extract_level_resource::<FogMaterialResources>,
            extract_level_resource::<crate::shadow_sampling::ParallelSamplingBuffer>,
            extract_level_resource::<crate::shadow_sampling::LocalSamplingBuffer>,
        ));
    }
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
    let observe_initial_images = std::env::var_os("DH2_DEBUG_OBSERVE_INITIAL_IMAGE_LOAD").is_some_and(|v| v == "1");
    let stop_initial_images = observe_initial_images
        && std::env::var_os("DH2_DEBUG_STOP_INITIAL_IMAGE_LOAD").is_some_and(|v| v == "1");
    if observe_initial_images && mode != Mode::Upload {
        return Err("initial texture phase observation requires GPU asset epochs".into());
    }
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
        if new_host && observe_initial_images {
            app.insert_resource(super::texture_streaming::SharedImages::new(&catalog)?);
        }
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
            step == 0,
            |_| false,
        )?;
        let image_budget = observe_initial_images.then(|| super::loading_screen::initial_image_batch_bytes(app.world())).flatten();
        let base_reader = super::scene_preparation::ScenePreparation::new(app.world_mut())
            .prepare(&mut prepared, image_budget, || false)?;
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
            if std::env::var_os("DH2_DEBUG_VISIBILITY_SCRATCH").is_some_and(|value| value == "1") {
                super::visibility_queries::observe_query_scratch(app.world_mut())?;
            }
            if std::env::var_os("DH2_DEBUG_MANIFEST_RETIREMENT").is_some_and(|value| value == "1") {
                crate::loading::observe_descriptor_release();
            }
            if std::env::var_os("DH2_DEBUG_LEVEL_POINT_QUERIES").is_some_and(|value| value == "1") {
                let physics = app.world().get_resource::<super::physics_scene::PhysicsScene>()
                    .ok_or("published physics query owner is absent")?;
                let first = physics.point_queries()?;
                let second = physics.point_queries()?;
                eprintln!("LEVEL_POINT_QUERY_LEASE shared={}", std::sync::Arc::ptr_eq(&first, &second));
                first.observe_queries()?;
            }
            if matches!(mode, Mode::Upload | Mode::RuntimeFx(_)) {
                // Only asset upload/removal is observed. No camera produces a
                // view, and no gameplay or audio systems were registered.
                let mut cameras = app.world_mut().query::<&mut Camera>();
                for mut camera in cameras.iter_mut(app.world_mut()) {
                    camera.is_active = false;
                }
                if observe_initial_images {
                    if stop_initial_images && base_reader.is_none() {
                        return Err("initial texture close observation requires bounded reads".into());
                    }
                    let closed = std::sync::atomic::AtomicBool::new(false);
                    let ready_common = |world: &World| {
                        let scene = world.resource::<SceneRenderQueue>();
                        let shared = world.resource::<super::texture_streaming::SharedImages>();
                        scene.diffuse_image_paths.iter().enumerate()
                            .filter(|(slot, path)| shared.contains(path) && scene.diffuse_mip_states[*slot].resident.is_some())
                            .count()
                    };
                    let ready_before = ready_common(app.world());
                    std::thread::scope(|scope| {
                        if let Some(reader) = base_reader {
                            let closed = &closed;
                            scope.spawn(move || reader.read(|| closed.load(std::sync::atomic::Ordering::Acquire)));
                        }
                        let result = super::loading_screen::observe_initial_image_phases(
                            &mut app, &catalog, stop_initial_images, render_epoch);
                        closed.store(true, std::sync::atomic::Ordering::Release);
                        // A failed render epoch must wake a waiting sender
                        // before the scope joins the installed reader.
                        app.world_mut().remove_resource::<super::texture_streaming::PreparedBaseImages>();
                        result
                    })?;
                    if stop_initial_images { eprintln!("INITIAL_TEXTURE_READER_DRAINED step={step}"); }
                    let resident = ready_common(app.world());
                    eprintln!("INITIAL_TEXTURE_COMMON step={step} before={ready_before} resident={resident}");
                }
                let inputs = gpu_inputs(app.world());
                acquire_water_images(app.world(), step)?;
                upload_assets(&mut app, &inputs)?;
                gpu_snapshot(&app, step, "uploaded");
                if let Some(lighting) = app.world().get_resource::<super::model_motion::ModelProbeLighting>() {
                    let id = lighting.asset_id();
                    let cpu = app.world().resource::<Assets<ShaderBuffer>>().get(id)
                        .and_then(|buffer| buffer.data.as_ref()).map_or(0, Vec::len);
                    let gpu = app.sub_app(RenderApp).world().resource::<RenderAssets<GpuShaderBuffer>>().get(id)
                        .map_or(0, |buffer| buffer.buffer.size());
                    eprintln!("INSTANCE_PROBE_UPLOAD step={step} cpu_upload_bytes={cpu} gpu_bytes={gpu}");
                }
                if std::env::var_os("DH2_DEBUG_SCREEN_SHADOW_PROGRAMS").is_some_and(|value| value == "1") {
                    let state = app.world().resource::<super::shadow_updates::ScreenShadowSource>().producer_state()?;
                    crate::screen_shadow_pass::observe_program_reuse(app.sub_app_mut(RenderApp).world_mut(), state)?;
                }
            }
            if let Mode::RuntimeFx(effects) = mode {
                inspect_runtime_fx(&mut app, &catalog, effects)?;
            }
            if let Mode::FxRequests(effect) = mode {
                super::usable_effects::inspect_source_requests(app.world_mut(), effect)?;
            }
            let screen_output = matches!(mode, Mode::Upload).then(|| {
                let render = app.sub_app(RenderApp).world();
                let frame = render.get_resource::<crate::screen_shadow_pass::ScreenShadowFrame>()?;
                let output = frame.output.id();
                let uploaded = render.resource::<RenderAssets<GpuImage>>().get(output).is_some();
                eprintln!("SCREEN_SHADOW_OWNER step={step} phase=extracted gpu_image={uploaded}");
                Some(output)
            }).flatten();
            let sampling_assets = if matches!(mode, Mode::Upload)
                && std::env::var_os("DH2_DEBUG_SAMPLING_RETIREMENT").is_some_and(|value| value == "1") {
                let render = app.sub_app(RenderApp).world();
                let fog = render.get_resource::<crate::rendering::bevy_backend::fog_material::FogMaterialResources>()
                    .ok_or("fog material resources were not extracted")?;
                let parallel = render.get_resource::<crate::shadow_sampling::ParallelSamplingBuffer>()
                    .ok_or("parallel sampling buffer was not extracted")?;
                let local = render.get_resource::<crate::shadow_sampling::LocalSamplingBuffer>()
                    .ok_or("local sampling buffer was not extracted")?;
                let images = [fog.world.id(), fog.canvas.id(), fog.opaque_depth.id(), fog.curve.id(), parallel.unavailable_depth.id()];
                let buffers = [fog.parameters.id(), fog.objects.id(), parallel.buffer.id(), local.buffer.id()];
                report_sampling_assets(&app, step, "published", &images, &buffers);
                Some((images, buffers))
            } else { None };
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
                if let Some((images, buffers)) = sampling_assets {
                    report_sampling_assets(&app, step, "released", &images, &buffers);
                }
                if let Some(output) = screen_output {
                    let render = app.sub_app(RenderApp).world();
                    let frame = render.contains_resource::<crate::screen_shadow_pass::ScreenShadowFrame>();
                    let gpu_image = render.resource::<RenderAssets<GpuImage>>().get(output).is_some();
                    let cpu_image = app.world().resource::<Assets<Image>>().get(output).is_some();
                    eprintln!("SCREEN_SHADOW_OWNER step={step} phase=released extracted={frame} cpu_image={cpu_image} gpu_image={gpu_image}");
                }
                gpu_snapshot(&app, step, "after-gpu-removal");
                resource_snapshot(app.world(), step, "after-gpu-removal");
            }
        } else {
            prepared.keep_registered_resources();
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

fn report_sampling_assets(app: &App, step: usize, phase: &str, images: &[AssetId<Image>], buffers: &[AssetId<ShaderBuffer>]) {
    let render = app.sub_app(RenderApp).world();
    let fog = render.contains_resource::<crate::rendering::bevy_backend::fog_material::FogMaterialResources>();
    let parallel = render.contains_resource::<crate::shadow_sampling::ParallelSamplingBuffer>();
    let local = render.contains_resource::<crate::shadow_sampling::LocalSamplingBuffer>();
    let cpu_images = images.iter().filter(|id| app.world().resource::<Assets<Image>>().contains(**id)).count();
    let cpu_buffers = buffers.iter().filter(|id| app.world().resource::<Assets<ShaderBuffer>>().contains(**id)).count();
    let gpu_images = images.iter().filter(|id| render.resource::<RenderAssets<GpuImage>>().get(**id).is_some()).count();
    let gpu_buffers = buffers.iter().filter(|id| render.resource::<RenderAssets<GpuShaderBuffer>>().get(**id).is_some()).count();
    eprintln!("SAMPLING_ASSET_LIFETIME step={step} phase={phase} fog={fog} parallel={parallel} local={local} cpu_images={cpu_images} cpu_buffers={cpu_buffers} gpu_images={gpu_images} gpu_buffers={gpu_buffers}");
}
