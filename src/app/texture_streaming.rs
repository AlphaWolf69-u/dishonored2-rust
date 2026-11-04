//! Texture streaming.
mod settings;
mod residency;
pub(super) mod particle_images;
pub(super) mod environment_images;
use crate::rendering::bevy_backend::asset_preparation::AssetWrite;
use super::ImageAccess;
use super::{
    Arc, AssetId, DecalMaterial, Extent3d, GpuImage, ImageAddressMode, ImageSampler,
    ImageSamplerDescriptor, MaterialHandle, Mutex, ProbeMaterial, Receiver, RenderAssetUsages,
    RenderAssets, RenderDevice, RenderQueue, SceneRenderQueue, StreamingDiagnostics,
    TexelCopyBufferLayout, TextureDimension, TextureFormat, VecDeque, archive, entities, material,
    mpsc, render_state, surface_gpu, texture,
};
use bevy::prelude::*;

/// Immutable post-process images use the same installed-image reader and mip
/// layout as streamed materials, with the complete initial chain resident.
pub(super) fn load_resident_2d(
    catalog: &archive::Catalog, name: &str, images: &mut dyn AssetWrite<Image>,
    resident: &super::resident_images::ResidentImages,
) -> Result<Handle<Image>, String> {
    resident.load(catalog, name, images)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiffuseMipLevel {
    pub(super) resource_mip: Option<u8>,
    pub(super) next_separate_suffix: Option<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) format: texture::ImageFormat,
}

#[derive(Clone, Default)]
pub(super) struct DiffuseMipState {
    pub(super) resident: Option<DiffuseMipLevel>,
    pub(super) pending: Option<DiffuseMipLevel>,
    pub(super) minimum: Option<DiffuseMipLevel>,
    pub(super) minimum_mip_count: u32,
    pub(super) pending_reduction: bool,
    pub(super) unavailable_finer: Option<u8>,
    /// 14030f3d0's +0x3310 counts finer-mip bytes, not base-chain storage.
    pub(super) streamed_bytes: u64,
    pub(super) reserved_bytes: u64,
    pub(super) sampling_priority: u8,
    pub(super) idle_frames: u32,
    pub(super) path_priority: u16,
}

/// Explicit init/common image references survive world destruction. Capture
/// the current streamed identity at a map boundary, never a second base copy.
/// No registry lookup or lock is added to ordinary gameplay streaming.
#[derive(Resource, Default, Clone)]
pub(super) struct SharedImages {
    paths: std::collections::HashSet<String>,
    resident: std::collections::HashMap<String, (Option<u8>, Handle<Image>, DiffuseMipState)>,
    pending: std::collections::HashMap<AssetId<Image>, residency::SharedReplacement>,
}

impl SharedImages {
    pub(super) fn contains(&self, path: &str) -> bool { self.paths.contains(path) }

    pub(super) fn can_reuse(&self, path: &str, preferred: Option<u8>) -> bool {
        self.resident.get(path).is_some_and(|(mip, _, _)| *mip == preferred)
    }

    pub(super) fn current_view(&self, path: &str) -> Option<(Handle<Image>, DiffuseMipLevel)> {
        let (_, handle, state) = self.resident.get(path)?;
        Some((handle.clone(), state.resident?))
    }

    pub fn new(catalog: &archive::Catalog) -> Result<Self, String> {
        let language = crate::assets::lang::read_text_language(catalog.base())?;
        let mut paths = std::collections::HashSet::new();
        for name in ["init", "common"] {
            let list = crate::assets::map_resources::acquire_list(
                catalog.base(),
                catalog,
                name,
                language,
            )?;
            paths.extend(
                list.iter()
                    .filter(|r| r.kind == "image")
                    .map(|r| r.path.clone()),
            );
        }
        Ok(Self {
            paths,
            resident: Default::default(),
            pending: Default::default(),
        })
    }

    pub fn capture(&mut self, queue: &SceneRenderQueue) {
        // Publication reset cancels outstanding backend replacements. Keep
        // the last committed source, including its original byte charge.
        self.pending.clear();
        for (_, _, state) in self.resident.values_mut() {
            state.pending = None;
            state.pending_reduction = false;
        }
        let mut count = 0;
        for (slot, path) in queue.diffuse_image_paths.iter().enumerate() {
            if !self.paths.contains(path) {
                continue;
            }
            let (Some(handle), state) = (
                &queue.diffuse_image_handles[slot],
                &queue.diffuse_mip_states[slot],
            ) else {
                continue;
            };
            if state.resident.is_none() {
                continue;
            }
            let mut state = state.clone();
            state.pending = None;
            state.pending_reduction = false;
            state.reserved_bytes = 0;
            state.path_priority = residency::path_priority(path);
            state.idle_frames = queue.texture_stream_frame.saturating_sub(queue.diffuse_image_last_visible[slot]).min(0xffff) as u32;
            self.resident
                .insert(path.clone(), (queue.preferred_mip, handle.clone(), state));
            count += 1;
        }
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!(
                "LOAD shared scene images: {count} captured, {} retained",
                self.resident.len()
            );
        }
    }
}

pub(super) fn reuse_shared_image(
    shared: &mut SharedImages,
    slot: usize,
    queue: &mut SceneRenderQueue,
    images: &mut Assets<Image>,
    probe_materials: &mut Assets<ProbeMaterial>,
    decal_materials: &mut Assets<DecalMaterial>,
    sky_materials: &mut Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
) -> bool {
    let Some((_, handle, mut state)) = shared
        .resident
        .remove(&queue.diffuse_image_paths[slot])
        .filter(|(preferred, _, _)| *preferred == queue.preferred_mip)
    else {
        return false;
    };
    attach_prepared_diffuse_image(
        slot,
        handle.clone(),
        queue,
        images,
        probe_materials,
        decal_materials,
        sky_materials,
    );
    state.sampling_priority = state.sampling_priority.max(queue.diffuse_mip_states[slot].sampling_priority);
    queue.diffuse_image_demands[slot].raise_priority(state.sampling_priority);
    queue.diffuse_mip_states[slot] = state;
    queue.diffuse_image_attempted[slot] = true;
    true
}

/// Borrow the image owner and its material consumers together at a load boundary.
fn with_scene_images<R>(
    world: &mut World,
    f: impl FnOnce(
        &mut SceneRenderQueue,
        &mut Assets<Image>,
        &mut Assets<ProbeMaterial>,
        &mut Assets<DecalMaterial>,
        &mut Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
        &mut SharedImages,
    ) -> R,
) -> R {
    world.resource_scope(|world, mut scene: Mut<SceneRenderQueue>| {
        world.resource_scope(|world, mut images: Mut<Assets<Image>>| {
            world.resource_scope(|world, mut probes: Mut<Assets<ProbeMaterial>>| {
                world.resource_scope(|world, mut decals: Mut<Assets<DecalMaterial>>| {
                    world.resource_scope(
                        |world,
                         mut skies: Mut<
                            Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
                        >| {
                            let mut shared = world.resource_mut::<SharedImages>();
                            f(
                                &mut scene,
                                &mut images,
                                &mut probes,
                                &mut decals,
                                &mut skies,
                                &mut shared,
                            )
                        },
                    )
                })
            })
        })
    })
}

/// A registered image identity can precede its payload (1400607f0 /
/// 14005feb0's placeholder entries). Bevy then retries dependent materials,
/// rather than preparing them against temporary fallback bindings.
pub(super) fn reserve_scene_images(world: &mut World) {
    let mut prepared = world.remove_resource::<PreparedBaseImages>().unwrap_or_default();
    with_scene_images(world, |scene, images, probes, decals, skies, shared| {
        for slot in 0..scene.diffuse_image_paths.len() {
            if reuse_shared_image(shared, slot, scene, images, probes, decals, skies) {
                continue;
            }
            if let Some(view) = prepared.views.remove(&slot) {
                attach_prepared_diffuse_image(slot, view.handle.clone(), scene, images, probes, decals, skies);
                commit_prepared_diffuse_level(scene, slot, view.level, view.mip_count);
                scene.diffuse_image_attempted[slot] = true;
                continue;
            }
            let handle = images.reserve_handle();
            attach_prepared_diffuse_image(slot, handle, scene, images, probes, decals, skies);
        }
    });
    world.insert_resource(prepared);
}

/// 1400661e0 acquires init/common resources before map resources. Keep those
/// upload phases distinct as well: persistent textures must not be scattered
/// among disposable map allocations in the backend's memory blocks.
pub(super) fn initialize_scene_images(
    world: &mut World,
    catalog: &archive::Catalog,
    common: bool,
) -> Result<Vec<AssetId<Image>>, String> {
    let mut prepared = world.remove_resource::<PreparedBaseImages>().unwrap_or_default();
    let result =     with_scene_images(world, |scene, images, probes, decals, skies, shared| {
        let mut ids = Vec::new();
        let (mut loaded, mut bytes, mut reused) = (0, 0, 0);
        for slot in 0..scene.diffuse_image_paths.len() {
            if shared.paths.contains(&scene.diffuse_image_paths[slot]) != common {
                continue;
            }
            let resident = scene.diffuse_mip_states[slot].resident.is_some();
            let size =
                if let Some(streamed) = prepared.payloads.remove(&slot) {
                    commit_resident_image(streamed, slot, scene, images, probes, decals, skies)?
                } else {
                    initialize_resident_image(catalog, slot, scene, images, probes, decals, skies)?
                };
            if resident {
                reused += 1;
            } else {
                loaded += 1;
                bytes += size;
            }
            if let Some(handle) = &scene.diffuse_image_handles[slot] {
                ids.push(handle.id());
            }
        }
        info!(target: "dh2::textures", common, resident_images=loaded, resident_bytes=bytes, reused_images=reused, "Scene base mips initialized");
        Ok(ids)
    });
    if !prepared.payloads.is_empty() { world.insert_resource(prepared); }
    result
}

#[derive(Resource, Default)]
pub(super) struct PreparedBaseImages {
    payloads: std::collections::HashMap<usize, StreamedDiffuse>,
    views: std::collections::HashMap<usize, Arc<super::resident_images::ResidentImage>>,
}

pub(super) fn prepare_base_images(catalog: &archive::Catalog, queue: &SceneRenderQueue, shared: &SharedImages,
    resident: &super::resident_images::ResidentImages, inline_slots: &std::collections::HashSet<usize>) -> Result<PreparedBaseImages, String> {
    let mut result = PreparedBaseImages { payloads: Default::default(),
        views: resident.scene_views(catalog, &queue.diffuse_image_paths, queue.preferred_mip, inline_slots) };
    let mut reused_bytes = 0usize;
    let mut reused = 0;
    for (slot, path) in queue.diffuse_image_paths.iter().enumerate() {
        if shared.can_reuse(path, queue.preferred_mip) { continue; }
        if let Some(view) = result.views.get(&slot) {
            reused += 1;
            if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
                eprintln!("MEMORY scene registered view: path={path} level={:?} mips={}", view.level, view.mip_count);
            }
            reused_bytes += (0..view.mip_count).map(|mip| view.level.format.mip_bytes(
                (view.level.width >> mip).max(1), (view.level.height >> mip).max(1))).sum::<usize>();
            continue;
        }
        let preferred = if inline_slots.contains(&slot) { None } else { queue.preferred_mip };
        let mut streamed = read_resident_diffuse_image(catalog, path, preferred)?
            .ok_or_else(|| format!("resident image is absent: {path}"))?;
        if !streamed.uploads.is_empty() {
            streamed.image.data = Some(std::mem::take(&mut streamed.uploads).into_iter().flat_map(|mip| mip.payload).collect());
        }
        result.payloads.insert(slot, streamed);
    }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY scene registered views: reused={reused} reused_bytes={reused_bytes} prepared={}", result.payloads.len());
    }
    Ok(result)
}

pub(super) struct StreamedDiffuse {
    pub(super) image: Image,
    pub(super) uploads: Vec<CompressedMipUpload>,
    pub(super) level: DiffuseMipLevel,
}

pub(super) struct CompressedMipUpload {
    pub(super) mip_level: u32,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) payload: Vec<u8>,
}

// Registered executable defaults: r_textureStreamingPerFrameBudgetKB and
// r_maxFramesBeforeTextureUnload. Bevy uploads changed images after this
// system, so the byte limit bounds staged payloads, not completed GPU work.
pub(super) const TEXTURE_UPLOAD_BUDGET_KB: f32 = 4096.0;
pub(super) const TEXTURE_REDUCE_AFTER_FRAMES: u64 = 60;
pub(super) const TEXTURE_READ_QUEUE_CAPACITY: usize = 16;

#[derive(Default)]
pub(super) struct PreparedImageState {
    pub(super) pending: Vec<PendingGpuImageUpload>,
    pub(super) completed: Vec<AssetId<Image>>,
    pub(super) failed: Vec<(AssetId<Image>, String)>,
    pub(super) generation: u64,
}

pub(super) struct PendingGpuImageUpload {
    pub(super) id: AssetId<Image>,
    pub(super) previous: Option<AssetId<Image>>,
    pub(super) copy: GpuMipCopy,
    pub(super) uploads: Vec<CompressedMipUpload>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum GpuMipCopy {
    None,
    Extend,
    Reduce { dropped: u32 },
}

#[derive(Resource, Clone, Default)]
pub(super) struct PreparedImageMailbox(pub(super) Arc<Mutex<PreparedImageState>>);

#[derive(Default)]
pub(super) struct UploadStats {
    frames: u64,
    uploads: u64,
    bytes: u64,
    deferred: u64,
    peak_frame_bytes: u64,
    peak_job_bytes: u64,
}

/// Host staging metadata only: drained jobs and their payloads remain owned by
/// the mailbox or are dropped after upload, never by these empty buffers.
pub(super) struct UploadScratch {
    pending: Vec<PendingGpuImageUpload>,
    waiting: Vec<PendingGpuImageUpload>,
    completed: Vec<AssetId<Image>>,
    failed: Vec<(AssetId<Image>, String)>,
    generation: Option<u64>,
    separate: bool,
    timing: bool,
    waiting_growth_frames: usize,
    completed_growth_frames: usize,
    failed_growth_frames: usize,
}
impl Default for UploadScratch {
    fn default() -> Self {
        Self {
            pending: Vec::new(), waiting: Vec::new(), completed: Vec::new(), failed: Vec::new(),
            generation: None,
            separate: std::env::var("DH2_DEBUG_ALLOCATE_TEXTURE_UPLOAD_SCRATCH").as_deref() == Ok("1"),
            timing: std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"),
            waiting_growth_frames: 0, completed_growth_frames: 0, failed_growth_frames: 0,
        }
    }
}

pub(super) fn report_prepared_diffuse_images(
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mailbox: Res<PreparedImageMailbox>,
    mut settings: Local<settings::SettingsCache>,
    adapter: Option<Res<bevy::render::renderer::RenderAdapter>>,
    mut stats: Local<UploadStats>,
    mut scratch: Local<UploadScratch>,
) {
    let settings = settings.get(adapter.as_deref());
    // 14030d1e0 limits actual uploads, not only game-thread staging.
    let mut budget = texture::MipUploadBudget::new(settings.upload_kb);
    let mut frame_bytes = 0u64;
    if scratch.separate {
        // Same-build reference for the former per-frame allocation path.
        scratch.pending = Vec::new();
        scratch.waiting = Vec::new();
        scratch.completed = Vec::new();
        scratch.failed = Vec::new();
    }
    let UploadScratch { pending, waiting, completed, failed, generation, separate, timing,
        waiting_growth_frames, completed_growth_frames, failed_growth_frames } = &mut *scratch;
    {
        let mut state = mailbox.0.lock().unwrap();
        if *generation != Some(state.generation) {
            if *timing && generation.is_some() {
                eprintln!("MEMORY texture upload scratch map release: generation={} capacities={}/{}/{}/{} empty={}/{}/{}/{} growth_frames={}/{}/{} separate={}",
                    state.generation, pending.capacity(), waiting.capacity(), completed.capacity(), failed.capacity(),
                    pending.len(), waiting.len(), completed.len(), failed.len(),
                    waiting_growth_frames, completed_growth_frames, failed_growth_frames, separate);
            }
            // A mailbox reset follows outgoing-render acknowledgement. These
            // empty vectors are map scratch, so do not retain peak capacity.
            *pending = Vec::new();
            *waiting = Vec::new();
            *completed = Vec::new();
            *failed = Vec::new();
            *generation = Some(state.generation);
            *waiting_growth_frames = 0;
            *completed_growth_frames = 0;
            *failed_growth_frames = 0;
        }
        std::mem::swap(&mut state.pending, pending);
    }
    let capacities = (waiting.capacity(), completed.capacity(), failed.capacity());
    for job in pending.drain(..) {
        if !budget.allows_next_upload() {
            stats.deferred += 1;
            waiting.push(job);
            continue;
        }
        let Some(image) = gpu_images.get(job.id) else {
            waiting.push(job);
            continue;
        };
        let previous = match job.previous {
            Some(id) => match gpu_images.get(id) {
                Some(previous) => Some(previous),
                None => {
                    failed.push((job.id, "previous resident GPU image is unavailable".into()));
                    continue;
                }
            },
            None => None,
        };
        match upload_compressed_mips(
            &render_device,
            &render_queue,
            image,
            previous,
            job.copy,
            &job.uploads,
        ) {
            Ok(()) => {
                let bytes = job.uploads.iter().map(|mip| mip.payload.len() as u64).sum::<u64>();
                budget.record_upload(bytes as i32);
                stats.uploads += 1;
                stats.bytes += bytes;
                stats.peak_job_bytes = stats.peak_job_bytes.max(bytes);
                frame_bytes += bytes;
                completed.push(job.id);
            },
            Err(error) => failed.push((job.id, error)),
        }
    }
    let mut state = mailbox.0.lock().unwrap();
    // Older deferred jobs precede jobs staged while this batch ran.
    waiting.append(&mut state.pending);
    if *timing {
        *waiting_growth_frames += usize::from(waiting.capacity() > capacities.0);
        *completed_growth_frames += usize::from(completed.capacity() > capacities.1);
        *failed_growth_frames += usize::from(failed.capacity() > capacities.2);
    }
    std::mem::swap(&mut state.pending, waiting);
    state.completed.extend(completed.drain(..));
    state.failed.extend(failed.drain(..));
    drop(state);
    stats.frames += 1;
    stats.peak_frame_bytes = stats.peak_frame_bytes.max(frame_bytes);
    if stats.frames % 600 == 0 && *timing {
        eprintln!("MEMORY texture upload scratch: frames={} separate={} growth_frames={}/{}/{} empty={}/{}/{}/{}",
            stats.frames, separate, waiting_growth_frames, completed_growth_frames, failed_growth_frames,
            pending.len(), waiting.len(), completed.len(), failed.len());
    }
    if stats.frames % 600 == 0 && std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY texture GPU uploads: frames={} jobs={} upload_bytes={} deferred_jobs={} peak_frame_bytes={} peak_job_bytes={} budget_kb={}",
            stats.frames, stats.uploads, stats.bytes, stats.deferred, stats.peak_frame_bytes, stats.peak_job_bytes, settings.upload_kb);
    }
}

/// FUN_1402f4990 copies resident levels to the newly sized texture; the
/// FUN_14030d1e0 upload path writes only the requested BC7 level afterward.
pub(super) fn upload_compressed_mips(
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
    image: &GpuImage,
    previous: Option<&GpuImage>,
    copy: GpuMipCopy,
    uploads: &[CompressedMipUpload],
) -> Result<(), String> {
    let descriptor = &image.texture_descriptor;
    let block_bytes =
        streamed_block_bytes(descriptor.format).ok_or("unsupported streamed GPU image format")?;
    let (block_width, block_height) = descriptor.format.block_dimensions();
    let (source_first, target_first, copy_count) = match (copy, previous) {
        (GpuMipCopy::None, None) if !uploads.is_empty() => (0, 0, 0),
        (GpuMipCopy::Extend, Some(previous))
            if previous.texture_descriptor.format == descriptor.format
                && descriptor.mip_level_count
                    == previous.texture_descriptor.mip_level_count + 1
                && (descriptor.size.width / 2).max(1) == previous.texture_descriptor.size.width
                && (descriptor.size.height / 2).max(1)
                    == previous.texture_descriptor.size.height
                && uploads.len() == 1
                && uploads[0].mip_level == 0 =>
        {
            (0, 1, previous.texture_descriptor.mip_level_count)
        }
        (GpuMipCopy::Reduce { dropped }, Some(previous))
            if dropped > 0
                && dropped < 32
                && uploads.is_empty()
                && previous.texture_descriptor.format == descriptor.format
                && descriptor.mip_level_count + dropped
                    == previous.texture_descriptor.mip_level_count
                && descriptor.size.width
                    == (previous.texture_descriptor.size.width >> dropped).max(1)
                && descriptor.size.height
                    == (previous.texture_descriptor.size.height >> dropped).max(1) =>
        {
            (dropped, 0, descriptor.mip_level_count)
        }
        _ => return Err("streamed GPU mip transfer does not match resident chain".into()),
    };
    for upload in uploads {
        let width = (descriptor.size.width >> upload.mip_level).max(1);
        let height = (descriptor.size.height >> upload.mip_level).max(1);
        let expected = width.div_ceil(block_width) as usize
            * height.div_ceil(block_height) as usize
            * block_bytes;
        if upload.mip_level >= descriptor.mip_level_count
            || upload.width != width
            || upload.height != height
            || upload.payload.len() != expected
        {
            return Err("streamed compressed payload does not match GPU mip extent".into());
        }
    }
    if let Some(previous) = previous {
        let mut encoder = render_device.create_command_encoder(&Default::default());
        for index in 0..copy_count {
            let source_level = source_first + index;
            let mut source = previous.texture.as_image_copy();
            source.mip_level = source_level;
            let mut destination = image.texture.as_image_copy();
            destination.mip_level = target_first + index;
            encoder.copy_texture_to_texture(
                source,
                destination,
                Extent3d {
                    width: (previous.texture_descriptor.size.width >> source_level).max(1),
                    height: (previous.texture_descriptor.size.height >> source_level).max(1),
                    depth_or_array_layers: 1,
                },
            );
        }
        render_queue.submit([encoder.finish()]);
    }
    for upload in uploads {
        let mut destination = image.texture.as_image_copy();
        destination.mip_level = upload.mip_level;
        render_queue.write_texture(
            destination,
            &upload.payload,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(upload.width.div_ceil(block_width) * block_bytes as u32),
                rows_per_image: Some(upload.height.div_ceil(block_height)),
            },
            Extent3d {
                width: upload.width,
                height: upload.height,
                depth_or_array_layers: 1,
            },
        );
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
pub(super) enum TextureReadTarget {
    Initial(Option<u8>),
    Exact(u8, texture::ImageFormat),
}

pub(super) type TextureReadResult = (
    usize,
    TextureReadTarget,
    Result<Option<StreamedDiffuse>, String>,
);

type TextureReadRequest = (usize, String, TextureReadTarget);

/// Requests read one at a time in submission order. A reader task runs on
/// Bevy's I/O pool only while requests are pending, so no pool thread stays
/// blocked waiting for work.
#[derive(Default)]
struct TextureReadQueue {
    pending: VecDeque<TextureReadRequest>,
    reading: bool,
    cancelled: bool,
}

#[derive(Resource)]
pub(super) struct TextureReadWorker {
    catalog: Arc<archive::Catalog>,
    queue: Arc<Mutex<TextureReadQueue>>,
    completed_tx: mpsc::Sender<TextureReadResult>,
    pub(super) completed: Mutex<Receiver<TextureReadResult>>,
    pub(super) ready: VecDeque<TextureReadResult>,
    pub(super) inflight: Vec<bool>,
    pub(super) inflight_count: usize,
}

impl Drop for TextureReadWorker {
    fn drop(&mut self) {
        // Map teardown cancels queued acquisitions. A blocking archive read
        // already executing can finish, but may neither publish nor start
        // another outgoing-map request after the owner has been released.
        let mut queue = self.queue.lock().unwrap();
        queue.cancelled = true;
        queue.pending.clear();
    }
}

impl TextureReadWorker {
    /// The installed archives, for on-demand diagnostic reads.
    pub(super) fn catalog(&self) -> &archive::Catalog {
        &self.catalog
    }

    pub(super) fn new(catalog: Arc<archive::Catalog>, image_count: usize) -> Self {
        let (completed_tx, completed) = mpsc::channel();
        Self {
            catalog,
            queue: Arc::default(),
            completed_tx,
            completed: Mutex::new(completed),
            ready: VecDeque::new(),
            inflight: vec![false; image_count],
            inflight_count: 0,
        }
    }

    /// Queues a read; `Err` returns the request when the queue is full.
    fn request(&self, request: TextureReadRequest) -> Result<(), TextureReadRequest> {
        let mut queue = self.queue.lock().unwrap();
        if queue.pending.len() >= TEXTURE_READ_QUEUE_CAPACITY {
            return Err(request);
        }
        queue.pending.push_back(request);
        if std::mem::replace(&mut queue.reading, true) {
            return Ok(());
        }
        drop(queue);
        let (catalog, queue, completed_tx) = (
            self.catalog.clone(),
            self.queue.clone(),
            self.completed_tx.clone(),
        );
        bevy::tasks::IoTaskPool::get()
            .spawn(async move {
                let _tag = crate::platform::allocation_tags::tag("texture worker");
                loop {
                    let Some((slot, stem, target)) = ({
                        let mut queue = queue.lock().unwrap();
                        if queue.cancelled { return; }
                        let next = queue.pending.pop_front();
                        queue.reading = next.is_some();
                        next
                    }) else {
                        return;
                    };
                    let result = read_streamed_diffuse_at_target(&catalog, &stem, target);
                    if queue.lock().unwrap().cancelled { return; }
                    if completed_tx.send((slot, target, result)).is_err() {
                        return;
                    }
                }
            })
            .detach();
        Ok(())
    }
}

pub(super) fn read_streamed_diffuse_image(
    catalog: &archive::Catalog,
    stem: &str,
    preferred_mip: u8,
) -> Result<Option<StreamedDiffuse>, String> {
    read_streamed_diffuse_at_target(
        catalog,
        stem,
        TextureReadTarget::Initial(Some(preferred_mip)),
    )
}

pub(super) fn read_streamed_diffuse_at_target(
    catalog: &archive::Catalog,
    stem: &str,
    target: TextureReadTarget,
) -> Result<Option<StreamedDiffuse>, String> {
    read_diffuse_image(catalog, stem, target, false)
}

/// Whole-image consumers own one final packed payload; streamed jobs above
/// retain independent mip buffers for their cross-thread queue uploads.
pub(super) fn read_resident_diffuse_image(catalog: &archive::Catalog, stem: &str,
    preferred: Option<u8>) -> Result<Option<StreamedDiffuse>, String> {
    static MODE: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    let (split, trace) = *MODE.get_or_init(|| (
        std::env::var("DH2_DEBUG_SPLIT_RESIDENT_MIPS").as_deref() == Ok("1"),
        std::env::var("DH2_DEBUG_IMAGE_PACKING").as_deref() == Ok("1"),
    ));
    let result = read_diffuse_image(catalog, stem, TextureReadTarget::Initial(preferred), !split)?;
    if trace {
        if let Some(image) = &result {
            let mut crc = flate2::Crc::new();
            let bytes = if let Some(data) = &image.image.data {
                crc.update(data); data.len()
            } else {
                for mip in &image.uploads { crc.update(&mip.payload); }
                image.uploads.iter().map(|mip|mip.payload.len()).sum()
            };
            let descriptor = &image.image.texture_descriptor;
            eprintln!("MEMORY image packing: path={stem} preferred={preferred:?} mip_buffers={} mip_buffer_bytes={} final_bytes={bytes} extent={}x{} mips={} format={:?} checksum={:08x}",
                image.uploads.len(), image.uploads.iter().map(|mip|mip.payload.len()).sum::<usize>(),
                descriptor.size.width, descriptor.size.height, descriptor.mip_level_count, descriptor.format, crc.sum());
        }
    }
    Ok(result)
}

fn read_diffuse_image(catalog: &archive::Catalog, stem: &str, target: TextureReadTarget,
    packed: bool) -> Result<Option<StreamedDiffuse>, String> {
    if let Some(payload) = material::intrinsic_image_pixels(stem) {
        if matches!(target, TextureReadTarget::Exact(..)) {
            return Err("intrinsic image has no streamed finer mip".into());
        }
        let mut image = empty_streamed_image(16, 16, 1, TextureFormat::Rgba8Unorm);
        let uploads = if packed { image.data = Some(payload); Vec::new() }
            else {vec![CompressedMipUpload {mip_level: 0, width: 16, height: 16, payload}]};
        return Ok(Some(StreamedDiffuse {
            image,
            uploads,
            level: DiffuseMipLevel {
                resource_mip: None,
                next_separate_suffix: None,
                width: 16,
                height: 16,
                format: texture::ImageFormat::Rgba8,
            },
        }));
    }
    if let TextureReadTarget::Exact(mip, format) = target {
        let path = format!("{stem}_mip{mip}");
        let Some(bytes) = catalog.read(&path).map_err(|error| error.to_string())? else {
            return Ok(None);
        };
        let compressed = texture::compressed_mip(&bytes, u32::from(mip), format)?;
        let uploads = vec![CompressedMipUpload {
            mip_level: 0,
            width: compressed.width,
            height: compressed.height,
            payload: compressed.payload.to_vec(),
        }];
        return Ok(Some(StreamedDiffuse {
            image: empty_streamed_image(
                compressed.width,
                compressed.height,
                1,
                gpu_image_format(format),
            ),
            uploads,
            level: DiffuseMipLevel {
                resource_mip: Some(mip),
                next_separate_suffix: mip.checked_sub(1),
                width: compressed.width,
                height: compressed.height,
                format,
            },
        }));
    }
    let Some(bytes) = catalog.read(stem).map_err(|error| error.to_string())? else {
        return Err(format!("missing base BIM format descriptor {stem}"));
    };
    // Host cross-thread buffers still own their bytes. Borrow validated source
    // records instead of first building and then splitting a packed payload.
    static STAGING: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    let (copy_inline, report_staging) = *STAGING.get_or_init(|| (
        std::env::var("DH2_DEBUG_COPY_INLINE_MIPS").as_deref() == Ok("1"),
        std::env::var("DH2_DEBUG_IMAGE_STAGING").as_deref() == Ok("1"),
    ));
    let copied = if copy_inline { Some(texture::compressed_inline_chain(&bytes)?) } else { None };
    let mut inline_mips = [None; 16];
    let inline = if let Some(chain) = &copied {
        let mut offset = 0;
        for index in 0..chain.info.mip_count {
            let width = (chain.info.width >> index).max(1);
            let height = (chain.info.height >> index).max(1);
            let size = chain.info.format.mip_bytes(width, height);
            inline_mips[index as usize] = Some(texture::CompressedMip {
                width, height, payload: &chain.payload[offset..offset + size],
            });
            offset += size;
        }
        chain.info
    } else {
        texture::visit_inline_mips(&bytes, |index, mip| inline_mips[index as usize] = Some(mip))?
    };
    let format = inline.format;
    let highest_separate = (0..=15)
        .rev()
        .find(|mip| catalog.contains(&format!("{stem}_mip{mip}")));
    let selected_separate = match target {
        TextureReadTarget::Initial(None) => None,
        TextureReadTarget::Initial(Some(preferred)) => (0..=preferred)
            .rev()
            .find(|mip| catalog.contains(&format!("{stem}_mip{mip}"))),
        TextureReadTarget::Exact(_, _) => unreachable!(),
    };
    let mut uploads = Vec::new();
    let mut top = None;
    let mut previous: Option<(u32, u32)> = None;
    let mut mip_count = 0;
    let total_levels = inline.mip_count + selected_separate
        .map_or(0, |selected| u32::from(highest_separate.unwrap() - selected) + 1);
    let mut packed_payload = Vec::new();
    let mut emit = |mip_level: u32, width: u32, height: u32, payload: &[u8]| {
        if packed {
            if packed_payload.capacity() == 0 {
                let bytes = (0..total_levels).map(|level|
                    format.mip_bytes((width >> level).max(1), (height >> level).max(1))).sum();
                packed_payload.reserve_exact(bytes);
            }
            packed_payload.extend_from_slice(payload);
        } else {
            uploads.push(CompressedMipUpload {mip_level, width, height, payload: payload.to_vec()});
        }
    };
    if let Some(selected) = selected_separate {
        let last = highest_separate.ok_or("selected image mip has no lower levels")?;
        for mip in selected..=last {
            let path = format!("{stem}_mip{mip}");
            let bytes = catalog
                .read(&path)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("missing authored lower mip {path}"))?;
            let compressed = texture::compressed_mip(&bytes, u32::from(mip), format)?;
            if let Some((width, height)) = previous {
                if compressed.width != (width / 2).max(1)
                    || compressed.height != (height / 2).max(1)
                {
                    return Err("authored separate mip dimensions are not contiguous".into());
                }
            }
            top.get_or_insert((compressed.width, compressed.height));
            previous = Some((compressed.width, compressed.height));
            emit(mip_count, compressed.width, compressed.height, compressed.payload);
            mip_count += 1;
        }
    }
    if let Some((width, height)) = previous {
        if inline.width != (width / 2).max(1) || inline.height != (height / 2).max(1) {
            return Err("authored inline base does not follow separate mips".into());
        }
    }
    top.get_or_insert((inline.width, inline.height));
    for (index, mip) in inline_mips.iter().flatten().enumerate() {
        emit(mip_count + index as u32, mip.width, mip.height, mip.payload);
    }
    drop(emit);
    mip_count += inline.mip_count;
    if report_staging && !packed {
        use std::hash::{Hash, Hasher};
        let mut checksum = std::hash::DefaultHasher::new();
        inline.packed_format.hash(&mut checksum);
        for mip in &uploads {
            (mip.mip_level, mip.width, mip.height).hash(&mut checksum);
            mip.payload.hash(&mut checksum);
        }
        let inline_bytes = inline_mips.iter().flatten().map(|mip| mip.payload.len()).sum::<usize>();
        eprintln!("MEMORY image staging: path={stem} target={target:?} inline_bytes={inline_bytes} redundant_copy_bytes={} upload_bytes={} mips={} checksum={:016x}",
            if copy_inline {inline_bytes} else {0}, uploads.iter().map(|mip|mip.payload.len()).sum::<usize>(), uploads.len(), checksum.finish());
    }
    let (width, height) = top.ok_or("authored image has no mip payload")?;
    let mut image = empty_streamed_image(width, height, mip_count, gpu_image_format(format));
    if packed { image.data = Some(packed_payload); }
    Ok(Some(StreamedDiffuse {
        image,
        uploads,
        level: DiffuseMipLevel {
            resource_mip: selected_separate,
            next_separate_suffix: selected_separate
                .and_then(|mip| mip.checked_sub(1))
                .or_else(|| {
                    if selected_separate.is_none() {
                        highest_separate
                    } else {
                        None
                    }
                }),
            width,
            height,
            format,
        },
    }))
}

pub(super) use crate::rendering::bevy_backend::image_formats::gpu_image_format;

pub(super) fn streamed_block_bytes(format: TextureFormat) -> Option<usize> {
    match format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => Some(4),
        TextureFormat::Rg8Unorm | TextureFormat::R16Float | TextureFormat::R16Unorm => Some(2),
        TextureFormat::Rg32Float => Some(8),
        TextureFormat::Bgra8Unorm => Some(4),
        TextureFormat::Bc1RgbaUnorm
        | TextureFormat::Bc1RgbaUnormSrgb
        | TextureFormat::Bc4RUnorm
        | TextureFormat::Bc4RSnorm => Some(8),
        TextureFormat::Bc3RgbaUnorm
        | TextureFormat::Bc3RgbaUnormSrgb
        | TextureFormat::Bc5RgUnorm
        | TextureFormat::Bc5RgSnorm
        | TextureFormat::Bc6hRgbUfloat
        | TextureFormat::Bc6hRgbFloat
        | TextureFormat::Bc7RgbaUnorm
        | TextureFormat::Bc7RgbaUnormSrgb => Some(16),
        _ => None,
    }
}

pub(super) fn surface_sampler_image(catalog: Option<&archive::Catalog>) -> Result<Image, String> {
    // Installed anisotropicSampler declaration; FUN_1402d9300 canonicalizes
    // any anisotropic filter to all three2-valued filter fields.
    let mut own_anisotropy = 4.0f32;
    if let Some(catalog) = catalog {
        let bytes = catalog
            .read("generated/decls/renderparm/anisotropicsampler.decl")
            .map_err(|e| e.to_string())?
            .ok_or("missing native anisotropic sampler")?;
        for line in entities::declaration_text(&bytes).lines() {
            let mut words = line.split_whitespace();
            let Some(key) = words.next() else {
                continue;
            };
            match key.to_ascii_lowercase().as_str() {
                "minfilter" | "magfilter" | "mipfilter" => {
                    if words
                        .next()
                        .is_none_or(|v| !v.eq_ignore_ascii_case("aniso"))
                    {
                        return Err("changed native anisotropic filter declaration".into());
                    }
                }
                "addressu" | "addressv" | "addressw" => {
                    if words.next().is_none_or(|v| !v.eq_ignore_ascii_case("wrap")) {
                        return Err("changed native anisotropic address declaration".into());
                    }
                }
                "maxanisotropy" => {
                    own_anisotropy = words
                        .next()
                        .ok_or("missing native anisotropy")?
                        .parse::<f32>()
                        .map_err(|e| e.to_string())?
                }
                "{" | "}" => {}
                _ => return Err(format!("unsupported sampler field {key}")),
            }
        }
    }
    // With mipFilter=ANISO, native378280 uses the declaration's own limit,
    // independently of image_anisotropy. The descriptor port has768 oracles.
    // Native default MaxLOD DAT_141b1a400 is13.0, verified in installed EXE.
    let descriptor = render_state::sampler_descriptor(
        &[
            0x2a,
            0,
            0,
            0,
            0,
            0,
            0,
            13.0f32.to_bits(),
            own_anisotropy.to_bits(),
        ],
        1.0,
    );
    let anisotropy = u16::try_from(descriptor[5]).map_err(|e| e.to_string())?;
    if !(1..=16).contains(&anisotropy) {
        return Err("anisotropy exceeds device-supported range".into());
    }
    let mut image = Image::new_fill(
        Extent3d::default(),
        TextureDimension::D2,
        &[255; 4],
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    );
    let mut sampler = ImageSamplerDescriptor::linear();
    sampler.set_address_mode(ImageAddressMode::Repeat);
    sampler.anisotropy_clamp = anisotropy;
    sampler.lod_max_clamp = f32::from_bits(descriptor[12]);
    image.sampler = ImageSampler::Descriptor(sampler);
    Ok(image)
}

pub(super) fn empty_streamed_image(
    width: u32,
    height: u32,
    mip_count: u32,
    format: TextureFormat,
) -> Image {
    let mut image = Image::new_uninit(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = mip_count;
    let mut sampler = ImageSamplerDescriptor::linear();
    sampler.set_address_mode(ImageAddressMode::Repeat);
    sampler.lod_max_clamp = 13.0;
    image.sampler = ImageSampler::Descriptor(sampler);
    image
}

pub(super) fn attach_prepared_diffuse_image(
    slot: usize,
    handle: Handle<Image>,
    queue: &mut SceneRenderQueue,
    images: &mut Assets<Image>,
    probe_materials: &mut Assets<ProbeMaterial>,
    decal_materials: &mut Assets<DecalMaterial>,
    sky_materials: &mut Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
) {
    for (material, role) in &queue.diffuse_image_materials[slot] {
        match material {
            MaterialHandle::Interior(_)
            | MaterialHandle::BoatVolume(_)
            | MaterialHandle::Water(_) => {
                unreachable!("depth-only materials have no texture bindings")
            }
            MaterialHandle::Sky(material) => {
                if let Some(mut material) = sky_materials.get_mut(material) {
                    *material.image_mut(role.0) = Some(handle.clone());
                }
            }
            MaterialHandle::Probe(material) => {
                if let Some(mut material) = probe_materials.get_mut(material) {
                    if *role == surface_gpu::ImageRole::occlusion() {
                        material.base.occlusion_texture = Some(handle.clone());
                    } else {
                        *material.extension.image_mut(*role) = Some(handle.clone());
                        if role.0 == 0 {
                            material.base.base_color_texture = Some(handle.clone());
                        }
                    }
                }
            }
            MaterialHandle::Decal(material) => {
                if let Some(mut material) = decal_materials.get_mut(material) {
                    if *role == surface_gpu::ImageRole::occlusion() {
                        material.base.occlusion_texture = Some(handle.clone());
                    } else {
                        *material.extension.image_mut(*role) = Some(handle.clone());
                        if role.0 == 0 {
                            material.base.base_color_texture = Some(handle.clone());
                        }
                    }
                }
            }
        }
    }
    if let Some(old) = queue.diffuse_image_handles[slot].replace(handle) {
        images.remove(old.id());
    }
}

/// Create the authored initial mip chain before any caster can reference it.
/// LoadResourceInternal (1402ea140 -> 1402e9ff0) uploads the binary image's
/// base levels when the resource is created; finer streaming is independent.
/// Bevy prepares this ordinary data-backed Image before exposing its GPU view.
pub(super) fn initialize_resident_image(
    catalog: &archive::Catalog,
    slot: usize,
    queue: &mut SceneRenderQueue,
    images: &mut Assets<Image>,
    probe_materials: &mut Assets<ProbeMaterial>,
    decal_materials: &mut Assets<DecalMaterial>,
    sky_materials: &mut Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
) -> Result<usize, String> {
    if queue
        .diffuse_image_handles
        .get(slot)
        .is_some_and(Option::is_some)
        && queue.diffuse_mip_states[slot].resident.is_some()
    {
        return Ok(0);
    }
    let path = queue
        .diffuse_image_paths
        .get(slot)
        .ok_or("resident image slot is absent")?;
    let streamed = read_resident_diffuse_image(catalog, path, queue.preferred_mip)?
    .ok_or_else(|| format!("resident image is absent: {path}"))?;
    commit_resident_image(streamed, slot, queue, images, probe_materials, decal_materials, sky_materials)
}

fn commit_resident_image(
    mut streamed: StreamedDiffuse, slot: usize, queue: &mut SceneRenderQueue,
    images: &mut Assets<Image>, probe_materials: &mut Assets<ProbeMaterial>,
    decal_materials: &mut Assets<DecalMaterial>,
    sky_materials: &mut Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>,
) -> Result<usize, String> {
    if !streamed.uploads.is_empty() {
        streamed.image.data = Some(streamed.uploads.into_iter().flat_map(|mip| mip.payload).collect());
    }
    let bytes = streamed.image.data.as_ref().map_or(0, Vec::len);
    let mip_count = streamed.image.texture_descriptor.mip_level_count;
    if let Some(handle) = &queue.diffuse_image_handles[slot] {
        // Fulfil the registered identity. Its material references already wait
        // on this exact handle; no redundant material mutation is needed.
        images
            .insert(handle.id(), streamed.image)
            .map_err(|e| e.to_string())?;
    } else {
        let handle = images.add(streamed.image);
        attach_prepared_diffuse_image(
            slot,
            handle,
            queue,
            images,
            probe_materials,
            decal_materials,
            sky_materials,
        );
    }
    commit_prepared_diffuse_level(queue, slot, streamed.level, mip_count);
    queue.diffuse_image_attempted[slot] = true;
    Ok(bytes)
}

pub(super) fn commit_prepared_diffuse_level(
    queue: &mut SceneRenderQueue,
    slot: usize,
    level: DiffuseMipLevel,
    mip_count: u32,
) {
    if queue.diffuse_mip_states[slot]
        .resident
        .is_some_and(|previous| level.width > previous.width || level.height > previous.height)
    {
        queue.diffuse_mip_upgrades += 1;
    }
    if queue.diffuse_mip_states[slot].minimum.is_none() {
        queue.diffuse_mip_states[slot].minimum = Some(level);
        queue.diffuse_mip_states[slot].minimum_mip_count = mip_count;
    }
    queue.diffuse_mip_states[slot].resident = Some(level);
    queue.diffuse_mip_states[slot].pending_reduction = false;
    queue.diffuse_mip_states[slot].reserved_bytes = 0;
    queue.diffuse_mip_states[slot].streamed_bytes = residency::finer_bytes(
        level, queue.diffuse_mip_states[slot].minimum.unwrap(),
    );
}

/// Host scratch belongs to the loaded world's streaming jobs, not one frame.
pub(super) struct FrameRequests {
    flags: Vec<bool>,
    order: Vec<usize>,
    allocate_each_frame: bool,
    timing: bool,
    flag_growth_frames: usize,
    order_growth_frames: usize,
    collect_completions: bool,
    completion_batches: usize,
    completion_items: usize,
    completion_temp_bytes: usize,
    clone_completion_paths: bool,
    completion_path_clones: usize,
    completion_path_bytes: usize,
    pressure: residency::Pressure,
}
impl Default for FrameRequests {
    fn default() -> Self {
        Self { flags: Vec::new(), order: Vec::new(), flag_growth_frames: 0, order_growth_frames: 0, pressure: Default::default(),
            collect_completions: std::env::var("DH2_DEBUG_COLLECT_TEXTURE_COMPLETIONS").as_deref() == Ok("1"),
            completion_batches: 0, completion_items: 0, completion_temp_bytes: 0,
            clone_completion_paths: std::env::var("DH2_DEBUG_CLONE_TEXTURE_COMPLETION_PATHS").as_deref() == Ok("1"),
            completion_path_clones: 0, completion_path_bytes: 0,
            allocate_each_frame: std::env::var("DH2_DEBUG_ALLOCATE_TEXTURE_REQUESTS").as_deref() == Ok("1"),
            timing: std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") }
    }
}
impl Drop for FrameRequests {
    fn drop(&mut self) {
        if self.timing {
            eprintln!("MEMORY texture completion map release: collect={} batches={} items={} temporary_capacity_bytes={} path_clones={} path_clone_bytes={}",
                self.collect_completions, self.completion_batches, self.completion_items, self.completion_temp_bytes,
                self.completion_path_clones, self.completion_path_bytes);
        }
    }
}

/// Resolve authored diffuse resources after a render instance has been shown
/// by the current visibility system. Archive reads run on a separate worker.
pub(super) fn stream_visible_textures(
    worker: Option<ResMut<TextureReadWorker>>,
    prepared_images: Option<Res<PreparedImageMailbox>>,
    mut queue: ResMut<SceneRenderQueue>,
    visible_instances: Option<Res<super::scene_queue::VisibleSceneInstances>>,
    mut images: ResMut<Assets<Image>>,
    mut probe_materials: ResMut<Assets<ProbeMaterial>>,
    mut decal_materials: ResMut<Assets<DecalMaterial>>,
    mut sky_materials: ResMut<Assets<crate::rendering::bevy_backend::sky_mask::SkyMaterial>>,
    mut diagnostics: Option<ResMut<StreamingDiagnostics>>,
    owners: (Option<Res<super::player_arms::PlayerArms>>, Option<Res<super::resident_images::ResidentImages>>),
    mut instance_slots: Local<InstanceImageSlots>,
    mut scratch: Local<FrameRequests>,
    mut initial_view_ready: Local<bool>,
    mut settings: Local<settings::SettingsCache>,
    adapter: Option<Res<bevy::render::renderer::RenderAdapter>>,
    mut shared_images: Option<ResMut<SharedImages>>,
) {
    let (arms, registry) = owners;
    let Some(mut worker) = worker else {
        return;
    };
    let settings = settings.get(adapter.as_deref());
    if queue.is_added() {
        *initial_view_ready = false;
        *instance_slots = Default::default();
        *scratch = Default::default();
    }
    queue.texture_stream_frame += 1;
    let frame = queue.texture_stream_frame;
    let FrameRequests { flags: requested, order: request_order, allocate_each_frame, timing,
        flag_growth_frames, order_growth_frames, collect_completions, completion_batches,
        completion_items, completion_temp_bytes, clone_completion_paths, completion_path_clones,
        completion_path_bytes, pressure } = &mut *scratch;
    if *allocate_each_frame { *requested = Vec::new(); *request_order = Vec::new(); }
    let flag_capacity = requested.capacity();
    let order_capacity = request_order.capacity();
    requested.resize(queue.diffuse_image_paths.len(), false);
    requested.fill(false);
    *flag_growth_frames += usize::from(requested.capacity() > flag_capacity);
    instance_slots.update(&queue);
    let slots = &*instance_slots;
    super::scene_queue::for_each_visible_instance_index(
        &queue,
        visible_instances.as_deref(),
        |index| {
            for &slot in slots.of(index) {
                if let Some(needed) = requested.get_mut(slot as usize) {
                    *needed = true;
                }
            }
        },
    );
    for material in arms.iter().flat_map(|arms| arms.shown_materials()) {
        for &slot in queue.material_images.get(material).into_iter().flatten() {
            requested[slot] = true;
        }
    }
    for &slot in &queue.shadow_diffuse_slots {
        if let Some(needed) = requested.get_mut(slot) {
            *needed = true;
        }
    }
    if let Some(prepared_images) = prepared_images.as_ref() {
        let (completed, failed) = {
            let mut state = prepared_images.0.lock().unwrap();
            (
                std::mem::take(&mut state.completed),
                std::mem::take(&mut state.failed),
            )
        };
        for (id, error) in failed {
            if shared_images.as_deref_mut().is_some_and(|shared| shared.finish_replacement(id, false, registry.as_deref())) {
                eprintln!("Shared image GPU reduction failed: {error}");
                images.remove(id);
                continue;
            }
            let Some(slot) = queue
                .diffuse_image_pending
                .iter()
                .position(|pending| pending.as_ref().is_some_and(|handle| handle.id() == id))
            else {
                continue;
            };
            eprintln!(
                "Streamed diffuse GPU upload {}: {error}",
                queue.diffuse_image_paths[slot]
            );
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.texture_event(format_args!(
                    "gpu_upload_failed stream_frame={frame} slot={slot} path={:?} image_id={id:?} resident={:?} pending={:?} error={error:?}",
                    queue.diffuse_image_paths[slot],
                    queue.diffuse_mip_states[slot].resident,
                    queue.diffuse_mip_states[slot].pending,
                ));
            }
            queue.diffuse_image_pending[slot] = None;
            let target = queue.diffuse_mip_states[slot].pending.take();
            let was_reduction = queue.diffuse_mip_states[slot].pending_reduction;
            queue.diffuse_mip_states[slot].pending_reduction = false;
            queue.diffuse_mip_states[slot].reserved_bytes = 0;
            if queue.diffuse_image_handles[slot].is_none() {
                queue.diffuse_image_attempted[slot] = false;
            } else if !was_reduction {
                if let Some(mip) = target.and_then(|level| level.resource_mip) {
                    queue.diffuse_mip_states[slot].unavailable_finer = Some(mip);
                }
            }
            images.remove(id);
        }
        for id in completed {
            if shared_images.as_deref_mut().is_some_and(|shared| shared.finish_replacement(id, true, registry.as_deref())) { continue; }
            let Some(slot) = queue
                .diffuse_image_pending
                .iter()
                .position(|pending| pending.as_ref().is_some_and(|handle| handle.id() == id))
            else {
                continue;
            };
            let handle = queue.diffuse_image_pending[slot].take().unwrap();
            let prepared_level = queue.diffuse_mip_states[slot].pending.take();
            let reduction = queue.diffuse_mip_states[slot].pending_reduction;
            let mip_count = images
                .get(&handle)
                .map(|image| image.texture_descriptor.mip_level_count)
                .unwrap_or(0);
            let image_id = handle.id();
            let resident_before = queue.diffuse_mip_states[slot].resident;
            // 14030d1e0 finishes admitted work; pressure owns later eviction.
            {
                attach_prepared_diffuse_image(
                    slot,
                    handle,
                    &mut queue,
                    &mut images,
                    &mut probe_materials,
                    &mut decal_materials,
                    &mut sky_materials,
                );
                if let Some(level) = prepared_level {
                    commit_prepared_diffuse_level(&mut queue, slot, level, mip_count);
                    if let Some(registry) = registry.as_deref() {
                        registry.publish_view(&queue.diffuse_image_paths[slot], queue.diffuse_image_handles[slot].as_ref().unwrap(), level, mip_count);
                    }
                }
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "gpu_upload_committed stream_frame={frame} slot={slot} path={:?} image_id={image_id:?} requested={} reduction={} mip_count={mip_count} resident_before={resident_before:?} resident_after={:?}",
                        queue.diffuse_image_paths[slot],
                        requested[slot],
                        reduction,
                        queue.diffuse_mip_states[slot].resident,
                    ));
                }
            }
        }
    }
    for (slot, &needed) in requested.iter().enumerate() {
        let last_visible = queue.diffuse_image_last_visible[slot];
        if needed {
            if last_visible == 0 || last_visible != frame.saturating_sub(1) {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "demand_enter stream_frame={frame} slot={slot} path={:?} projected_area={:.3} viewport_pixels={} resident={:?} pending={:?} attempted={} unavailable_finer={:?}",
                        queue.diffuse_image_paths[slot],
                        queue.diffuse_image_demands[slot].area(),
                        queue.texture_viewport_pixels,
                        queue.diffuse_mip_states[slot].resident,
                        queue.diffuse_mip_states[slot].pending,
                        queue.diffuse_image_attempted[slot],
                        queue.diffuse_mip_states[slot].unavailable_finer,
                    ));
                }
            }
            queue.diffuse_image_last_visible[slot] = frame;
        } else if last_visible == frame.saturating_sub(1) {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.texture_event(format_args!(
                    "demand_exit stream_frame={frame} slot={slot} path={:?} idle_frames={} resident={:?} pending={:?} attempted={} unavailable_finer={:?}",
                    queue.diffuse_image_paths[slot],
                    frame.saturating_sub(last_visible),
                    queue.diffuse_mip_states[slot].resident,
                    queue.diffuse_mip_states[slot].pending,
                    queue.diffuse_image_attempted[slot],
                    queue.diffuse_mip_states[slot].unavailable_finer,
                ));
            }
        }
    }
    let completion_count = {
        let worker = &mut *worker;
        let receiver = worker.completed.lock().unwrap();
        let incoming = std::iter::from_fn(|| receiver.try_recv().ok());
        if *collect_completions {
            // Same-build control for the former temporary batch allocation.
            let completions = incoming.collect::<Vec<_>>();
            let count = completions.len();
            if *timing { *completion_temp_bytes += completions.capacity() * std::mem::size_of::<TextureReadResult>(); }
            worker.ready.extend(completions);
            count
        } else {
            // Sender owns the channel, not this receiver mutex. Transfer each
            // completion directly to the map's queue, preserving FIFO order.
            let before = worker.ready.len();
            worker.ready.extend(incoming);
            worker.ready.len() - before
        }
    };
    if *timing {
        *completion_batches += usize::from(completion_count != 0);
        *completion_items += completion_count;
        if frame % 600 == 0 {
            eprintln!("MEMORY texture completion batches: frames={frame} collect={} batches={} items={} temporary_capacity_bytes={}",
                collect_completions, completion_batches, completion_items, completion_temp_bytes);
        }
    }
    let mut budget = texture::MipUploadBudget::new(settings.upload_kb);
    while budget.allows_next_upload() {
        let Some((slot, target, result)) = worker.ready.pop_front() else {
            break;
        };
        worker.inflight[slot] = false;
        worker.inflight_count -= 1;
        // The read belongs to this map even if the camera moved meanwhile.
        // Its original reservation remains charged through GPU completion.
        // The async request already owned its path. A completed read needs the
        // indexed name only for immediate diagnostics and errors.
        let path_copy = (*clone_completion_paths).then(|| queue.diffuse_image_paths[slot].clone());
        if *timing {
            if let Some(path) = &path_copy {
                *completion_path_clones += 1;
                *completion_path_bytes += path.len();
            }
        }
        let mut streamed = match result {
            Ok(Some(image)) => image,
            Ok(None) => {
                queue.diffuse_mip_states[slot].reserved_bytes = 0;
                let path = path_copy.as_deref().unwrap_or(&queue.diffuse_image_paths[slot]);
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "read_missing stream_frame={frame} slot={slot} path={path:?} target={target:?} resident={:?}",
                        queue.diffuse_mip_states[slot].resident,
                    ));
                }
                eprintln!("Streamed diffuse missing: {path}");
                if let TextureReadTarget::Exact(mip, _) = target {
                    queue.diffuse_mip_states[slot].unavailable_finer = Some(mip);
                }
                continue;
            }
            Err(error) => {
                queue.diffuse_mip_states[slot].reserved_bytes = 0;
                let path = path_copy.as_deref().unwrap_or(&queue.diffuse_image_paths[slot]);
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "read_failed stream_frame={frame} slot={slot} path={path:?} target={target:?} resident={:?} error={error:?}",
                        queue.diffuse_mip_states[slot].resident,
                    ));
                }
                eprintln!("Streamed diffuse {path}: {error}");
                if let TextureReadTarget::Exact(mip, _) = target {
                    queue.diffuse_mip_states[slot].unavailable_finer = Some(mip);
                }
                continue;
            }
        };
        let previous = queue.diffuse_image_handles[slot]
            .as_ref()
            .map(|handle| handle.id());
        if matches!(target, TextureReadTarget::Exact(_, _)) {
            let Some(old) = previous.and_then(|id| Assets::get(&*images, id)) else {
                queue.diffuse_mip_states[slot].reserved_bytes = 0;
                let path = path_copy.as_deref().unwrap_or(&queue.diffuse_image_paths[slot]);
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "exact_mip_lost_resident stream_frame={frame} slot={slot} path={path:?} target={target:?} recorded_resident={:?} handle_id={previous:?}",
                        queue.diffuse_mip_states[slot].resident,
                    ));
                }
                eprintln!("Streamed diffuse {} lost its resident image", path);
                continue;
            };
            streamed.image.texture_descriptor.mip_level_count =
                old.texture_descriptor.mip_level_count + 1;
        }
        let upload_bytes = streamed
            .uploads
            .iter()
            .map(|mip| mip.payload.len())
            .sum::<usize>();
        let level = streamed.level;
        if matches!(target, TextureReadTarget::Exact(_, _)) {
            queue.diffuse_mip_states[slot].reserved_bytes = upload_bytes as u64;
        }
        let upload_mips = streamed.uploads.len();
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            let path = path_copy.as_deref().unwrap_or(&queue.diffuse_image_paths[slot]);
            diagnostics.texture_event(format_args!(
                "read_succeeded stream_frame={frame} slot={slot} path={path:?} target={target:?} resident_before={:?} loaded_level={level:?} upload_mips={upload_mips} upload_bytes={upload_bytes}",
                queue.diffuse_mip_states[slot].resident,
            ));
        }
        budget.record_upload(upload_bytes as i32);
        let mip_count = streamed.image.texture_descriptor.mip_level_count;
        let handle = images.add(streamed.image);
        let image_id = handle.id();
        let copy = if previous.is_some() {
            GpuMipCopy::Extend
        } else {
            GpuMipCopy::None
        };
        if let Some(prepared_images) = prepared_images.as_ref() {
            queue.diffuse_image_pending[slot] = Some(handle.clone());
            queue.diffuse_mip_states[slot].pending = Some(level);
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                let path = path_copy.as_deref().unwrap_or(&queue.diffuse_image_paths[slot]);
                diagnostics.texture_event(format_args!(
                    "gpu_upload_submitted stream_frame={frame} slot={slot} path={path:?} image_id={image_id:?} previous={previous:?} copy={copy:?} target={target:?} level={level:?} mip_count={mip_count} upload_mips={upload_mips} upload_bytes={upload_bytes}",
                ));
            }
            prepared_images
                .0
                .lock()
                .unwrap()
                .pending
                .push(PendingGpuImageUpload {
                    id: image_id,
                    previous,
                    copy,
                    uploads: streamed.uploads,
                });
        } else {
            attach_prepared_diffuse_image(
                slot,
                handle,
                &mut queue,
                &mut images,
                &mut probe_materials,
                &mut decal_materials,
                &mut sky_materials,
            );
            commit_prepared_diffuse_level(&mut queue, slot, level, mip_count);
            if let Some(registry) = registry.as_deref() {
                registry.publish_view(&queue.diffuse_image_paths[slot], queue.diffuse_image_handles[slot].as_ref().unwrap(), level, mip_count);
            }
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                let path = path_copy.as_deref().unwrap_or(&queue.diffuse_image_paths[slot]);
                diagnostics.texture_event(format_args!(
                    "resident_committed_without_gpu_mailbox stream_frame={frame} slot={slot} path={path:?} image_id={image_id:?} target={target:?} level={level:?} mip_count={mip_count} upload_mips={upload_mips} upload_bytes={upload_bytes} resident_after={:?}",
                    queue.diffuse_mip_states[slot].resident,
                ));
            }
        }
    }
    if let Some(shared) = shared_images.as_deref_mut() { shared.age(); }
    let inactive_bytes = shared_images.as_deref().map_or(0, |shared| shared.charged_bytes());
    let mut charged = residency::charged_bytes(&queue) + inactive_bytes;
    let mut reduction = pressure.choose(&queue, &worker, &images, settings, charged);
    if let Some(shared) = shared_images.as_deref_mut() {
        if let Some(released) = shared.reduce(&mut images, prepared_images.as_deref(), settings, charged, reduction.as_ref().map(|r| r.key)) {
            charged -= released;
            pressure.reductions += 1;
            pressure.released_bytes += released;
            reduction = None;
        }
    }
    if let Some(reduction) = reduction {
        let residency::Reduction { slot, level, mip_count, dropped, released_bytes, .. } = reduction;
        let previous = queue.diffuse_image_handles[slot].as_ref().unwrap().id();
        let handle = images.add(empty_streamed_image(level.width, level.height, mip_count, gpu_image_format(level.format)));
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.texture_event(format_args!("mip_reduction_requested stream_frame={frame} slot={slot} path={:?} dropped={dropped} released_bytes={released_bytes} charged_bytes={charged} budget_bytes={}",
                queue.diffuse_image_paths[slot], residency::budget_bytes(settings)));
        }
        if let Some(prepared_images) = prepared_images.as_ref() {
            queue.diffuse_image_pending[slot] = Some(handle.clone());
            queue.diffuse_mip_states[slot].pending = Some(level);
            queue.diffuse_mip_states[slot].pending_reduction = true;
            prepared_images.0.lock().unwrap().pending.push(PendingGpuImageUpload {
                id: handle.id(), previous: Some(previous), copy: GpuMipCopy::Reduce { dropped }, uploads: Vec::new(),
            });
        } else {
            attach_prepared_diffuse_image(slot, handle, &mut queue, &mut images, &mut probe_materials, &mut decal_materials, &mut sky_materials);
            commit_prepared_diffuse_level(&mut queue, slot, level, mip_count);
            if let Some(registry) = registry.as_deref() {
                registry.publish_view(&queue.diffuse_image_paths[slot], queue.diffuse_image_handles[slot].as_ref().unwrap(), level, mip_count);
            }
        }
        charged -= released_bytes;
        pressure.reductions += 1;
        pressure.released_bytes += released_bytes;
    }
    if *timing && frame % 600 == 0 {
        eprintln!("MEMORY texture residency: frames={frame} charged_bytes={charged} budget_bytes={} reductions={} released_bytes={}",
            residency::budget_bytes(settings), pressure.reductions, pressure.released_bytes);
    }
    request_order.clear();
    if !settings.frozen && charged <= residency::budget_bytes(settings) {
        pressure.request_order(&queue, requested, settings.add_all_mips, request_order);
    }
    *order_growth_frames += usize::from(request_order.capacity() > order_capacity);
    if *timing && frame % 600 == 0 {
        eprintln!("MEMORY texture request scratch: frames={frame} allocate_each_frame={allocate_each_frame} flag_growth_frames={flag_growth_frames} order_growth_frames={order_growth_frames} flags_capacity={} order_capacity={}",
            requested.capacity(), request_order.capacity());
    }
    let request_count = request_order.len();
    for &slot in request_order.iter() {
        if charged > residency::budget_bytes(settings) { break; }
        if worker.inflight[slot] || queue.diffuse_image_pending[slot].is_some() {
            queue.diffuse_image_demands[slot].take();
            if frame % 60 == 0 {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "request_waiting stream_frame={frame} slot={slot} path={:?} read_inflight={} gpu_pending={} resident={:?} pending={:?} demand_area={:.3}",
                        queue.diffuse_image_paths[slot],
                        worker.inflight[slot],
                        queue.diffuse_image_pending[slot].is_some(),
                        queue.diffuse_mip_states[slot].resident,
                        queue.diffuse_mip_states[slot].pending,
                        queue.diffuse_image_demands[slot].area(),
                    ));
                }
            }
            continue;
        }
        if worker.inflight_count == TEXTURE_READ_QUEUE_CAPACITY {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.texture_event(format_args!(
                    "read_queue_saturated stream_frame={frame} inflight={}/{} remaining_requests={}",
                    worker.inflight_count,
                    TEXTURE_READ_QUEUE_CAPACITY,
                    request_count,
                ));
            }
            break;
        }
        let demand_area = queue.diffuse_image_demands[slot].take();
        let target = if let Some(level) = queue.diffuse_mip_states[slot].resident {
            let Some(next_suffix) = level.next_separate_suffix else {
                continue;
            };
            if queue.diffuse_mip_states[slot].unavailable_finer == Some(next_suffix) {
                continue;
            }
            let multiplier = if settings.priority_multiplier { (queue.diffuse_image_demands[slot].metadata() >> 1 & 31) as f32 } else { 1.0 };
            let projected_pixels = demand_area * multiplier * queue.texture_viewport_pixels as f32;
            let (Ok(width), Ok(height)) = (i32::try_from(level.width), i32::try_from(level.height))
            else {
                continue;
            };
            if !texture::next_mip_is_eligible(
                0,
                1,
                0,
                width,
                height,
                level.format.bits_per_pixel(),
                projected_pixels,
                settings.add_all_mips,
                settings.max_texture_kb,
            ) {
                continue;
            }
            TextureReadTarget::Exact(next_suffix, level.format)
        } else {
            if queue.diffuse_image_attempted[slot] {
                continue;
            }
            TextureReadTarget::Initial(queue.preferred_mip)
        };
        match worker.request((slot, queue.diffuse_image_paths[slot].clone(), target)) {
            Ok(()) => {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "read_requested stream_frame={frame} slot={slot} path={:?} target={target:?} resident={:?} projected_area={demand_area:.3} viewport_pixels={} inflight_before={}/{} unavailable_finer={:?}",
                        queue.diffuse_image_paths[slot],
                        queue.diffuse_mip_states[slot].resident,
                        queue.texture_viewport_pixels,
                        worker.inflight_count,
                        TEXTURE_READ_QUEUE_CAPACITY,
                        queue.diffuse_mip_states[slot].unavailable_finer,
                    ));
                }
                // Accepted finer-mip work is charged before the next image is
                // considered, as at 14030ff34. Base chains remain excluded.
                if let TextureReadTarget::Exact(_, _) = target {
                    let mut next = queue.diffuse_mip_states[slot].resident.unwrap();
                    next.width *= 2;
                    next.height *= 2;
                    let bytes = residency::level_bytes(next);
                    queue.diffuse_mip_states[slot].reserved_bytes = bytes;
                    charged += bytes;
                }
                if matches!(target, TextureReadTarget::Initial(_)) {
                    queue.diffuse_image_attempted[slot] = true;
                }
                worker.inflight[slot] = true;
                worker.inflight_count += 1;
            }
            Err(_) => {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format_args!(
                        "read_queue_full stream_frame={frame} slot={slot} path={:?} target={target:?} inflight={}/{}",
                        queue.diffuse_image_paths[slot],
                        worker.inflight_count,
                        TEXTURE_READ_QUEUE_CAPACITY,
                    ));
                }
                break;
            }
        }
    }
    if !*initial_view_ready && queue.texture_viewport_pixels != 0 && worker.inflight_count == 0
        && queue.diffuse_image_pending.iter().all(Option::is_none) {
        *initial_view_ready = true;
        crate::loading::complete("initial view texture residency");
        super::load_timing::mark("initial view texture residency ready");
    }
}

/// The image slots each scene instance requests when visible: its material's
/// slots, else its own diffuse slot. Materials and their slots are fixed once
/// built and instances are only appended, so each instance is resolved once
/// instead of hashing its material for every visible instance every frame.
#[derive(Default)]
pub(super) struct InstanceImageSlots {
    instances: usize,
    ranges: Vec<(u32, u32)>,
    slots: Vec<u32>,
}

impl InstanceImageSlots {
    fn update(&mut self, queue: &SceneRenderQueue) {
        let buffer = queue.instances.as_ptr() as usize;
        if self.instances != buffer || self.ranges.len() > queue.instances.len() {
            *self = Self {
                instances: buffer,
                ..Default::default()
            };
        }
        for instance in &queue.instances[self.ranges.len()..] {
            let start = self.slots.len() as u32;
            if let Some(slots) = queue.material_images.get(&instance.material) {
                self.slots.extend(slots.iter().map(|&slot| slot as u32));
            } else if let Some(slot) = instance.diffuse_image_slot {
                self.slots.push(slot as u32);
            }
            self.ranges.push((start, self.slots.len() as u32));
        }
    }

    fn of(&self, index: usize) -> &[u32] {
        let (start, end) = self.ranges[index];
        &self.slots[start as usize..end as usize]
    }
}
