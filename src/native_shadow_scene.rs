//! Bevy render-world bridge for original opaque, cutout and vegetation shadows.
//!
//! The game thread publishes the already-resolved native shadow state and caster set;
//! this module uploads each program's actual instance block and submits draws against
//! Bevy's existing mesh allocations. It deliberately does not query/cull scene geometry,
//! infer cascade/cache policy, or enable Bevy light shadows.

use crate::native_shadow_gpu::{
    native_shadow_alpha_sampler, NativeOpaqueShadowDraw, NativeOpaqueShadowPipeline,
    NativeShadowDepthArray, NativeShadowScissor,
};
use bevy::{
    app::{App, Plugin},
    asset::{AssetId, Handle},
    core_pipeline::{Core3d, Core3dSystems},
    ecs::prelude::*,
    image::Image,
    mesh::{Mesh, MeshVertexAttribute, MeshVertexBufferLayoutRef},
    render::{
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::{
            BindGroup, Buffer, BufferDescriptor, BufferInitDescriptor, BufferUsages,
            DepthBiasState, Face, FrontFace, LoadOp, Operations, RawVertexBufferLayout,
            RenderPassDepthStencilAttachment, Sampler, StoreOp, TextureView, VertexStepMode,
        },
        renderer::{RenderAdapter, RenderContext, RenderDevice, RenderQueue},
        storage::{GpuShaderBuffer, ShaderBuffer},
        texture::GpuImage,
        Render, RenderApp, RenderSystems,
    },
};
use std::collections::HashMap;

/// A resolved original shader *class*, not a transient RenderDoc resource ID.
/// The opaque class is evidenced by VS hash be596c78-7b23f0b2-921af52f-29b1e602.
/// The CPU owner must resolve the original program bank before selecting it.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum NativeShadowProgram {
    RigidOpaque,
    RigidAlphaTested,
    Vegetation,
    Unsupported(u64),
}

/// Actual raw diffuse image and resolved Layer0 coordinate data from the
/// original material owner. No forward alpha cutoff/tint is used here.
#[derive(Clone, Debug)]
pub struct NativeShadowAlphaMaterial {
    pub diffuse: Handle<Image>,
    pub uv_set: u32,
    pub uv_scale_bias: [f32; 4],
}

pub const NATIVE_SHADOW_INSTANCE_BYTES: usize = 80;
pub const NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES: usize = 208;

/// These attributes distinguish original local vegetation inputs from the
/// forward mesh's baked position, transformed normal and color/tint.
pub const NATIVE_VEGETATION_NORMAL: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeVegetationNormal",
    0x4448_3201,
    bevy::render::render_resource::VertexFormat::Float32x2,
);
pub const NATIVE_VEGETATION_COLOR: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeVegetationColor",
    0x4448_3202,
    bevy::render::render_resource::VertexFormat::Float32x3,
);
pub const NATIVE_VEGETATION_POSITION: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeVegetationPosition",
    0x4448_3203,
    bevy::render::render_resource::VertexFormat::Float32x3,
);

/// The payload ABI is explicit: an 80-byte rigid record cannot become a wind
/// record by padding it or supplying a zero/default clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeShadowInstanceConstants {
    Rigid([u8; NATIVE_SHADOW_INSTANCE_BYTES]),
    Vegetation([u8; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES]),
}

impl NativeShadowInstanceConstants {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Rigid(bytes) => bytes,
            Self::Vegetation(bytes) => bytes,
        }
    }
}

impl From<[u8; NATIVE_SHADOW_INSTANCE_BYTES]> for NativeShadowInstanceConstants {
    fn from(bytes: [u8; NATIVE_SHADOW_INSTANCE_BYTES]) -> Self {
        Self::Rigid(bytes)
    }
}

impl From<[u8; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES]> for NativeShadowInstanceConstants {
    fn from(bytes: [u8; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES]) -> Self {
        Self::Vegetation(bytes)
    }
}

/// Actual resolved vegetanim parameters. Defaults/overrides are resolved by
/// the material owner; none are substituted in the renderer.
#[derive(Clone, Debug)]
pub struct NativeVegetationShadowParameters {
    pub main_variation: [f32; 4],
    pub main_frequency: [f32; 4],
    pub main_amplitude: [f32; 4],
    pub minimum_wind: [f32; 4],
    pub leaf_frequency: [f32; 4],
    pub leaf_amplitude: [f32; 4],
    pub branch_amplitude: [f32; 4],
}

/// Installed VS14200's 208-byte CB: UV, seven wind vectors, Z/Y/X/W rows,
/// then the model's selected localTime float2 and depth multiplier. The time
/// is supplied by its CPU owner, not inferred from Bevy's clock or forced to zero.
pub fn native_vegetation_shadow_instance_cb(
    uv_scale_bias: [f32; 4],
    wind: &NativeVegetationShadowParameters,
    mvp_rows: &[[f32; 4]; 4],
    local_time: [f32; 2],
    model_depth_multiplier: f32,
) -> [u8; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES] {
    let mut bytes = [0; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES];
    let vectors = [
        uv_scale_bias,
        wind.main_variation,
        wind.main_frequency,
        wind.main_amplitude,
        wind.minimum_wind,
        wind.leaf_frequency,
        wind.leaf_amplitude,
        wind.branch_amplitude,
        mvp_rows[2],
        mvp_rows[1],
        mvp_rows[0],
        mvp_rows[3],
    ];
    for (word, value) in vectors
        .iter()
        .flatten()
        .chain(local_time.iter())
        .chain(std::iter::once(&model_depth_multiplier))
        .enumerate()
    {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Encode the bridge's matrix block, not the model owner's transform selection.
/// For opaque this is its native 80-byte W,Z,Y,X/depthhack PerInstanceCB.
/// The verified alpha program instead has a native 96-byte UV/Z/Y/X/W/depthhack
/// layout (VS reads CB2[1..5]): the bridge binds these logical variables using this
/// matrix block plus a separate 16-byte UV block. Do not memcpy a native alpha
/// CB into this function's output or interpret its 80 bytes as that native ABI.
/// Callers supply the original MVP and clip-depth multiplier.
pub fn native_shadow_instance_cb(
    mvp_rows: &[[f32; 4]; 4],
    model_depth_multiplier: f32,
) -> [u8; NATIVE_SHADOW_INSTANCE_BYTES] {
    let mut bytes = [0; NATIVE_SHADOW_INSTANCE_BYTES];
    for (word, value) in mvp_rows
        .iter()
        .rev()
        .flatten()
        .copied()
        .chain(std::iter::once(model_depth_multiplier))
        .enumerate()
    {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Caller-owned target dimensions. No fallback resolution or format downgrade is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeParallelShadowTargetDimensions {
    pub width: u32,
    pub height: u32,
    pub layers: u32,
    pub depth_32_bits: bool,
}

/// The exact receiver/cascade inputs consumed by the original directional shader path.
/// These values are supplied by the native CPU owner; this bridge does not synthesize them.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeParallelReceiverVars {
    /// Resolved original compile branch, not an inferred opaque/alpha flag.
    pub path: crate::render_shadow::ParallelShadowReceiverPath,
    pub world_to_shadow: Vec<[[f32; 4]; 4]>,
    pub view_row2: [f32; 4],
    pub cascade_fars: [f32; 4],
    pub shadow_size: [f32; 2],
    pub light_quality: i32,
    pub filtering_quality: i32,
    /// Chosen sun in the host renderer's directional-light array.
    pub directional_light_index: u32,
    /// Actual original shadowCommon options/parameters, not shader defaults.
    pub offset: crate::native_shadow_sampling::NativeShadowOffsetParameters,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeShadowLoad {
    Load,
    Clear(f32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeStencilLoad {
    Load,
    Clear(u32),
}

/// Per-layer cache policy is explicit: `Clear` executes even for an empty dirty layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeShadowLayerOps {
    pub depth: NativeShadowLoad,
    pub stencil: NativeStencilLoad,
}

/// A single instance of a resolved original model-shadow program.
#[derive(Clone, Debug)]
pub struct NativeShadowInstance {
    pub mesh: Handle<Mesh>,
    /// Program-specific variables from the actual owner. Rigid records use
    /// native_shadow_instance_cb; wind records use the full 208-byte encoder.
    pub per_instance_cb: NativeShadowInstanceConstants,
    /// Shader class resolved from this model's original program bank. A capture
    /// resource number (e.g. 17296) is not an engine program identifier.
    pub program: NativeShadowProgram,
    pub alpha_material: Option<NativeShadowAlphaMaterial>,
    /// Actual mesh representation and resolved original program culling.
    /// Imported game meshes have already converted native CW indices to CCW.
    pub front_face: FrontFace,
    pub cull_mode: Option<Face>,
    pub draw_state: [u32; 9],
    pub scissor: NativeShadowScissor,
}

#[derive(Clone, Debug)]
pub struct NativeShadowLayer {
    pub ops: NativeShadowLayerOps,
    pub instances: Vec<NativeShadowInstance>,
}

#[derive(Clone, Debug)]
pub enum NativeShadowSceneState {
    /// Bindings must resolve to neutral visibility (factor 1); no shadow map is fabricated.
    Disabled,
    /// CPU owner has not yet published a usable target/cascade state.
    Unavailable(String),
    Ready {
        target_image: Handle<Image>,
        dimensions: NativeParallelShadowTargetDimensions,
        receiver: NativeParallelReceiverVars,
        layers: Vec<NativeShadowLayer>,
    },
}

/// Extracted CPU publication. `revision` must increase whenever the owner publishes a new
/// frame/cache generation; repeated Core3d views then share a single render submission.
///
/// `Ready` requirements: target Image handle is persistent; dimensions are the exact
/// caller-owned target dimensions; one draw/receiver record per physical layer,
/// including the auxiliary whole-view shadow prepared by FUN_140334170. It is
/// not another cascade, but ComputeParallelShadow samples it beyond the splits.
/// Receiver uniforms,
/// native model drawstates, bridge matrix/UV blocks and scissor rectangles are already resolved.
/// Rigid opaque/alpha-tested and vegetation classes are supported. Vegetation
/// requires original local positions, dedicated raw normal/color attributes and
/// a full 208-byte instance block. Skinned/cloth/paraboloid programs remain unavailable.
#[derive(Resource, Clone, ExtractResource)]
pub struct NativeShadowSceneFrame {
    pub revision: u64,
    pub state: NativeShadowSceneState,
}

impl Default for NativeShadowSceneFrame {
    fn default() -> Self {
        Self {
            revision: 0,
            state: NativeShadowSceneState::Disabled,
        }
    }
}

#[derive(Resource, Default)]
struct NativeShadowSceneGpu {
    last_revision: Option<u64>,
    targets: HashMap<AssetId<Image>, CachedTarget>,
    pipelines: HashMap<PipelineKey, CachedPipeline>,
    uniform_ring: Option<Buffer>,
    uniform_capacity: usize,
    uniform_stride: u64,
    uniform_bytes: Vec<u8>,
    alpha_sampler: Option<Sampler>,
    sampling_buffer: Option<Buffer>,
    sampling_bytes: Vec<u8>,
}

struct CachedTarget {
    dimensions: NativeParallelShadowTargetDimensions,
    array: NativeShadowDepthArray,
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct PipelineKey {
    layout: u64,
    depth_32_bits: bool,
    front_face: FrontFace,
    cull_mode: Option<Face>,
    program: NativeShadowProgram,
    alpha_uv: Option<u32>,
    constant: i32,
    slope: u32,
    clamp: u32,
}

struct CachedPipeline {
    layout: MeshVertexBufferLayoutRef,
    pipeline: NativeOpaqueShadowPipeline,
    bindings: Vec<BindGroup>,
    alpha_materials: HashMap<(AssetId<Image>, [u32; 4]), CachedAlphaMaterial>,
}

struct CachedAlphaMaterial {
    view: TextureView,
    _uv_buffer: Option<Buffer>,
    binding: BindGroup,
}

pub struct NativeShadowScenePlugin;

impl Plugin for NativeShadowScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeShadowSceneFrame>()
            .add_plugins(ExtractResourcePlugin::<NativeShadowSceneFrame>::default())
            .add_plugins(ExtractResourcePlugin::<
                crate::native_shadow_sampling::NativeParallelSamplingBuffer,
            >::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<NativeShadowSceneGpu>()
            .add_systems(
                Render,
                prepare_shadow_uniform_ring.in_set(RenderSystems::PrepareResources),
            )
            .add_systems(
                Core3d,
                render_native_shadow_scene.before(Core3dSystems::MainPass),
            );
    }
}

fn prepare_shadow_uniform_ring(
    frame: Res<NativeShadowSceneFrame>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<NativeShadowSceneGpu>,
) {
    // Repeated views/extractions of the same cached owner generation do not
    // rewrite every instance CB. Failed submissions retain the old revision
    // and therefore still retry their preparation.
    if gpu.last_revision == Some(frame.revision) {
        return;
    }
    let NativeShadowSceneState::Ready { layers, .. } = &frame.state else {
        return;
    };
    let draw_count: usize = layers.iter().map(|layer| layer.instances.len()).sum();
    let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
    let stride = instance_uniform_stride(layers, alignment);
    let needed = draw_count.max(1);
    let grow =
        gpu.uniform_ring.is_none() || gpu.uniform_capacity < needed || gpu.uniform_stride != stride;
    if grow {
        let capacity = needed.next_power_of_two();
        let Some(size) = stride.checked_mul(capacity as u64) else {
            return;
        };
        gpu.uniform_ring = Some(device.create_buffer(&BufferDescriptor {
            label: Some("native shadow program-specific instance ring"),
            size,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        gpu.uniform_capacity = capacity;
        gpu.uniform_stride = stride;
        for cached in gpu.pipelines.values_mut() {
            cached.bindings.clear();
        }
    }
    let Some(buffer) = gpu.uniform_ring.as_ref().cloned() else {
        return;
    };
    gpu.uniform_bytes
        .resize(draw_count.saturating_mul(stride as usize), 0);
    for (slot, instance) in layers.iter().flat_map(|layer| &layer.instances).enumerate() {
        let offset = slot * stride as usize;
        let bytes = instance.per_instance_cb.as_bytes();
        gpu.uniform_bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
    }
    if !gpu.uniform_bytes.is_empty() {
        queue.write_buffer(&buffer, 0, &gpu.uniform_bytes);
    }
}

fn render_native_shadow_scene(
    frame: Res<NativeShadowSceneFrame>,
    mut gpu: ResMut<NativeShadowSceneGpu>,
    allocator: Res<MeshAllocator>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    sampling_handle: Option<Res<crate::native_shadow_sampling::NativeParallelSamplingBuffer>>,
    sampling_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    adapter: Res<RenderAdapter>,
    mut context: RenderContext,
) {
    let sampling = sampling_handle
        .as_ref()
        .and_then(|handle| sampling_buffers.get(handle.buffer.id()));
    let NativeShadowSceneState::Ready {
        target_image,
        dimensions,
        receiver,
        layers,
    } = &frame.state
    else {
        publish_sampling(
            &mut gpu,
            sampling,
            &queue,
            crate::native_shadow_sampling::NativeParallelSamplingUniform::default(),
        );
        gpu.last_revision = None;
        return;
    };
    if gpu.last_revision == Some(frame.revision) {
        return;
    }
    // No new receiver rows may sample an incomplete or failed atlas submission.
    publish_sampling(
        &mut gpu,
        sampling,
        &queue,
        crate::native_shadow_sampling::NativeParallelSamplingUniform::default(),
    );
    let Some(sampling) = sampling else {
        return;
    }; // Retry after PrepareAssets.
    if let Err(error) = validate_frame(*dimensions, receiver, layers) {
        bevy::log::error!("native shadow frame rejected: {error}");
        return;
    }

    let target_id = target_image.id();
    let Some(image) = images.get(target_id) else {
        return;
    }; // Retry after PrepareAssets.
    let expected_format = parallel_target_format(dimensions.depth_32_bits);
    let expected_size = bevy::render::render_resource::Extent3d {
        width: dimensions.width,
        height: dimensions.height,
        depth_or_array_layers: dimensions.layers,
    };
    if image.texture.format() != expected_format || image.texture.size() != expected_size {
        bevy::log::error!(
            "native shadow Image allocation disagrees with its owner dimensions/format"
        );
        return;
    }
    let replace = gpu
        .targets
        .get(&target_id)
        .is_none_or(|target| target.array.texture.id() != image.texture.id());
    if replace {
        match NativeShadowDepthArray::from_owned_texture(
            image.texture.clone(),
            "persistent native directional shadow Image",
        ) {
            Ok(array) => {
                gpu.targets.insert(
                    target_id,
                    CachedTarget {
                        dimensions: *dimensions,
                        array,
                    },
                );
            }
            Err(error) => {
                bevy::log::error!("native shadow target unavailable: {error}");
                return;
            }
        }
    }
    let Some(target) = gpu.targets.get(&target_id) else {
        return;
    };
    if target.dimensions != *dimensions {
        bevy::log::error!("native shadow target handle was reused with different dimensions; refusing per-frame recreation");
        return;
    }

    let Some(uniform_buffer) = gpu.uniform_ring.as_ref().cloned() else {
        return;
    };
    let uniform_stride = gpu.uniform_stride;
    let mut records = Vec::new();
    for (slot, (layer_index, instance)) in layers
        .iter()
        .enumerate()
        .flat_map(|(layer_index, layer)| {
            layer
                .instances
                .iter()
                .map(move |instance| (layer_index, instance))
        })
        .enumerate()
    {
        let mesh_id = instance.mesh.id();
        let Some(render_mesh) = meshes.get(mesh_id) else {
            bevy::log::error!(
                "native shadow owner Mesh {:?} has no RenderMesh; refusing an incomplete layer",
                mesh_id
            );
            return;
        };
        let (Some(vertex), Some(index), Some(index_format)) = (
            allocator.mesh_vertex_slice(&mesh_id),
            allocator.mesh_index_slice(&mesh_id),
            render_mesh.index_format(),
        ) else {
            bevy::log::error!(
                "native shadow owner Mesh {:?} has no indexed GPU allocation",
                mesh_id
            );
            return;
        };
        let bias = state_depth_bias(&instance.draw_state);
        let layout_key = hash_layout(&render_mesh.layout);
        let key = PipelineKey {
            layout: layout_key,
            depth_32_bits: dimensions.depth_32_bits,
            front_face: instance.front_face,
            cull_mode: instance.cull_mode,
            program: instance.program,
            alpha_uv: instance
                .alpha_material
                .as_ref()
                .map(|material| material.uv_set),
            constant: bias.constant,
            slope: bias.slope_scale.to_bits(),
            clamp: bias.clamp.to_bits(),
        };
        if let Err(error) = ensure_pipeline(
            &mut gpu,
            key,
            render_mesh.layout.clone(),
            &device,
            &adapter,
            bias,
        ) {
            bevy::log::error!("native shadow pipeline unavailable: {error}");
            return;
        }
        let alpha_sampler = instance.alpha_material.as_ref().map(|_| {
            gpu.alpha_sampler
                .get_or_insert_with(|| native_shadow_alpha_sampler(&device))
                .clone()
        });
        let cached = gpu.pipelines.get_mut(&key).expect("pipeline just inserted");
        let alpha_binding = if let Some(material) = &instance.alpha_material {
            let Some(diffuse) = images.get(material.diffuse.id()) else {
                bevy::log::error!(
                    "native shadow diffuse {:?} has no GPU image; retrying whole layer",
                    material.diffuse.id()
                );
                return;
            };
            match ensure_alpha_material(
                cached,
                instance.program,
                material,
                diffuse,
                alpha_sampler.as_ref().unwrap(),
                &device,
            ) {
                Ok(binding) => Some(binding),
                Err(error) => {
                    bevy::log::error!("native shadow alpha material unavailable: {error}");
                    return;
                }
            }
        } else {
            None
        };
        if let Err(error) =
            ensure_binding_count(cached, &uniform_buffer, &device, uniform_stride, slot + 1)
        {
            bevy::log::error!("native shadow uniform binding unavailable: {error}");
            return;
        }
        records.push((
            layer_index,
            instance,
            key,
            slot,
            vertex.buffer,
            vertex.range.start as i32,
            index.buffer,
            index.range.clone(),
            index_format,
            alpha_binding,
        ));
    }

    for (layer_index, layer) in layers.iter().enumerate() {
        let target = &gpu.targets[&target_id].array;
        let mut remaining = Vec::new();
        for record in records.iter().filter(|record| record.0 == layer_index) {
            let (
                _,
                instance,
                key,
                slot,
                vertex_buffer,
                base_vertex,
                index_buffer,
                indices,
                index_format,
                alpha_binding,
            ) = record;
            let binding = &gpu.pipelines[key].bindings[*slot];
            let draw = NativeOpaqueShadowDraw {
                model_state: binding,
                alpha_material: alpha_binding.as_ref(),
                vertex_buffer,
                index_buffer,
                index_format: *index_format,
                indices: indices.clone(),
                base_vertex: *base_vertex,
                instances: 0..1,
                scissor: instance.scissor,
            };
            remaining.push((*key, draw));
        }
        // Keep one pass per resolved depth-bias state; subsequent groups load the
        // layer, so a dirty clear happens once and an empty dirty layer still clears.
        let mut first = true;
        let mut groups = HashMap::<PipelineKey, Vec<NativeOpaqueShadowDraw<'_>>>::new();
        for (key, draw) in remaining {
            groups.entry(key).or_default().push(draw);
        }
        if groups.is_empty() {
            if matches!(layer.ops.depth, NativeShadowLoad::Clear(_))
                || matches!(layer.ops.stencil, NativeStencilLoad::Clear(_))
            {
                clear_empty_layer(
                    context.command_encoder(),
                    target,
                    layer_index as u32,
                    layer.ops,
                );
            }
            continue;
        }
        for (key, draws) in groups {
            let ops = layer_operations(
                layer.ops,
                first,
                target.texture.format().has_stencil_aspect(),
            );
            first = false;
            if let Some(cached) = gpu.pipelines.get(&key) {
                if let Err(error) = cached.pipeline.draw_layer(
                    context.command_encoder(),
                    target,
                    layer_index as u32,
                    ops.0,
                    ops.1,
                    &draws,
                ) {
                    bevy::log::error!("native shadow draw failed: {error}");
                    return;
                }
            }
        }
    }
    // Only the fully submitted owner generation may become a forward receiver.
    // Texture and bind-group handles stay stable while these 432 bytes change.
    let sampling_value =
        crate::native_shadow_sampling::NativeParallelSamplingUniform::from_receiver(receiver)
            .expect("receiver validated before submitting any layer");
    publish_sampling(&mut gpu, Some(sampling), &queue, sampling_value);
    // Do not cache missing assets or failed submissions; retry that same owner revision.
    gpu.last_revision = Some(frame.revision);
}

fn publish_sampling(
    gpu: &mut NativeShadowSceneGpu,
    prepared: Option<&GpuShaderBuffer>,
    queue: &RenderQueue,
    value: crate::native_shadow_sampling::NativeParallelSamplingUniform,
) {
    let buffer = prepared
        .map(|value| value.buffer.clone())
        .or_else(|| gpu.sampling_buffer.clone());
    let Some(buffer) = buffer else {
        return;
    };
    let bytes = ShaderBuffer::from(value)
        .data
        .expect("typed receiver transport");
    if buffer.size() < bytes.len() as u64 {
        bevy::log::error!("native receiver transport buffer is smaller than its owner schema");
        return;
    }
    if gpu
        .sampling_buffer
        .as_ref()
        .is_none_or(|old| old.id() != buffer.id())
        || gpu.sampling_bytes != bytes
    {
        queue.write_buffer(&buffer, 0, &bytes);
        gpu.sampling_bytes = bytes;
        gpu.sampling_buffer = Some(buffer);
    }
}

fn validate_frame(
    dimensions: NativeParallelShadowTargetDimensions,
    receiver: &NativeParallelReceiverVars,
    layers: &[NativeShadowLayer],
) -> Result<(), String> {
    if dimensions.width == 0 || dimensions.height == 0 || dimensions.layers == 0 {
        return Err("native shadow target dimensions must be nonzero".into());
    }
    // 36f8a0 allocates N+1: N cascades plus the whole-view shadow from
    // 334170/32de00. shadow::ComputeParallelShadow selects sum(step(fars,Z)),
    // including index N beyond the final split. Never leave that projection or
    // texture layer uninitialized, even though it is not a fifth cascade.
    let cascade_count = dimensions.layers.saturating_sub(1) as usize;
    if !(1..=4).contains(&cascade_count) {
        return Err(
            "native directional target requires 1..=4 cascades plus its auxiliary shadow layer"
                .into(),
        );
    }
    if layers.len() != dimensions.layers as usize
        || receiver.world_to_shadow.len() != dimensions.layers as usize
    {
        return Err("native shadow frame requires draw and receiver records for cascades AND the auxiliary layer".into());
    }
    if receiver.cascade_fars[cascade_count..]
        .iter()
        .any(|&far| far != f32::MAX)
    {
        return Err(
            "native unused cascade splits must retain their initialized FLT_MAX values".into(),
        );
    }
    if receiver.shadow_size != [dimensions.width as f32, dimensions.height as f32] {
        return Err("native directional receiver size does not match target dimensions".into());
    }
    crate::native_shadow_sampling::NativeParallelSamplingUniform::from_receiver(receiver)?;
    if !receiver
        .shadow_size
        .iter()
        .all(|v| v.is_finite() && *v > 0.0)
        || !receiver.view_row2.iter().all(|v| v.is_finite())
        || !receiver.cascade_fars.iter().all(|v| v.is_finite())
    {
        return Err(
            "native directional receiver variables must be finite and have positive shadow size"
                .into(),
        );
    }
    for layer in layers {
        for instance in &layer.instances {
            match (instance.program, &instance.alpha_material) {
                (NativeShadowProgram::RigidOpaque, None) => {}
                (
                    NativeShadowProgram::RigidAlphaTested | NativeShadowProgram::Vegetation,
                    Some(material),
                ) => {
                    if material.uv_set > 1 || !material.uv_scale_bias.iter().all(|v| v.is_finite())
                    {
                        return Err("native alpha shadow needs a resolved UV0/UV1 stream and finite scale/bias".into());
                    }
                }
                _ => {
                    return Err(format!(
                        "unsupported or unresolved native caster shader class {:?}",
                        instance.program
                    ))
                }
            }
            match (&instance.per_instance_cb, instance.program) {
                (
                    NativeShadowInstanceConstants::Rigid(_),
                    NativeShadowProgram::RigidOpaque | NativeShadowProgram::RigidAlphaTested,
                ) => {}
                (
                    NativeShadowInstanceConstants::Vegetation(bytes),
                    NativeShadowProgram::Vegetation,
                ) => {
                    let material = instance
                        .alpha_material
                        .as_ref()
                        .expect("material checked above");
                    if bytes[..16]
                        .chunks_exact(4)
                        .zip(material.uv_scale_bias)
                        .any(|(bytes, value)| bytes != value.to_le_bytes())
                    {
                        return Err("native vegetation UV variables disagree with its 208-byte instance record".into());
                    }
                }
                _ => {
                    return Err(
                        "native shadow program and instance constant layout disagree".into(),
                    )
                }
            }
            let scissor = instance.scissor;
            if scissor.width == 0
                || scissor.height == 0
                || scissor
                    .x
                    .checked_add(scissor.width)
                    .is_none_or(|right| right > dimensions.width)
                || scissor
                    .y
                    .checked_add(scissor.height)
                    .is_none_or(|bottom| bottom > dimensions.height)
            {
                return Err(format!("native shadow scissor outside target: {scissor:?}"));
            }
        }
    }
    Ok(())
}

fn instance_uniform_stride(layers: &[NativeShadowLayer], alignment: u64) -> u64 {
    let bytes = layers
        .iter()
        .flat_map(|layer| &layer.instances)
        .map(|instance| instance.per_instance_cb.as_bytes().len() as u64)
        .max()
        .unwrap_or(NATIVE_SHADOW_INSTANCE_BYTES as u64);
    let alignment = alignment.max(1);
    bytes.div_ceil(alignment) * alignment
}

fn state_depth_bias(state: &[u32; 9]) -> DepthBiasState {
    DepthBiasState {
        constant: state[6] as i32,
        clamp: f32::from_bits(state[7]),
        slope_scale: f32::from_bits(state[8]),
    }
}

fn hash_layout(layout: &MeshVertexBufferLayoutRef) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    layout.hash(&mut hasher);
    hasher.finish()
}

fn ensure_pipeline(
    gpu: &mut NativeShadowSceneGpu,
    key: PipelineKey,
    layout: MeshVertexBufferLayoutRef,
    device: &RenderDevice,
    adapter: &RenderAdapter,
    bias: DepthBiasState,
) -> Result<(), String> {
    if gpu.pipelines.contains_key(&key) {
        return Ok(());
    }
    let position = if key.program == NativeShadowProgram::Vegetation {
        NATIVE_VEGETATION_POSITION
    } else {
        Mesh::ATTRIBUTE_POSITION
    };
    let mut attributes = vec![position.at_shader_location(0)];
    if let Some(uv_set) = key.alpha_uv {
        attributes.push(
            match uv_set {
                0 => Mesh::ATTRIBUTE_UV_0,
                1 => Mesh::ATTRIBUTE_UV_1,
                _ => return Err("unsupported native layer-zero shadow UV stream".into()),
            }
            .at_shader_location(1),
        );
    }
    if key.program == NativeShadowProgram::Vegetation {
        attributes.push(NATIVE_VEGETATION_COLOR.at_shader_location(2));
        attributes.push(NATIVE_VEGETATION_NORMAL.at_shader_location(3));
    }
    let position_layout = layout.0.get_layout(&attributes).map_err(|error| {
        format!("native caster mesh lacks position/selected UV stream: {error}")
    })?;
    let raw = RawVertexBufferLayout {
        array_stride: position_layout.array_stride,
        step_mode: if position_layout.step_mode == VertexStepMode::Vertex {
            bevy::render::render_resource::VertexStepMode::Vertex
        } else {
            return Err("native opaque shadow position stream must be per-vertex".into());
        },
        attributes: &position_layout.attributes,
    };
    let constructor = match key.program {
        NativeShadowProgram::RigidOpaque => NativeOpaqueShadowPipeline::new_with_raster_state,
        NativeShadowProgram::RigidAlphaTested => {
            NativeOpaqueShadowPipeline::new_alpha_with_raster_state
        }
        NativeShadowProgram::Vegetation => {
            NativeOpaqueShadowPipeline::new_vegetation_with_raster_state
        }
        NativeShadowProgram::Unsupported(_) => {
            return Err("unresolved native shadow program".into())
        }
    };
    let pipeline = constructor(
        device,
        adapter,
        &raw,
        bias,
        parallel_target_format(key.depth_32_bits),
        key.front_face,
        key.cull_mode,
    )?;
    gpu.pipelines.insert(
        key,
        CachedPipeline {
            layout,
            pipeline,
            bindings: Vec::new(),
            alpha_materials: HashMap::new(),
        },
    );
    Ok(())
}

fn ensure_alpha_material(
    cached: &mut CachedPipeline,
    program: NativeShadowProgram,
    material: &NativeShadowAlphaMaterial,
    image: &GpuImage,
    sampler: &Sampler,
    device: &RenderDevice,
) -> Result<BindGroup, String> {
    let size = image.texture.size();
    if image.texture.dimension() != bevy::render::render_resource::TextureDimension::D2
        || size.depth_or_array_layers != 1
        || image
            .texture_view_descriptor
            .as_ref()
            .and_then(|view| view.dimension)
            .is_some_and(|dimension| {
                dimension != bevy::render::render_resource::TextureViewDimension::D2
            })
    {
        return Err("native rigid shadow diffuse requires the resolved 2D image".into());
    }
    // Vegetation reads its UV variables from the instance CB, not a fabricated
    // second UV uniform. Its material binding can be shared across model clocks.
    let uv_key = if program == NativeShadowProgram::Vegetation {
        [0; 4]
    } else {
        material.uv_scale_bias.map(f32::to_bits)
    };
    let key = (material.diffuse.id(), uv_key);
    if cached
        .alpha_materials
        .get(&key)
        .is_none_or(|entry| entry.view.id() != image.texture_view.id())
    {
        let (uv_buffer, binding) = match program {
            NativeShadowProgram::RigidAlphaTested => {
                let uv_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("persistent native layer-zero shadow scale/bias"),
                    contents: &material
                        .uv_scale_bias
                        .iter()
                        .flat_map(|v| v.to_le_bytes())
                        .collect::<Vec<_>>(),
                    usage: BufferUsages::UNIFORM,
                });
                let binding = cached.pipeline.bind_alpha_material(
                    device,
                    &image.texture_view,
                    sampler,
                    &uv_buffer,
                )?;
                (Some(uv_buffer), binding)
            }
            NativeShadowProgram::Vegetation => (
                None,
                cached
                    .pipeline
                    .bind_vegetation_material(device, &image.texture_view, sampler)?,
            ),
            _ => return Err("native shadow material requires a resolved cutout program".into()),
        };
        cached.alpha_materials.insert(
            key,
            CachedAlphaMaterial {
                view: image.texture_view.clone(),
                _uv_buffer: uv_buffer,
                binding,
            },
        );
    }
    Ok(cached.alpha_materials[&key].binding.clone())
}

fn ensure_binding_count(
    cached: &mut CachedPipeline,
    buffer: &Buffer,
    device: &RenderDevice,
    stride: u64,
    needed: usize,
) -> Result<(), String> {
    while cached.bindings.len() < needed {
        let slot = cached.bindings.len();
        cached
            .bindings
            .push(cached.pipeline.bind_prepared_model_state_at(
                device,
                buffer,
                slot as u64 * stride,
            )?);
    }
    Ok(())
}

fn parallel_target_format(depth_32_bits: bool) -> bevy::render::render_resource::TextureFormat {
    use bevy::render::render_resource::TextureFormat;
    if depth_32_bits {
        TextureFormat::Depth24PlusStencil8
    } else {
        TextureFormat::Depth16Unorm
    }
}

fn layer_operations(
    ops: NativeShadowLayerOps,
    first: bool,
    has_stencil: bool,
) -> (Operations<f32>, Option<Operations<u32>>) {
    let depth = if first {
        match ops.depth {
            NativeShadowLoad::Load => Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
            NativeShadowLoad::Clear(value) => Operations {
                load: LoadOp::Clear(value),
                store: StoreOp::Store,
            },
        }
    } else {
        Operations {
            load: LoadOp::Load,
            store: StoreOp::Store,
        }
    };
    let stencil = if !has_stencil {
        None
    } else if first {
        Some(match ops.stencil {
            NativeStencilLoad::Load => Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
            NativeStencilLoad::Clear(value) => Operations {
                load: LoadOp::Clear(value),
                store: StoreOp::Store,
            },
        })
    } else {
        Some(Operations {
            load: LoadOp::Load,
            store: StoreOp::Store,
        })
    };
    (depth, stencil)
}

fn clear_empty_layer(
    encoder: &mut bevy::render::render_resource::CommandEncoder,
    target: &NativeShadowDepthArray,
    layer: u32,
    ops: NativeShadowLayerOps,
) {
    let Some(view) = target.layers.get(layer as usize) else {
        return;
    };
    let depth = match ops.depth {
        NativeShadowLoad::Load => LoadOp::Load,
        NativeShadowLoad::Clear(value) => LoadOp::Clear(value),
    };
    let stencil = match ops.stencil {
        NativeStencilLoad::Load => LoadOp::Load,
        NativeStencilLoad::Clear(value) => LoadOp::Clear(value),
    };
    let _pass = encoder.begin_render_pass(&bevy::render::render_resource::RenderPassDescriptor {
        label: Some("native shadow explicit empty-layer operation"),
        color_attachments: &[],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(Operations {
                load: depth,
                store: StoreOp::Store,
            }),
            stencil_ops: target
                .texture
                .format()
                .has_stencil_aspect()
                .then_some(Operations {
                    load: stencil,
                    store: StoreOp::Store,
                }),
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rigid_instance_cb_preserves_reflected_row_order_and_float_bits() {
        let rows = std::array::from_fn(|row| {
            std::array::from_fn(|column| f32::from_bits(0x3f80_0000 + (row * 4 + column) as u32))
        });
        let multiplier = f32::from_bits(0x8000_0000);
        let bytes = native_shadow_instance_cb(&rows, multiplier);
        for (word, value) in rows.iter().rev().flatten().enumerate() {
            assert_eq!(&bytes[word * 4..word * 4 + 4], &value.to_le_bytes());
        }
        assert_eq!(&bytes[64..68], &multiplier.to_le_bytes());
        assert_eq!(&bytes[68..], &[0; 12]);
    }

    #[test]
    fn vegetation_instance_cb_keeps_native_layout_and_model_time() {
        let wind = NativeVegetationShadowParameters {
            main_variation: [1.0; 4],
            main_frequency: [2.0; 4],
            main_amplitude: [3.0; 4],
            minimum_wind: [4.0; 4],
            leaf_frequency: [5.0; 4],
            leaf_amplitude: [6.0; 4],
            branch_amplitude: [7.0; 4],
        };
        let rows = [[8.0; 4], [9.0; 4], [10.0; 4], [11.0; 4]];
        let uv = [0.5, 2.0, -0.0, -1.0];
        let time = [16_777_218.0, -0.0];
        let cb = native_vegetation_shadow_instance_cb(uv, &wind, &rows, time, 0.75);
        let vectors = [
            uv,
            wind.main_variation,
            wind.main_frequency,
            wind.main_amplitude,
            wind.minimum_wind,
            wind.leaf_frequency,
            wind.leaf_amplitude,
            wind.branch_amplitude,
            rows[2],
            rows[1],
            rows[0],
            rows[3],
        ];
        let expected: Vec<u8> = vectors
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(&cb[..192], &expected);
        assert_eq!(&cb[192..196], &time[0].to_le_bytes());
        assert_eq!(&cb[196..200], &time[1].to_le_bytes());
        assert_eq!(&cb[200..204], &0.75f32.to_le_bytes());
        assert_eq!(&cb[204..], &[0; 4]);

        let instance = NativeShadowInstance {
            mesh: Handle::default(),
            per_instance_cb: cb.into(),
            program: NativeShadowProgram::Vegetation,
            alpha_material: Some(NativeShadowAlphaMaterial {
                diffuse: Handle::default(),
                uv_set: 0,
                uv_scale_bias: uv,
            }),
            front_face: FrontFace::Ccw,
            cull_mode: Some(Face::Back),
            draw_state: [0; 9],
            scissor: NativeShadowScissor {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
        };
        let mut layers = vec![
            NativeShadowLayer {
                ops: NativeShadowLayerOps {
                    depth: NativeShadowLoad::Load,
                    stencil: NativeStencilLoad::Load
                },
                instances: vec![instance],
            };
            2
        ];
        let dimensions = NativeParallelShadowTargetDimensions {
            width: 32,
            height: 32,
            layers: 2,
            depth_32_bits: false,
        };
        validate_frame(dimensions, &receiver(), &layers).unwrap();
        for alignment in [1, 16, 64, 256, 512] {
            assert_eq!(
                instance_uniform_stride(&layers, alignment),
                208u64.div_ceil(alignment) * alignment
            );
        }
        layers[0].instances[0].per_instance_cb = [0; NATIVE_SHADOW_INSTANCE_BYTES].into();
        assert!(validate_frame(dimensions, &receiver(), &layers)
            .unwrap_err()
            .contains("constant layout"));
        layers[0].instances[0].per_instance_cb = cb.into();
        layers[0].instances[0]
            .alpha_material
            .as_mut()
            .unwrap()
            .uv_scale_bias[0] = 1.0;
        assert!(validate_frame(dimensions, &receiver(), &layers)
            .unwrap_err()
            .contains("UV variables"));
        layers[0].instances[0]
            .alpha_material
            .as_mut()
            .unwrap()
            .uv_scale_bias = uv;
        layers[0].instances[0].program = NativeShadowProgram::RigidAlphaTested;
        assert!(validate_frame(dimensions, &receiver(), &layers)
            .unwrap_err()
            .contains("constant layout"));
        layers[0].instances[0].per_instance_cb = [0; NATIVE_SHADOW_INSTANCE_BYTES].into();
        validate_frame(dimensions, &receiver(), &layers).unwrap();
        assert_eq!(
            instance_uniform_stride(&layers, 64),
            256,
            "rigid+wind use the same actual ring stride"
        );
        layers[1].instances.clear();
        assert_eq!(instance_uniform_stride(&layers, 64), 128);
    }

    #[test]
    #[ignore = "requires GPU; verifies render-world mixed instance ring and native material routing"]
    fn native_scene_program_uniform_routing_gpu() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::prelude::*;
        use bevy::render::{render_resource::*, RenderApp};
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        );
        app.finish();
        app.cleanup();
        let world = app.sub_app_mut(RenderApp).world_mut();
        let material = NativeShadowAlphaMaterial {
            diffuse: Handle::default(),
            uv_set: 0,
            uv_scale_bias: [2.0, 0.5, -0.25, 0.125],
        };
        // Synthetic owner records test ABI/routing, not a fabricated live frame.
        let make_instance = |program, constants| NativeShadowInstance {
            mesh: Handle::default(),
            per_instance_cb: constants,
            program,
            alpha_material: Some(material.clone()),
            front_face: FrontFace::Ccw,
            cull_mode: None,
            draw_state: [0; 9],
            scissor: NativeShadowScissor {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
        };
        let mut wind_cb = [0x5a; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES];
        for (component, value) in material.uv_scale_bias.iter().enumerate() {
            wind_cb[component * 4..component * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        let rigid_cb = [0xa5; NATIVE_SHADOW_INSTANCE_BYTES];
        let layers = vec![
            NativeShadowLayer {
                ops: NativeShadowLayerOps {
                    depth: NativeShadowLoad::Clear(1.0),
                    stencil: NativeStencilLoad::Load,
                },
                instances: vec![
                    make_instance(NativeShadowProgram::RigidAlphaTested, rigid_cb.into()),
                    make_instance(NativeShadowProgram::Vegetation, wind_cb.into()),
                ],
            },
            NativeShadowLayer {
                ops: NativeShadowLayerOps {
                    depth: NativeShadowLoad::Load,
                    stencil: NativeStencilLoad::Load,
                },
                instances: Vec::new(),
            },
        ];
        world.insert_resource(NativeShadowSceneFrame {
            revision: 1,
            state: NativeShadowSceneState::Ready {
                target_image: Handle::default(),
                dimensions: NativeParallelShadowTargetDimensions {
                    width: 32,
                    height: 32,
                    layers: 2,
                    depth_32_bits: false,
                },
                receiver: receiver(),
                layers,
            },
        });
        world.init_resource::<NativeShadowSceneGpu>();
        world.run_system_once(prepare_shadow_uniform_ring).unwrap();
        let mut gpu = world.remove_resource::<NativeShadowSceneGpu>().unwrap();
        let device = world.resource::<RenderDevice>();
        let adapter = world.resource::<RenderAdapter>();
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
        let stride = gpu.uniform_stride;
        assert_eq!(stride, 208u64.div_ceil(alignment) * alignment);
        assert_eq!(&gpu.uniform_bytes[..80], &rigid_cb);
        assert_eq!(
            &gpu.uniform_bytes[stride as usize..stride as usize + 208],
            &wind_cb
        );
        let ring = gpu.uniform_ring.as_ref().unwrap().clone();

        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[10.0, 20.0, 30.0]; 3]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; 3]);
        mesh.insert_attribute(NATIVE_VEGETATION_POSITION, vec![[0.0, 0.0, 0.0]; 3]);
        mesh.insert_attribute(NATIVE_VEGETATION_COLOR, vec![[0.25, 0.5, 0.75]; 3]);
        mesh.insert_attribute(NATIVE_VEGETATION_NORMAL, vec![[0.0, 1.0]; 3]);
        let mut layouts = bevy::mesh::MeshVertexBufferLayouts::default();
        let layout = mesh.get_mesh_vertex_buffer_layout(&mut layouts);
        let descriptor = TextureDescriptor {
            label: Some("native scene material routing fixture"),
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8Unorm,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        };
        let texture = device.create_texture(&descriptor);
        let sampler = native_shadow_alpha_sampler(device);
        let image = GpuImage {
            texture_view: texture.create_view(&TextureViewDescriptor::default()),
            texture,
            sampler: sampler.clone(),
            texture_descriptor: descriptor,
            texture_view_descriptor: None,
            had_data: false,
        };
        for program in [
            NativeShadowProgram::RigidOpaque,
            NativeShadowProgram::RigidAlphaTested,
            NativeShadowProgram::Vegetation,
        ] {
            let key = PipelineKey {
                layout: hash_layout(&layout),
                depth_32_bits: false,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                program,
                alpha_uv: (program != NativeShadowProgram::RigidOpaque).then_some(0),
                constant: 0,
                slope: 0,
                clamp: 0,
            };
            ensure_pipeline(
                &mut gpu,
                key.clone(),
                layout.clone(),
                device,
                adapter,
                DepthBiasState::default(),
            )
            .unwrap();
            let cached = gpu.pipelines.get_mut(&key).unwrap();
            ensure_binding_count(cached, &ring, device, stride, 2).unwrap();
            assert_eq!(cached.bindings.len(), 2);
            if program != NativeShadowProgram::RigidOpaque {
                ensure_alpha_material(cached, program, &material, &image, &sampler, device)
                    .unwrap();
                assert_eq!(cached.alpha_materials.len(), 1);
                assert_eq!(
                    cached
                        .alpha_materials
                        .values()
                        .next()
                        .unwrap()
                        ._uv_buffer
                        .is_some(),
                    program == NativeShadowProgram::RigidAlphaTested
                );
                if program == NativeShadowProgram::Vegetation {
                    let mut other = material.clone();
                    other.uv_scale_bias[0] = 3.0;
                    ensure_alpha_material(cached, program, &other, &image, &sampler, device)
                        .unwrap();
                    assert_eq!(
                        cached.alpha_materials.len(),
                        1,
                        "wind UV is per-instance, not per-material"
                    );
                }
            }
        }
        assert_eq!(
            gpu.pipelines.len(),
            3,
            "same mesh/layout must not alias different native programs"
        );
        mesh.remove_attribute(NATIVE_VEGETATION_POSITION);
        let missing_raw = mesh.get_mesh_vertex_buffer_layout(&mut layouts);
        let key = PipelineKey {
            layout: hash_layout(&missing_raw),
            depth_32_bits: false,
            front_face: FrontFace::Ccw,
            cull_mode: None,
            program: NativeShadowProgram::Vegetation,
            alpha_uv: Some(0),
            constant: 0,
            slope: 0,
            clamp: 0,
        };
        assert!(
            ensure_pipeline(
                &mut gpu,
                key,
                missing_raw,
                device,
                adapter,
                DepthBiasState::default()
            )
            .is_err(),
            "baked forward POSITION is not a native wind input"
        );
    }

    fn receiver() -> NativeParallelReceiverVars {
        NativeParallelReceiverVars {
            path: crate::render_shadow::ParallelShadowReceiverPath::DirectVisibility,
            world_to_shadow: vec![[[0.0; 4]; 4]; 2],
            view_row2: [0.0; 4],
            cascade_fars: [4.0, f32::MAX, f32::MAX, f32::MAX],
            shadow_size: [32.0; 2],
            light_quality: 0,
            filtering_quality: 0,
            directional_light_index: 0,
            offset: crate::native_shadow_sampling::NativeShadowOffsetParameters {
                sun_direction: [0.0, 0.0, -1.0],
                use_normal_offset: false,
                use_slope_scale: false,
                use_slope_bias: false,
                slope_post_projection: false,
                use_uv_normal_offset: false,
                normal_offset: 0.0,
                bias: 0.0,
                slope_bias: 0.0,
            },
        }
    }

    #[test]
    fn parallel_target_uses_actual_16_or_24_plus_stencil_format() {
        use bevy::render::render_resource::TextureFormat;
        let layer = NativeShadowLayer {
            ops: NativeShadowLayerOps {
                depth: NativeShadowLoad::Clear(1.0),
                stencil: NativeStencilLoad::Clear(0),
            },
            instances: vec![],
        };
        let layers = [layer.clone(), layer];
        for depth_32_bits in [false, true] {
            let dims = NativeParallelShadowTargetDimensions {
                width: 32,
                height: 32,
                layers: 2,
                depth_32_bits,
            };
            validate_frame(dims, &receiver(), &layers).unwrap();
            assert_eq!(
                parallel_target_format(depth_32_bits),
                if depth_32_bits {
                    TextureFormat::Depth24PlusStencil8
                } else {
                    TextureFormat::Depth16Unorm
                }
            );
        }
    }

    #[test]
    fn auxiliary_layer_requires_its_own_draw_and_receiver_records() {
        for cascade_count in 1..=4 {
            let target = crate::render_shadow::ParallelShadowTargetDimensions::from_settings(
                2048,
                0,
                cascade_count,
            )
            .unwrap();
            let dims = NativeParallelShadowTargetDimensions {
                width: target.side,
                height: target.side,
                layers: target.layers,
                depth_32_bits: true,
            };
            let mut vars = receiver();
            vars.world_to_shadow = vec![[[0.0; 4]; 4]; target.layers as usize];
            vars.cascade_fars = [f32::MAX; 4];
            vars.cascade_fars[..cascade_count as usize]
                .copy_from_slice(&[4.0, 12.0, 30.0, 75.0][..cascade_count as usize]);
            vars.shadow_size = [target.side as f32; 2];
            let mut draws = vec![
                NativeShadowLayer {
                    ops: NativeShadowLayerOps {
                        depth: NativeShadowLoad::Load,
                        stencil: NativeStencilLoad::Load
                    },
                    instances: vec![],
                };
                target.layers as usize
            ];
            validate_frame(dims, &vars, &draws).unwrap();
            // The native step sum reaches exactly the auxiliary layer after
            // the last real split (unused splits remain FLT_MAX).
            let depth = vars.cascade_fars[cascade_count as usize - 1] + 1.0;
            let selected = vars
                .cascade_fars
                .iter()
                .filter(|&&far| far <= depth)
                .count();
            assert_eq!(selected, cascade_count as usize);
            assert!(selected < vars.world_to_shadow.len());
            draws.pop();
            assert!(
                validate_frame(dims, &vars, &draws).is_err(),
                "missing whole-view shadow cannot become Ready"
            );
            draws.push(draws[0].clone());
            vars.world_to_shadow.pop();
            assert!(
                validate_frame(dims, &vars, &draws).is_err(),
                "missing auxiliary projection cannot become Ready"
            );
        }
    }

    #[test]
    fn capture_resource_number_does_not_classify_an_engine_program() {
        let dims = NativeParallelShadowTargetDimensions {
            width: 32,
            height: 32,
            layers: 2,
            depth_32_bits: true,
        };
        let layers = [NativeShadowLayer {
            ops: NativeShadowLayerOps {
                depth: NativeShadowLoad::Load,
                stencil: NativeStencilLoad::Load,
            },
            instances: vec![NativeShadowInstance {
                mesh: Handle::default(),
                per_instance_cb: [0; NATIVE_SHADOW_INSTANCE_BYTES].into(),
                program: NativeShadowProgram::Unsupported(17_296),
                alpha_material: None,
                front_face: FrontFace::Ccw,
                cull_mode: Some(Face::Back),
                draw_state: [0; 9],
                scissor: NativeShadowScissor {
                    x: 0,
                    y: 0,
                    width: 32,
                    height: 32,
                },
            }],
        }];
        // A valid frame also supplies the auxiliary layer, even if its owner
        // explicitly retains the cached image and has no new draws this frame.
        let mut layers = vec![
            layers[0].clone(),
            NativeShadowLayer {
                ops: NativeShadowLayerOps {
                    depth: NativeShadowLoad::Load,
                    stencil: NativeStencilLoad::Load,
                },
                instances: vec![],
            },
        ];
        assert!(validate_frame(dims, &receiver(), &layers)
            .unwrap_err()
            .contains("shader class"));
        layers[0].instances[0].program = NativeShadowProgram::RigidOpaque;
        validate_frame(dims, &receiver(), &layers).unwrap();
        layers[0].instances[0].program = NativeShadowProgram::RigidAlphaTested;
        assert!(
            validate_frame(dims, &receiver(), &layers).is_err(),
            "no missing-diffuse fallback"
        );
        layers[0].instances[0].alpha_material = Some(NativeShadowAlphaMaterial {
            diffuse: Handle::default(),
            uv_set: 1,
            uv_scale_bias: [1.0, 1.0, 0.0, 0.0],
        });
        validate_frame(dims, &receiver(), &layers).unwrap();
        layers[0].instances[0]
            .alpha_material
            .as_mut()
            .unwrap()
            .uv_set = 2;
        assert!(
            validate_frame(dims, &receiver(), &layers).is_err(),
            "unmapped UV set cannot become UV0"
        );
        let alpha = layers[0].instances[0].alpha_material.as_mut().unwrap();
        alpha.uv_set = 0;
        alpha.uv_scale_bias[0] = f32::NAN;
        assert!(validate_frame(dims, &receiver(), &layers).is_err());
    }

    #[test]
    fn clear_is_explicit_and_stencil_only_exists_in_32_bit_target() {
        let layer = NativeShadowLayerOps {
            depth: NativeShadowLoad::Clear(1.0),
            stencil: NativeStencilLoad::Clear(0),
        };
        for has_stencil in [false, true] {
            let reused = layer_operations(layer, false, has_stencil);
            assert!(matches!(reused.0.load, LoadOp::Load));
            assert_eq!(reused.1.is_some(), has_stencil);
            let first = layer_operations(layer, true, has_stencil);
            assert!(matches!(first.0.load, LoadOp::Clear(1.0)));
            if has_stencil {
                assert!(matches!(first.1.unwrap().load, LoadOp::Clear(0)));
            } else {
                assert!(first.1.is_none());
            }
        }
    }

    #[test]
    fn disabled_and_unavailable_are_explicit_neutral_states() {
        assert!(matches!(
            NativeShadowSceneFrame::default().state,
            NativeShadowSceneState::Disabled
        ));
        let unavailable = NativeShadowSceneState::Unavailable("owner not ready".into());
        assert!(matches!(
            unavailable,
            NativeShadowSceneState::Unavailable(_)
        ));
    }
}
