//! Texture streaming.
use super::ImageAccess;
use super::{
    archive, entities, material, mpsc, render_state, surface_gpu, texture, Arc, AssetId,
    DecalMaterial, Extent3d, GpuImage, ImageAddressMode, ImageSampler, ImageSamplerDescriptor,
    MaterialHandle, Mutex, ProbeMaterial, Receiver, RenderAssetUsages, RenderAssets, RenderDevice,
    RenderQueue, SceneRenderQueue, StreamingDiagnostics, SyncSender, TexelCopyBufferLayout,
    TextureDimension, TextureFormat, VecDeque,
};
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiffuseMipLevel {
    pub(super) resource_mip: Option<u8>,
    pub(super) next_separate_suffix: Option<u8>,
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) format: texture::ImageFormat,
}

#[derive(Default)]
pub(super) struct DiffuseMipState {
    pub(super) resident: Option<DiffuseMipLevel>,
    pub(super) pending: Option<DiffuseMipLevel>,
    pub(super) minimum: Option<DiffuseMipLevel>,
    pub(super) minimum_mip_count: u32,
    pub(super) pending_reduction: bool,
    pub(super) unavailable_finer: Option<u8>,
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

pub(super) fn report_prepared_diffuse_images(
    gpu_images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mailbox: Res<PreparedImageMailbox>,
) {
    let pending = {
        let mut state = mailbox.0.lock().unwrap();
        std::mem::take(&mut state.pending)
    };
    let mut waiting = Vec::new();
    let mut completed = Vec::new();
    let mut failed = Vec::new();
    for job in pending {
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
            Ok(()) => completed.push(job.id),
            Err(error) => failed.push((job.id, error)),
        }
    }
    let mut state = mailbox.0.lock().unwrap();
    state.pending.extend(waiting);
    state.completed.extend(completed);
    state.failed.extend(failed);
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

#[derive(Resource)]
pub(super) struct TextureReadWorker {
    pub(super) requests: SyncSender<(usize, String, TextureReadTarget)>,
    pub(super) completed: Mutex<Receiver<TextureReadResult>>,
    pub(super) ready: VecDeque<TextureReadResult>,
    pub(super) inflight: Vec<bool>,
    pub(super) inflight_count: usize,
}

impl TextureReadWorker {
    pub(super) fn new(catalog: archive::Catalog, image_count: usize) -> Self {
        let (requests, request_rx) =
            mpsc::sync_channel::<(usize, String, TextureReadTarget)>(TEXTURE_READ_QUEUE_CAPACITY);
        let (completed_tx, completed) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok((slot, stem, target)) = request_rx.recv() {
                let result = read_streamed_diffuse_at_target(&catalog, &stem, target);
                if completed_tx.send((slot, target, result)).is_err() {
                    break;
                }
            }
        });
        Self {
            requests,
            completed: Mutex::new(completed),
            ready: VecDeque::new(),
            inflight: vec![false; image_count],
            inflight_count: 0,
        }
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
    if let Some(payload) = material::intrinsic_image_pixels(stem) {
        if matches!(target, TextureReadTarget::Exact(..)) {
            return Err("intrinsic image has no streamed finer mip".into());
        }
        return Ok(Some(StreamedDiffuse {
            image: empty_streamed_image(16, 16, 1, TextureFormat::Rgba8Unorm),
            uploads: vec![CompressedMipUpload {
                mip_level: 0,
                width: 16,
                height: 16,
                payload,
            }],
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
    let inline = texture::compressed_inline_chain(&bytes)?;
    let format = inline.info.format;
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
            uploads.push(CompressedMipUpload {
                mip_level: mip_count,
                width: compressed.width,
                height: compressed.height,
                payload: compressed.payload.to_vec(),
            });
            mip_count += 1;
        }
    }
    if let Some((width, height)) = previous {
        if inline.info.width != (width / 2).max(1) || inline.info.height != (height / 2).max(1) {
            return Err("authored inline base does not follow separate mips".into());
        }
    }
    top.get_or_insert((inline.info.width, inline.info.height));
    let mut offset = 0;
    for index in 0..inline.info.mip_count {
        let width = (inline.info.width >> index).max(1);
        let height = (inline.info.height >> index).max(1);
        let size = format.mip_bytes(width, height);
        uploads.push(CompressedMipUpload {
            mip_level: mip_count + index,
            width,
            height,
            payload: inline.payload[offset..offset + size].to_vec(),
        });
        offset += size;
    }
    mip_count += inline.info.mip_count;
    let (width, height) = top.ok_or("authored image has no mip payload")?;
    Ok(Some(StreamedDiffuse {
        image: empty_streamed_image(width, height, mip_count, gpu_image_format(format)),
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

pub(super) fn gpu_image_format(format: texture::ImageFormat) -> TextureFormat {
    use texture::ImageFormat::*;
    match format {
        Rgba8 => TextureFormat::Rgba8Unorm,
        Rgba8Srgb => TextureFormat::Rgba8UnormSrgb,
        Rg8 => TextureFormat::Rg8Unorm,
        R16Float => TextureFormat::R16Float,
        Bgra8 => TextureFormat::Bgra8Unorm,
        Bc1 => TextureFormat::Bc1RgbaUnorm,
        Bc1Srgb => TextureFormat::Bc1RgbaUnormSrgb,
        Bc3 => TextureFormat::Bc3RgbaUnorm,
        Bc3Srgb => TextureFormat::Bc3RgbaUnormSrgb,
        Bc4 => TextureFormat::Bc4RUnorm,
        Bc4Snorm => TextureFormat::Bc4RSnorm,
        Bc5 => TextureFormat::Bc5RgUnorm,
        Bc5Snorm => TextureFormat::Bc5RgSnorm,
        Bc6Unsigned => TextureFormat::Bc6hRgbUfloat,
        Bc6Signed => TextureFormat::Bc6hRgbFloat,
        Bc7 => TextureFormat::Bc7RgbaUnorm,
        Bc7Srgb => TextureFormat::Bc7RgbaUnormSrgb,
    }
}

pub(super) fn streamed_block_bytes(format: TextureFormat) -> Option<usize> {
    match format {
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb => Some(4),
        TextureFormat::Rg8Unorm | TextureFormat::R16Float => Some(2),
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

pub(super) fn surface_sampler_image(
    catalog: Option<&archive::Catalog>,
) -> Result<Image, String> {
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
) {
    for (material, role) in &queue.diffuse_image_materials[slot] {
        match material {
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
) -> Result<usize, String> {
    if queue.diffuse_image_handles.get(slot).is_some_and(Option::is_some) {
        return Ok(0);
    }
    let path = queue.diffuse_image_paths.get(slot).ok_or("resident image slot is absent")?;
    let mut streamed = read_streamed_diffuse_at_target(
        catalog, path, TextureReadTarget::Initial(queue.preferred_mip),
    )?.ok_or_else(|| format!("resident image is absent: {path}"))?;
    let data: Vec<_> = streamed.uploads.into_iter().flat_map(|mip| mip.payload).collect();
    let bytes = data.len();
    streamed.image.data = Some(data);
    let mip_count = streamed.image.texture_descriptor.mip_level_count;
    let handle = images.add(streamed.image);
    attach_prepared_diffuse_image(slot, handle, queue, images, probe_materials, decal_materials);
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
    mut diagnostics: Option<ResMut<StreamingDiagnostics>>,
) {
    let Some(mut worker) = worker else {
        return;
    };
    queue.texture_stream_frame += 1;
    let frame = queue.texture_stream_frame;
    let mut requested = vec![false; queue.diffuse_image_paths.len()];
    super::scene_queue::for_each_visible_instance(
        &queue,
        visible_instances.as_deref(),
        |instance| {
            if let Some(slots) = queue.material_images.get(&instance.material) {
                for &slot in slots {
                    requested[slot] = true;
                }
            } else if let Some(slot) = instance.diffuse_image_slot {
                if let Some(needed) = requested.get_mut(slot) {
                    *needed = true;
                }
            }
        },
    );
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
                diagnostics.texture_event(format!(
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
            if requested[slot] || reduction {
                attach_prepared_diffuse_image(
                    slot,
                    handle,
                    &mut queue,
                    &mut images,
                    &mut probe_materials,
                    &mut decal_materials,
                );
                if let Some(level) = prepared_level {
                    commit_prepared_diffuse_level(&mut queue, slot, level, mip_count);
                }
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
                        "gpu_upload_committed stream_frame={frame} slot={slot} path={:?} image_id={image_id:?} requested={} reduction={} mip_count={mip_count} resident_before={resident_before:?} resident_after={:?}",
                        queue.diffuse_image_paths[slot],
                        requested[slot],
                        reduction,
                        queue.diffuse_mip_states[slot].resident,
                    ));
                }
            } else {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
                        "gpu_upload_discarded_invisible stream_frame={frame} slot={slot} path={:?} image_id={image_id:?} mip_count={mip_count} resident={resident_before:?} pending={prepared_level:?}",
                        queue.diffuse_image_paths[slot],
                    ));
                }
                queue.diffuse_mip_states[slot].pending_reduction = false;
                images.remove(id);
                if queue.diffuse_image_handles[slot].is_none() {
                    queue.diffuse_image_attempted[slot] = false;
                }
            }
        }
    }
    for (slot, &needed) in requested.iter().enumerate() {
        let last_visible = queue.diffuse_image_last_visible[slot];
        if needed {
            if last_visible == 0 || last_visible != frame.saturating_sub(1) {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
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
                diagnostics.texture_event(format!(
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
        if !needed && frame.saturating_sub(last_visible) > TEXTURE_REDUCE_AFTER_FRAMES {
            let state = &queue.diffuse_mip_states[slot];
            let (Some(resident), Some(minimum), Some(old_handle)) = (
                state.resident,
                state.minimum,
                queue.diffuse_image_handles[slot].as_ref(),
            ) else {
                continue;
            };
            if resident == minimum
                || state.pending.is_some()
                || queue.diffuse_image_pending[slot].is_some()
                || worker.inflight[slot]
            {
                continue;
            }
            let Some(old_image) = images.get(old_handle) else {
                continue;
            };
            let old_count = old_image.texture_descriptor.mip_level_count;
            let minimum_count = state.minimum_mip_count;
            if minimum_count == 0 || old_count <= minimum_count {
                continue;
            }
            let previous = old_handle.id();
            let dropped = old_count - minimum_count;
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.texture_event(format!(
                    "mip_reduction_requested stream_frame={frame} slot={slot} path={:?} old_mip_count={old_count} minimum_mip_count={minimum_count} dropped={dropped} resident={resident:?} minimum={minimum:?} idle_frames={}",
                    queue.diffuse_image_paths[slot],
                    frame.saturating_sub(queue.diffuse_image_last_visible[slot]),
                ));
            }
            let handle = images.add(empty_streamed_image(
                minimum.width,
                minimum.height,
                minimum_count,
                gpu_image_format(minimum.format),
            ));
            if let Some(prepared_images) = prepared_images.as_ref() {
                queue.diffuse_image_pending[slot] = Some(handle.clone());
                queue.diffuse_mip_states[slot].pending = Some(minimum);
                queue.diffuse_mip_states[slot].pending_reduction = true;
                prepared_images
                    .0
                    .lock()
                    .unwrap()
                    .pending
                    .push(PendingGpuImageUpload {
                        id: handle.id(),
                        previous: Some(previous),
                        copy: GpuMipCopy::Reduce { dropped },
                        uploads: Vec::new(),
                    });
            } else {
                attach_prepared_diffuse_image(
                    slot,
                    handle,
                    &mut queue,
                    &mut images,
                    &mut probe_materials,
                    &mut decal_materials,
                );
                commit_prepared_diffuse_level(&mut queue, slot, minimum, minimum_count);
            }
        }
    }
    let completions = {
        let receiver = worker.completed.lock().unwrap();
        std::iter::from_fn(|| receiver.try_recv().ok()).collect::<Vec<_>>()
    };
    worker.ready.extend(completions);
    let mut budget = texture::MipUploadBudget::new(TEXTURE_UPLOAD_BUDGET_KB);
    while budget.allows_next_upload() {
        let Some((slot, target, result)) = worker.ready.pop_front() else {
            break;
        };
        worker.inflight[slot] = false;
        worker.inflight_count -= 1;
        // A read may finish after its last visible instance has left the
        // query. Do not stage that payload or keep it resident for the unload
        // grace period; a later visible frame can request it again.
        let result_summary = match &result {
            Ok(Some(image)) => format!(
                "loaded_level={:?} upload_mips={} upload_bytes={}",
                image.level,
                image.uploads.len(),
                image
                    .uploads
                    .iter()
                    .map(|mip| mip.payload.len())
                    .sum::<usize>(),
            ),
            Ok(None) => "missing".to_owned(),
            Err(error) => format!("error={error:?}"),
        };
        if !requested[slot] {
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.texture_event(format!(
                    "read_completed_invisible stream_frame={frame} slot={slot} path={:?} target={target:?} resident={:?} result={result_summary}",
                    queue.diffuse_image_paths[slot],
                    queue.diffuse_mip_states[slot].resident,
                ));
            }
            if queue.diffuse_image_handles[slot].is_none() {
                queue.diffuse_image_attempted[slot] = false;
            }
            continue;
        }
        let path = queue.diffuse_image_paths[slot].clone();
        let mut streamed = match result {
            Ok(Some(image)) => image,
            Ok(None) => {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
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
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
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
            let Some(old) = previous.and_then(|id| images.get(id)) else {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
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
        let upload_mips = streamed.uploads.len();
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.texture_event(format!(
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
                diagnostics.texture_event(format!(
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
            );
            commit_prepared_diffuse_level(&mut queue, slot, level, mip_count);
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.texture_event(format!(
                    "resident_committed_without_gpu_mailbox stream_frame={frame} slot={slot} path={path:?} image_id={image_id:?} target={target:?} level={level:?} mip_count={mip_count} upload_mips={upload_mips} upload_bytes={upload_bytes} resident_after={:?}",
                    queue.diffuse_mip_states[slot].resident,
                ));
            }
        }
    }
    let request_order =
        texture::projected_area_request_order(&requested, &queue.diffuse_image_demands)
            .expect("diffuse request flags align with image demands");
    let request_count = request_order.len();
    for slot in request_order {
        if worker.inflight[slot] || queue.diffuse_image_pending[slot].is_some() {
            if frame % 60 == 0 {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
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
                diagnostics.texture_event(format!(
                    "read_queue_saturated stream_frame={frame} inflight={}/{} remaining_requests={}",
                    worker.inflight_count,
                    TEXTURE_READ_QUEUE_CAPACITY,
                    request_count,
                ));
            }
            break;
        }
        let demand_area = queue.diffuse_image_demands[slot].area();
        let target = if let Some(level) = queue.diffuse_mip_states[slot].resident {
            let Some(next_suffix) = level.next_separate_suffix else {
                continue;
            };
            if queue.diffuse_mip_states[slot].unavailable_finer == Some(next_suffix) {
                continue;
            }
            let projected_pixels =
                queue.diffuse_image_demands[slot].area() * queue.texture_viewport_pixels as f32;
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
                false,
                131_072,
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
        match worker
            .requests
            .try_send((slot, queue.diffuse_image_paths[slot].clone(), target))
        {
            Ok(()) => {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
                        "read_requested stream_frame={frame} slot={slot} path={:?} target={target:?} resident={:?} projected_area={demand_area:.3} viewport_pixels={} inflight_before={}/{} unavailable_finer={:?}",
                        queue.diffuse_image_paths[slot],
                        queue.diffuse_mip_states[slot].resident,
                        queue.texture_viewport_pixels,
                        worker.inflight_count,
                        TEXTURE_READ_QUEUE_CAPACITY,
                        queue.diffuse_mip_states[slot].unavailable_finer,
                    ));
                }
                // The original scheduler clears accumulated area only when
                // it accepts work for that image, not on every frame.
                queue.diffuse_image_demands[slot].take();
                if matches!(target, TextureReadTarget::Initial(_)) {
                    queue.diffuse_image_attempted[slot] = true;
                }
                worker.inflight[slot] = true;
                worker.inflight_count += 1;
            }
            Err(mpsc::TrySendError::Full(_)) => {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
                        "read_queue_full stream_frame={frame} slot={slot} path={:?} target={target:?} inflight={}/{}",
                        queue.diffuse_image_paths[slot],
                        worker.inflight_count,
                        TEXTURE_READ_QUEUE_CAPACITY,
                    ));
                }
                break;
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                if let Some(diagnostics) = diagnostics.as_deref_mut() {
                    diagnostics.texture_event(format!(
                        "read_queue_disconnected stream_frame={frame} slot={slot} path={:?} target={target:?}",
                        queue.diffuse_image_paths[slot],
                    ));
                }
                eprintln!("Texture read worker stopped");
                break;
            }
        }
    }
}
