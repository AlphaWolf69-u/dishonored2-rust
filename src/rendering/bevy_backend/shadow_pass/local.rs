//! Original local atlas/cache submission, using the shared native draw binder.
use super::*;
use crate::render_shadow::AtlasAllocation;
use crate::shadow_depth_copy::NativeShadowDepthCopy;
use std::collections::HashMap;
#[path = "local/receiver_trace.rs"]
mod receiver_trace;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeLocalShadowRegion {
    pub atlas: AtlasAllocation,
    pub cache: Option<AtlasAllocation>,
    pub static_instances: usize,
    pub contains_models: bool,
    pub static_prepared: bool,
}

#[derive(Clone, Debug)]
pub struct NativeLocalShadowSubmission {
    pub revision: u64,
    pub atlas: Handle<Image>,
    pub cache: Handle<Image>,
    pub atlas_extent: [u32; 3],
    pub cache_extent: [u32; 3],
    pub depth_32_bits: bool,
    pub copy_state: [u32; 9],
    pub layers: Vec<NativeShadowLayer>,
    pub regions: Vec<NativeLocalShadowRegion>,
    pub receiver: Result<NativeLocalShadowReceiver, String>,
}

#[derive(Clone, Debug)]
pub struct NativeLocalShadowReceiverLight {
    pub entity: Entity,
    pub kind: u32,
    pub position: [f32; 3],
    pub left: [f32; 3],
    pub direction: [f32; 3],
    pub first: u32,
    pub count: u32,
}

#[derive(Clone, Debug)]
pub struct NativeLocalShadowReceiver {
    pub bias: [f32; 3],
    pub lights: Vec<NativeLocalShadowReceiverLight>,
    pub shadows: Vec<[u32; 36]>,
}

#[derive(Resource, Clone, Default, ExtractResource)]
// A publication owns its immutable generation until both worlds release it.
// Extracting it must not clone every local caster's CB and asset handles.
pub struct NativeLocalShadowSceneFrame(pub Option<Arc<NativeLocalShadowSubmission>>);

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct NativeLocalShadowCompletion(
    pub Arc<Mutex<Option<(u64, AssetId<Image>, AssetId<Image>, HashMap<u32, bool>)>>>,
);

/// Forward pipeline readiness gates depth publication and diagnostic captures.
/// Atlas completion alone cannot establish drawable forward geometry.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct NativeShadowCaptureReadiness(pub Arc<std::sync::atomic::AtomicU32>);

#[derive(Resource, Default)]
struct LocalGpu {
    programs: NativeShadowSceneGpu,
    arrays: Option<(NativeShadowDepthArray, NativeShadowDepthArray)>,
    copy: Option<([u32; 9], NativeShadowDepthCopy)>,
    valid: HashMap<u32, bool>,
    receiver_error: Option<String>,
    receiver_reported: bool,
    retained_atlas: Option<RetainedLocalAtlas>,
    atlas_reused: u64,
}

/// Atlas pixels can survive an idle frame only after a successful submission
/// with these exact GPU owners, cache rectangles and copy state. No caster
/// commands or missing static-cache allocations may be skipped.
#[derive(PartialEq, Eq)]
struct RetainedLocalAtlas {
    atlas: bevy::render::render_resource::TextureId,
    cache: bevy::render::render_resource::TextureId,
    copy_state: [u32; 9],
    regions: Vec<NativeLocalShadowRegion>,
}
impl RetainedLocalAtlas {
    fn idle(
        frame: &NativeLocalShadowSubmission,
        atlas: bevy::render::render_resource::TextureId,
        cache: bevy::render::render_resource::TextureId,
        valid: &HashMap<u32, bool>,
    ) -> Option<Self> {
        if frame.layers.iter().any(|layer| !layer.instances.is_empty())
            || frame.regions.iter().any(|region| {
                region.static_instances != 0
                    || region
                        .cache
                        .is_some_and(|allocation| !valid.contains_key(&allocation.id))
            })
        {
            return None;
        }
        Some(Self {
            atlas,
            cache,
            copy_state: frame.copy_state,
            regions: frame.regions.clone(),
        })
    }
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<NativeLocalShadowSceneFrame>()
        .init_resource::<NativeLocalShadowCompletion>()
        .init_resource::<NativeShadowCaptureReadiness>()
        .add_plugins(ExtractResourcePlugin::<NativeShadowCaptureReadiness>::default())
        .add_plugins(ExtractResourcePlugin::<NativeLocalShadowSceneFrame>::default())
        .add_plugins(ExtractResourcePlugin::<NativeLocalShadowCompletion>::default())
        .add_plugins(ExtractResourcePlugin::<
            crate::shadow_sampling::NativeLocalSamplingBuffer,
        >::default());
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<LocalGpu>()
            .init_resource::<HdrTrace>()
            .add_systems(Render, prepare.in_set(RenderSystems::PrepareResources))
            .add_systems(
                Core3d,
                (draw, trace_material_readiness, update_capture_readiness)
                    .before(Core3dSystems::MainPass),
            )
            .add_systems(
                Core3d,
                trace_hdr
                    .after(Core3dSystems::MainPass)
                    .before(Core3dSystems::PostProcess),
            )
            .add_systems(Render, map_hdr_trace.in_set(RenderSystems::Cleanup));
        receiver_trace::install(render);
    }
}

#[derive(Resource, Default)]
struct HdrTrace {
    tick: u32,
    staging: Option<Buffer>,
    mapped: bool,
}

fn trace_hdr(
    mut trace: ResMut<HdrTrace>,
    gpu: Res<LocalGpu>,
    readiness: Res<NativeShadowCaptureReadiness>,
    targets: Query<&bevy::render::view::ViewTarget>,
    device: Res<RenderDevice>,
    mut context: RenderContext,
) {
    if std::env::var_os("DH2_TRACE_HDR").is_none()
        || trace.staging.is_some()
        || !gpu.receiver_reported
    {
        return;
    }
    if readiness.0.load(std::sync::atomic::Ordering::Acquire) < 30 {
        return;
    }
    trace.tick += 1;
    if trace.tick < 120 {
        return;
    }
    let Ok(target) = targets.single() else {
        return;
    };
    if target.main_texture().sample_count() != 1 {
        return;
    }
    use bevy::render::render_resource::*;
    let shader=device.create_and_validate_shader_module(ShaderModuleDescriptor {label:Some("native shadow HDR diagnostic"),source:ShaderSource::Wgsl(r#"
        @group(0) @binding(0) var hdr:texture_2d<f32>;
        @group(0) @binding(1) var<storage,read_write> values:array<vec4<f32>>;
        @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) i:vec3<u32>) {
            let size=textureDimensions(hdr);let uv=(vec2<f32>(f32(i.x%3u),f32(i.x/3u))+vec2(1.0))*0.25;
            values[i.x]=textureLoad(hdr,vec2<i32>(uv*vec2<f32>(size)),0);
        }
    "#.into())});
    let pipeline = device
        .wgpu_device()
        .create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("native shadow HDR diagnostic"),
            layout: None,
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 144,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 144,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let group = device
        .wgpu_device()
        .create_bind_group(&BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(target.main_texture_view()),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: output.as_entire_binding(),
                },
            ],
        });
    let encoder = context.command_encoder();
    {
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.dispatch_workgroups(9, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 144);
    trace.staging = Some(staging);
}

fn map_hdr_trace(mut trace: ResMut<HdrTrace>) {
    if trace.mapped {
        return;
    }
    let Some(staging) = trace.staging.clone() else {
        return;
    };
    trace.mapped = true;
    let callback = staging.clone();
    staging.slice(..).map_async(
        bevy::render::render_resource::MapMode::Read,
        move |result| {
            if let Err(error) = result {
                eprintln!("Native HDR diagnostic mapping failed: {error}");
                return;
            }
            let mapped = callback.slice(..).get_mapped_range();
            let mut text = String::new();
            for (i, pixel) in mapped.chunks_exact(16).enumerate() {
                let rgba: [f32; 4] = std::array::from_fn(|c| {
                    f32::from_le_bytes(pixel[c * 4..c * 4 + 4].try_into().unwrap())
                });
                text.push_str(&format!(
                    "{i}: {rgba:?} bits={:08x?}\n",
                    rgba.map(f32::to_bits)
                ));
            }
            eprintln!("Native HDR before tonemapping:\n{text}");
            if let Some(path) = std::env::var_os("DH2_TRACE_HDR") {
                if let Err(error) = std::fs::write(path, &text) {
                    eprintln!("Native HDR diagnostic write failed: {error}");
                }
            }
            drop(mapped);
            callback.unmap();
        },
    );
}

fn trace_material_readiness(
    materials: Res<bevy::pbr::RenderMaterialBindings>,
    pipelines: Res<bevy::render::render_resource::PipelineCache>,
    depth_keys: Option<Res<bevy::pbr::DepthPrepassMaterialKeys>>,
    mut tick: Local<u32>,
) {
    if std::env::var_os("DH2_TRACE_MATERIAL_READINESS").is_none() {
        return;
    }
    *tick += 1;
    if *tick % 120 != 1 {
        return;
    }
    let mut counts = [0; 3];
    for p in pipelines.pipelines() {
        match &p.state {
            bevy::render::render_resource::CachedPipelineState::Ok(_) => counts[0] += 1,
            bevy::render::render_resource::CachedPipelineState::Err(_) => counts[1] += 1,
            _ => counts[2] += 1,
        }
    }
    eprintln!(
        "Native material readiness: {} bindings, pipelines ready/error/pending={counts:?}",
        materials.len()
    );
    if let Some(keys) = depth_keys {
        eprintln!(
            "Depth specialization: normalized_keys={} omitted_materials={}",
            keys.normalized_keys(),
            keys.omitted_materials()
        );
    }
}

fn update_capture_readiness(
    readiness: Res<NativeShadowCaptureReadiness>,
    materials: Res<bevy::pbr::RenderMaterialBindings>,
    pipelines: Res<bevy::render::render_resource::PipelineCache>,
) {
    let ready = !materials.is_empty()
        && pipelines.pipelines().all(|p| {
            matches!(
                p.state,
                bevy::render::render_resource::CachedPipelineState::Ok(_)
            )
        });
    if ready {
        readiness
            .0
            .fetch_add(1, std::sync::atomic::Ordering::Release);
    } else {
        readiness.0.store(0, std::sync::atomic::Ordering::Release);
    }
}

fn prepare(
    frame: Res<NativeLocalShadowSceneFrame>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<LocalGpu>,
) {
    let Some(frame) = &frame.0 else {
        return;
    };
    if gpu.programs.last_revision == Some(frame.revision) {
        return;
    }
    upload_native_shadow_uniform_ring(&frame.layers, &device, &queue, &mut gpu.programs);
}

fn draw(
    frame: Res<NativeLocalShadowSceneFrame>,
    completion: Res<NativeLocalShadowCompletion>,
    mut gpu: ResMut<LocalGpu>,
    allocator: Res<MeshAllocator>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    device: Res<RenderDevice>,
    adapter: Res<RenderAdapter>,
    mut context: RenderContext,
    sampling: Option<Res<crate::shadow_sampling::NativeLocalSamplingBuffer>>,
    queue: Res<RenderQueue>,
    light_indices: Res<bevy::pbr::GlobalClusterableObjectMeta>,
    entities: Query<
        (Entity, &bevy::render::sync_world::MainEntity),
        With<bevy::pbr::ExtractedPointLight>,
    >,
    profile: Option<Res<crate::frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(28));
    let sampling = sampling
        .as_deref()
        .and_then(|s| buffers.get(s.buffer.id()).map(|b| (s, b)));
    let Some(frame) = &frame.0 else {
        if let Some((_, buffer)) = sampling {
            queue.write_buffer(&buffer.buffer, 0, &0u32.to_le_bytes());
        }
        return;
    };
    if gpu.programs.last_revision == Some(frame.revision) {
        return;
    }
    if let Some((_, buffer)) = sampling {
        queue.write_buffer(&buffer.buffer, 0, &0u32.to_le_bytes());
    }
    if frame.layers.len() != frame.regions.len() {
        bevy::log::error!("native local region/draw count mismatch");
        return;
    }
    let (Some(atlas), Some(cache)) = (images.get(frame.atlas.id()), images.get(frame.cache.id()))
    else {
        return;
    };
    let format = parallel_target_format(frame.depth_32_bits);
    for (image, extent) in [(atlas, frame.atlas_extent), (cache, frame.cache_extent)] {
        if image.texture.format() != format
            || image.texture.size()
                != (bevy::render::render_resource::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: extent[2],
                })
        {
            bevy::log::error!("native local atlas allocation differs from its owner");
            return;
        }
    }
    let replace = gpu.arrays.as_ref().is_none_or(|(a, c)| {
        a.texture.id() != atlas.texture.id() || c.texture.id() != cache.texture.id()
    });
    if replace {
        let result =
            NativeShadowDepthArray::from_owned_texture(atlas.texture.clone(), "native local atlas")
                .and_then(|a| {
                    Ok((
                        a,
                        NativeShadowDepthArray::from_owned_texture(
                            cache.texture.clone(),
                            "native local static cache",
                        )?,
                    ))
                });
        match result {
            Ok(arrays) => {
                gpu.arrays = Some(arrays);
                gpu.valid.clear();
                gpu.copy = None;
            }
            Err(error) => {
                bevy::log::error!("{error}");
                return;
            }
        }
    }
    if gpu
        .copy
        .as_ref()
        .is_none_or(|(state, _)| *state != frame.copy_state)
    {
        match NativeShadowDepthCopy::new(&device, format, frame.copy_state, false) {
            Ok(copy) => gpu.copy = Some((frame.copy_state, copy)),
            Err(error) => {
                bevy::log::error!("{error}");
                return;
            }
        }
    }
    if frame
        .regions
        .iter()
        .any(|r| r.cache.is_some_and(|c| !gpu.valid.contains_key(&c.id)) && !r.static_prepared)
    {
        // A recreated GPU texture invalidates a previous CPU acknowledgement.
        // Request the retained source generation again before touching pixels.
        if let Ok(mut done) = completion.0.lock() {
            *done = None;
        }
        return;
    }
    let Some(uniform) = gpu.programs.uniform_ring.clone() else {
        return;
    };
    let stride = gpu.programs.uniform_stride;
    let Some(records) = prepare_native_shadow_records(
        &frame.layers,
        frame.depth_32_bits,
        None,
        uniform,
        stride,
        &mut gpu.programs,
        &allocator,
        &meshes,
        &images,
        &buffers,
        &device,
        &adapter,
    ) else {
        return;
    };
    let idle_atlas = (std::env::var("DH2_REUSE_LOCAL_SHADOW_ATLAS").as_deref() != Ok("0"))
        .then(|| {
            RetainedLocalAtlas::idle(frame, atlas.texture.id(), cache.texture.id(), &gpu.valid)
        })
        .flatten();
    let reuse_atlas = idle_atlas.is_some() && gpu.retained_atlas.as_ref() == idle_atlas.as_ref();
    // Invalidate before recording: even a partially recorded failed submission
    // can touch the atlas. Only a fully successful frame may establish reuse.
    gpu.retained_atlas = None;
    let LocalGpu {
        programs,
        arrays,
        copy,
        valid,
        retained_atlas,
        atlas_reused,
        ..
    } = &mut *gpu;
    let (atlas, cache) = arrays.as_ref().unwrap();
    let copy = &copy.as_ref().unwrap().1;
    let result = if reuse_atlas {
        *atlas_reused += 1;
        if *atlas_reused % 300 == 1 && std::env::var_os("DH2_TRACE_LOCAL_ATLAS_REUSE").is_some() {
            eprintln!("LOCAL_ATLAS_REUSE idle_frames={}", *atlas_reused);
        }
        Ok(())
    } else {
        let diagnostics = context.diagnostic_recorder();
        let diagnostics = diagnostics.as_deref();
        let span = diagnostics.time_span(context.command_encoder(), "native_local_shadows");
        let encoder = context.command_encoder();
        //32d6e0 clears each complete atlas slice once, before static regions.
        let clear = NativeShadowLayerOps {
            depth: NativeShadowLoad::Clear(1.0),
            stencil: NativeStencilLoad::Clear(0),
        };
        for slice in 0..atlas.size.depth_or_array_layers {
            clear_empty_layer(encoder, atlas, slice, clear);
        }
        if replace {
            for slice in 0..cache.size.depth_or_array_layers {
                clear_empty_layer(encoder, cache, slice, clear);
            }
        }
        let result = (|| -> Result<(), String> {
            for (region_index, region) in frame.regions.iter().enumerate() {
                let mut draws = Vec::new();
                for record in records.iter().filter(|record| record.layer == region_index) {
                    draws.push((record.pipeline, record.draw(&programs.pipelines)));
                }
                if region.static_instances > draws.len() {
                    return Err("native local static/dynamic boundary exceeds draws".into());
                }
                let mut dynamic = draws.split_off(region.static_instances);
                if let Some(allocation) = region.cache {
                    if !valid.contains_key(&allocation.id) {
                        // A newly assigned rectangle can contain an older light.
                        // Initialize it from this frame's cleared atlas region
                        // before loading the cache attachment for caster draws.
                        copy.copy(
                            &device,
                            encoder,
                            atlas,
                            region.atlas.slice as u32,
                            region.atlas.rect,
                            cache,
                            allocation.slice as u32,
                            allocation.rect,
                        )?;
                        draw_region(encoder, cache, allocation, &programs.pipelines, &mut draws)?;
                        valid.insert(allocation.id, region.contains_models);
                    }
                    copy.copy(
                        &device,
                        encoder,
                        cache,
                        allocation.slice as u32,
                        allocation.rect,
                        atlas,
                        region.atlas.slice as u32,
                        region.atlas.rect,
                    )?;
                } else {
                    draw_region(
                        encoder,
                        atlas,
                        region.atlas,
                        &programs.pipelines,
                        &mut draws,
                    )?;
                }
                draw_region(
                    encoder,
                    atlas,
                    region.atlas,
                    &programs.pipelines,
                    &mut dynamic,
                )?;
            }
            Ok(())
        })();
        span.end(context.command_encoder());
        result
    };
    if let Err(error) = result {
        bevy::log::error!("native local atlas submission failed: {error}");
        return;
    }
    *retained_atlas = idle_atlas;
    valid.retain(|id, _| {
        frame
            .regions
            .iter()
            .any(|r| r.cache.is_some_and(|c| c.id == *id))
    });
    if let Ok(mut done) = completion.0.lock() {
        *done = Some((
            frame.revision,
            frame.atlas.id(),
            frame.cache.id(),
            valid.clone(),
        ));
    }
    if programs.last_revision.is_none() {
        println!(
            "Native local shadow GPU atlas online: {} regions, {} completed static cache allocations",
            frame.regions.len(),
            valid.len()
        );
    }
    programs.last_revision = Some(frame.revision);
    let result = (|| -> Result<(), String> {
        let (sampling, buffer) =
            sampling.ok_or("native local receiver transport awaits GPU preparation")?;
        let receiver = frame.receiver.as_ref().map_err(Clone::clone)?;
        let capacity = sampling.light_capacity as usize;
        let mut words =
            vec![0; crate::shadow_sampling::local_sampling_word_count(sampling.light_capacity)];
        words[0] = u32::from(std::env::var_os("DH2_DISABLE_NATIVE_LOCAL_RECEIVER").is_none());
        words[1] = sampling.light_capacity;
        words[2] = receiver.shadows.len() as u32;
        words[4..6].copy_from_slice(
            &[frame.atlas_extent[0] as f32, frame.atlas_extent[1] as f32].map(f32::to_bits),
        );
        words[8..11].copy_from_slice(&receiver.bias.map(f32::to_bits));
        for i in 0..capacity {
            words[16 + i * 16 + 1] = u32::MAX;
        }
        let mapped = entities
            .iter()
            .filter_map(|(entity, main)| {
                light_indices
                    .entity_to_index
                    .get(&entity)
                    .map(|&index| (main.id(), index))
            })
            .collect::<HashMap<_, _>>();
        let mut matched = 0;
        for light in &receiver.lights {
            let Some(&index) = mapped.get(&light.entity) else {
                continue;
            };
            if index >= capacity {
                return Err(
                    "native local receiver light index exceeds its owned transport capacity".into(),
                );
            }
            let record = &mut words[16 + index * 16..16 + (index + 1) * 16];
            record[..3].copy_from_slice(&[light.kind, light.first, light.count]);
            for (offset, value) in [(4, light.position), (8, light.left), (12, light.direction)] {
                record[offset..offset + 3].copy_from_slice(&value.map(f32::to_bits));
            }
            matched += 1;
        }
        let start = 16 + capacity * 16;
        for (i, shadow) in receiver.shadows.iter().enumerate() {
            words[start + i * 36..start + (i + 1) * 36].copy_from_slice(shadow);
        }
        let bytes = words
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        if !gpu.receiver_reported {
            if let Some(path) = std::env::var_os("DH2_CAPTURE_LOCAL_RECEIVER") {
                std::fs::write(path, &bytes)
                    .map_err(|e| format!("native local receiver capture: {e}"))?;
            }
        }
        if buffer.buffer.size() < bytes.len() as u64 {
            return Err(
                "native local receiver GPU buffer is smaller than the published schema".into(),
            );
        }
        queue.write_buffer(&buffer.buffer, 0, &bytes);
        if !gpu.receiver_reported {
            println!(
                "Native local shadow receiver online: {matched} actual clustered light indices, {} native shadow records",
                receiver.shadows.len()
            );
            gpu.receiver_reported = true;
        }
        Ok(())
    })();
    match result {
        Ok(()) => gpu.receiver_error = None,
        Err(error) => {
            if gpu.receiver_error.as_ref() != Some(&error) {
                bevy::log::warn!("Native local shadow receiver pending: {error}");
                gpu.receiver_error = Some(error);
            }
        }
    }
}

fn draw_region(
    encoder: &mut bevy::render::render_resource::CommandEncoder,
    target: &NativeShadowDepthArray,
    allocation: AtlasAllocation,
    pipelines: &ShadowPipelineStore,
    draws: &mut [(PipelineId, NativeOpaqueShadowDraw<'_>)],
) -> Result<(), String> {
    let [x, y, x1, y1] = allocation.rect;
    let scissor = NativeShadowScissor {
        x: x as u32,
        y: y as u32,
        width: (x1 - x + 1) as u32,
        height: (y1 - y + 1) as u32,
    };
    for (_, draw) in draws.iter_mut() {
        draw.scissor = scissor;
    }
    let sequence = draws
        .iter()
        .map(|(key, draw)| (&pipelines.by_id(*key).pipeline, &*draw))
        .collect::<Vec<_>>();
    NativeOpaqueShadowPipeline::draw_layer_region_sequence(
        encoder,
        target,
        allocation.slice as u32,
        scissor,
        &sequence,
    )
}

#[cfg(test)]
#[path = "local/atlas_reuse_tests.rs"]
mod atlas_reuse_tests;
