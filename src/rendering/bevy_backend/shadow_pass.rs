//! Bevy render-world bridge for original opaque, cutout and vegetation shadows.
//!
//! The game thread publishes the already-resolved native shadow state and caster set;
//! this module uploads each program's actual instance block and submits draws against
//! Bevy's existing mesh allocations. It deliberately does not query/cull scene geometry,
//! infer cascade/cache policy, or enable Bevy light shadows.

use crate::shadow_pipeline::{
    native_shadow_alpha_sampler, native_shadow_vista_sampler, NativeOpaqueShadowDraw,
    NativeOpaqueShadowPipeline, NativeShadowDepthArray, NativeShadowDepthProgram,
    NativeShadowScissor,
};
use bevy::platform::collections::HashMap;
mod pipeline_store;
use bevy::render::diagnostic::RecordDiagnostics;
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
            BindGroup, Buffer, BufferDescriptor, BufferInitDescriptor, BufferUsages, Face,
            FrontFace, LoadOp, Operations, RawVertexBufferLayout, RenderPassDepthStencilAttachment,
            Sampler, StoreOp, TextureView, VertexStepMode,
        },
        renderer::{RenderAdapter, RenderContext, RenderDevice, RenderQueue},
        storage::{GpuShaderBuffer, ShaderBuffer},
        texture::GpuImage,
        Render, RenderApp, RenderSystems,
    },
};
use pipeline_store::{PipelineId, ShadowPipelineStore};
use std::sync::{Arc, Mutex};

#[path = "shadow_pass/local.rs"]
mod local;
#[path = "shadow_pass/prepared_cache.rs"]
mod prepared_cache;
pub use local::{
    NativeLocalShadowCompletion, NativeLocalShadowReceiver, NativeLocalShadowReceiverLight,
    NativeLocalShadowRegion, NativeLocalShadowSceneFrame, NativeLocalShadowSubmission,
    NativeShadowCaptureReadiness,
};

/// A resolved original shader *class*, not a transient RenderDoc resource ID.
/// The opaque class is evidenced by VS hash be596c78-7b23f0b2-921af52f-29b1e602.
/// The CPU owner must resolve the original program bank before selecting it.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum NativeShadowProgram {
    Skinned {
        influences: u8,
        alpha: bool,
        dither: bool,
    },
    RigidOpaque,
    /// vertex.decl useMultiInstancing: original remap and 48-byte matrices.
    WorldOpaque,
    WorldAlphaTested,
    WorldVistaAlpha,
    WorldVegetation,
    WorldVegetationFixed,
    RigidAlphaTested,
    RigidVistaAlpha,
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

// GPU transport equality must retain signed zero and NaN payload words.
impl PartialEq for NativeShadowAlphaMaterial {
    fn eq(&self, other: &Self) -> bool {
        self.diffuse == other.diffuse
            && self.uv_set == other.uv_set
            && self.uv_scale_bias.map(f32::to_bits) == other.uv_scale_bias.map(f32::to_bits)
    }
}
impl Eq for NativeShadowAlphaMaterial {}

pub const NATIVE_SHADOW_INSTANCE_BYTES: usize = 80;
pub const NATIVE_WORLD_SHADOW_INSTANCE_BYTES: usize = 96;
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
/// Original local positions before the forward builder's world/model bake.
/// Shared by native model/world programs: original MVP rows require original
/// local game coordinates, including when forward positions were axis-swapped
/// or baked to preserve affine shear.
pub const NATIVE_VEGETATION_POSITION: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeVegetationPosition",
    0x4448_3203,
    bevy::render::render_resource::VertexFormat::Float32x3,
);

pub const NATIVE_SKIN_INDICES_0: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeSkinIndices0",
    0x4448_3210,
    bevy::render::render_resource::VertexFormat::Uint8x4,
);
pub const NATIVE_SKIN_WEIGHTS_0: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeSkinWeights0",
    0x4448_3211,
    bevy::render::render_resource::VertexFormat::Unorm8x4,
);
pub const NATIVE_SKIN_INDICES_1: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeSkinIndices1",
    0x4448_3212,
    bevy::render::render_resource::VertexFormat::Uint8x4,
);
pub const NATIVE_SKIN_WEIGHTS_1: MeshVertexAttribute = MeshVertexAttribute::new(
    "NativeSkinWeights1",
    0x4448_3213,
    bevy::render::render_resource::VertexFormat::Unorm8x4,
);

/// Bridge-owned packing; native VS families use different CB offsets for
/// these same named inputs. This is deliberately not a native CB memcpy.
pub fn native_skin_shadow_instance_cb(
    rows: &[[f32; 4]; 4],
    offset: u32,
    depth: f32,
    uv: [f32; 4],
) -> [u8; 112] {
    let mut bytes = [0; 112];
    bytes[..4].copy_from_slice(&offset.to_le_bytes());
    for (i, value) in [
        rows[2],
        rows[1],
        rows[0],
        rows[3],
        [depth, 0.0, 0.0, 0.0],
        uv,
    ]
    .iter()
    .flatten()
    .enumerate()
    {
        bytes[16 + i * 4..20 + i * 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// The payload ABI is explicit: an 80-byte rigid record cannot become a wind
/// record by padding it or supplying a zero/default clock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeShadowInstanceConstants {
    Skinned {
        constants: [u8; 112],
        palette: Handle<ShaderBuffer>,
    },
    Rigid([u8; NATIVE_SHADOW_INSTANCE_BYTES]),
    /// The owner validates all low-24-bit IDs against its original matrix
    /// array before publishing. The 96-byte bridge record stores VP W/Z/Y/X,
    /// the actual remap offset and the alpha family's UV block. It is not a
    /// rigid model/depth-hack block or a memcpy of the native split CB slots.
    World {
        constants: [u8; NATIVE_WORLD_SHADOW_INSTANCE_BYTES],
        visible_indices: Handle<ShaderBuffer>,
        matrices: Handle<ShaderBuffer>,
        instance_count: u32,
    },
    WorldVegetation {
        constants: [u8; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES],
        visible_indices: Handle<ShaderBuffer>,
        matrices: Handle<ShaderBuffer>,
        /// Present only for the original 63-slot wind-buffer variant.
        /// The 51-slot fixed-wind variant has no such resource.
        winds: Option<Handle<ShaderBuffer>>,
        instance_count: u32,
    },
    Vegetation([u8; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES]),
}

impl NativeShadowInstanceConstants {
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Skinned { constants, .. } => constants,
            Self::Rigid(bytes) => bytes,
            Self::World { constants, .. } => constants,
            Self::WorldVegetation { constants, .. } => constants,
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

/// Installed world-wind ABI: VP W/Z/Y/X then original 144-byte CB2.
/// The remap and wind offsets are independent native packet offsets.
pub fn native_world_vegetation_shadow_view_cb(
    rows: &[[f32; 4]; 4],
    uv: [f32; 4],
    wind: &NativeVegetationShadowParameters,
    time: [f32; 2],
    instance_offset: u32,
    wind_offset: u32,
) -> [u8; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES] {
    let mut bytes = [0; NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES];
    let vectors = [
        rows[3],
        rows[2],
        rows[1],
        rows[0],
        uv,
        wind.main_variation,
        wind.main_frequency,
        wind.main_amplitude,
        wind.minimum_wind,
        wind.leaf_frequency,
        wind.leaf_amplitude,
        wind.branch_amplitude,
    ];
    for (i, value) in vectors.iter().flatten().chain(time.iter()).enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[200..204].copy_from_slice(&instance_offset.to_le_bytes());
    bytes[204..208].copy_from_slice(&wind_offset.to_le_bytes());
    bytes
}

/// Original world position variables, transported in a bridge-owned 96-byte
/// record (unused alpha UV space remains zero). Unlike rigid models,
/// useMultiInstancing has no modelDepthHack.
/// Native packet.start supplies offset; packet.end-start supplies draw count.
pub fn native_world_shadow_view_cb(
    view_projection_rows: &[[f32; 4]; 4],
    instance_buffer_offset: u32,
) -> [u8; NATIVE_WORLD_SHADOW_INSTANCE_BYTES] {
    let mut bytes = [0; NATIVE_WORLD_SHADOW_INSTANCE_BYTES];
    for (word, value) in view_projection_rows.iter().rev().flatten().enumerate() {
        bytes[word * 4..word * 4 + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[64..68].copy_from_slice(&instance_buffer_offset.to_le_bytes());
    bytes
}

/// Same world record, including the actual installed alpha program's UV block.
/// These are bridge offsets, not a memcpy of the native two CB slots.
pub fn native_world_alpha_shadow_view_cb(
    view_projection_rows: &[[f32; 4]; 4],
    instance_buffer_offset: u32,
    uv_scale_bias: [f32; 4],
) -> [u8; NATIVE_WORLD_SHADOW_INSTANCE_BYTES] {
    let mut bytes = native_world_shadow_view_cb(view_projection_rows, instance_buffer_offset);
    for (word, value) in uv_scale_bias.iter().enumerate() {
        bytes[80 + word * 4..84 + word * 4].copy_from_slice(&value.to_le_bytes());
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
    /// N cascade matrices plus the separate whole-view matrix.
    pub world_to_shadow: Vec<[[f32; 4]; 4]>,
    pub view_row2: [f32; 4],
    /// Updated effective far bank from FUN_140330630; inactive slots are FLT_MAX.
    pub cascade_fars: [f32; 4],
    pub shadow_size: [f32; 2],
    pub light_quality: i32,
    pub filtering_quality: i32,
    /// Chosen sun in the host renderer's directional-light array.
    pub directional_light_index: u32,
    /// Actual original shadowCommon options/parameters, not shader defaults.
    pub offset: crate::shadow_sampling::NativeShadowOffsetParameters,
}

impl NativeParallelReceiverVars {
    /// FUN_140330c10 loads the count-indexed distance from
    /// `plVar20[1]+0xafc+DAT_14397a2f8*4` and passes it through
    /// FUN_140a70710 to FUN_140a707e0 as param_8. The captured four-cascade
    /// frame selects `+0xb0c` (the fourth far); in this Rust candidate that is
    /// `cascade_fars[N-1]`. `world_to_shadow` stores N cascades plus a whole-
    /// view record. `DAT_14397a2f8` is the +0x28 storage used by the registered
    /// `r_shadowParallelSliceCnt` object (`DAT_14397a2d0`); `FUN_14050ff40`
    /// updates it from the selected preset's `m_shadowParallelSliceCnt`.
    /// FUN_140351780 binds &DAT_143308e20 to the prepare task at 140353668
    /// and puts the SAME address in gather-results slot1 at 1403537c2.
    /// +0xb00 is DAT_143309920: the bank is written by preparation and copied
    /// TO the shader parameters by FUN_140310a80. Supply the post-update bank.
    pub fn receiver_maximum_distance(&self) -> Result<f32, String> {
        let cascade_count = self
            .world_to_shadow
            .len()
            .checked_sub(1)
            .ok_or("native receiver has no whole-view record")?;
        if !(1..=4).contains(&cascade_count) {
            return Err("native receiver cascade count must be 1..=4".into());
        }
        let maximum_distance = self.cascade_fars[cascade_count - 1];
        if !maximum_distance.is_finite() {
            return Err("native receiver maximum distance is nonfinite".into());
        }
        Ok(maximum_distance)
    }
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

/// Attachment load/clear policy: `Clear` executes even for an empty dirty layer.
/// This is NOT the original separate static-cache copy. FUN_14032ee50
/// (analysis/decompiled/chunk_000120.c:3203..3385) restores cache[layer-1]
/// and RETURNS from its static pass when reuse_static_cache is true; the
/// preserve-depth dynamic pass is a separate submission. A static refresh
/// stores its completed target into cache[layer-1] before that dynamic pass.
/// NativeShadowCacheOp represents those GPU store/restore operations separately
/// from the attachment loads below. Load alone is not a cache restore.
/// FUN_1402bb270 forwards the two +0x130 resources to context vtable +0x178:
/// ID3D11DeviceContext::CopyResource, slot47. Adjacent +0x170 has the exact
/// slot46 CopySubresourceRegion arguments (FUN_1402bb2a0). The extra target
/// kind in R9 is unused by the three-argument COM CopyResource method.
/// Menu resource capture confirms cache and atlas share format_word9e059856,
/// width/height2048 and mip1; caches have one layer, atlas five. The native
/// static-pass temporary is copied to/from cache before the separate atlas
/// depth-copy pass. CachedTarget keeps separate submission surfaces and caches;
/// its cache transfers preserve the original texels before atlas publication.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeShadowLayerOps {
    pub depth: NativeShadowLoad,
    pub stencil: NativeStencilLoad,
}

/// A single instance of a resolved original model-shadow program.
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// Dynamic reference supplied by the native pass owner, not a shader flag.
    pub stencil_reference: u32,
    pub scissor: NativeShadowScissor,
}

/// Operations of the static submission of FUN_14032ee50. The dynamic
/// preserve-depth submission follows it; it is never saved in the static cache.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeShadowCacheOp {
    #[default]
    None,
    /// Restore cache[layer-1], then render only this frame's dynamic instances.
    Restore,
    /// First render this many static instances, save the temporary, then render
    /// the remaining dynamic instances with Load. Zero still clears/stores.
    Store { static_instances: usize },
    /// Source-owned static snapshot from its actual last refresh frame. If
    /// extraction/asset preparation delayed that refresh, materialize THESE
    /// retained inputs before restoring. Never manufacture a cache from the
    /// current frame's different masks, matrices or vegetation clock.
    Retained {
        generation: u64,
        static_instances: usize,
    },
}

#[derive(Clone, Debug)]
pub struct NativeShadowLayer {
    pub ops: NativeShadowLayerOps,
    pub cache: NativeShadowCacheOp,
    /// Native submission order, static before dynamic for a cache refresh.
    pub instances: Vec<NativeShadowInstance>,
}

#[derive(Clone, Debug)]
pub enum NativeShadowSceneState {
    /// Bindings must resolve to neutral visibility (factor 1); no shadow map is fabricated.
    Disabled,
    /// CPU owner has not yet published a usable target/cascade state.
    Unavailable(String),
    /// Submit the real auxiliary whole-view pass independently, as
    /// FUN_140334170/14032de00 do. This is NOT a usable receiver frame:
    /// cascade layers are not fabricated and sampling remains disabled.
    WholeViewDepth {
        target_image: Handle<Image>,
        dimensions: NativeParallelShadowTargetDimensions,
        layer: NativeShadowLayer,
    },
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
/// Rigid opaque/alpha-tested, world opaque/alpha/Vista and both world wind
/// families are supported. Vegetation requires original local positions,
/// dedicated raw normal/color attributes and its full program-specific block;
/// world wind also requires the actual remap/matrix/wind resources. Four/eight
/// weight programs require original joint/weight streams and the owner palette.
/// Cloth and paraboloid programs remain unavailable. ScreenSpace atlas
/// publication preserves the screen route; it does not enable the direct hook
/// or stand in for the separate screen-shadow producer.
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

/// CPU acknowledgment of the actual auxiliary GPU submission. It allows the
/// live publisher to retain that completed atlas layer without rebuilding and
/// uploading its ten thousand immutable draw records on every camera frame.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct NativeShadowSceneCompletion(
    Arc<Mutex<Option<(AssetId<Image>, NativeParallelShadowTargetDimensions)>>>,
);

impl NativeShadowSceneCompletion {
    pub fn whole_view_completed(
        &self,
        target: &Handle<Image>,
        dimensions: NativeParallelShadowTargetDimensions,
    ) -> bool {
        self.0
            .lock()
            .is_ok_and(|done| *done == Some((target.id(), dimensions)))
    }
}

#[derive(Resource, Default)]
pub(crate) struct NativeShadowSceneGpu {
    last_revision: Option<u64>,
    targets: HashMap<AssetId<Image>, CachedTarget>,
    pipelines: ShadowPipelineStore,
    resource_bindings: HashMap<ShadowResourceKey, BindGroup>,
    mesh_allocations: mesh_allocations::MeshAllocationCache,
    prepared_records: Option<prepared_cache::PreparedRecordCache>,
    prepared_hits: u64,
    prepared_misses: u64,
    uniform_ring: Option<Buffer>,
    uniform_capacity: usize,
    uniform_stride: u64,
    uniform_bytes: Vec<u8>,
    uniform_records: Vec<ShadowUniformRecord>,
    alpha_sampler: Option<Sampler>,
    vista_sampler: Option<Sampler>,
    sampling_buffer: Option<Buffer>,
    sampling_bytes: Vec<u8>,
}

impl NativeShadowSceneGpu {
    pub(crate) fn completed_revision(&self, revision: u64) -> bool {
        self.last_revision == Some(revision)
    }
}

struct CachedTarget {
    dimensions: NativeParallelShadowTargetDimensions,
    array: NativeShadowDepthArray,
    /// Separate static/dynamic submission surfaces, then copied to the atlas.
    temporary: Vec<NativeShadowDepthArray>,
    /// Original cache[layer-1], excluding cascade0 and the auxiliary whole view.
    static_cache: Vec<NativeShadowDepthArray>,
    cache_valid: Vec<bool>,
    cache_generation: Vec<Option<u64>>,
    layer_valid: Vec<bool>,
}

impl CachedTarget {
    fn new(
        device: &RenderDevice,
        dimensions: NativeParallelShadowTargetDimensions,
        array: NativeShadowDepthArray,
    ) -> Result<Self, String> {
        let size = bevy::render::render_resource::Extent3d {
            width: dimensions.width,
            height: dimensions.height,
            depth_or_array_layers: 1,
        };
        let allocate = |label| {
            NativeShadowDepthArray::new_with_format(device, size, array.texture.format(), label)
        };
        let temporary = (0..dimensions.layers)
            .map(|_| allocate("native directional slice temporary"))
            .collect::<Result<Vec<_>, _>>()?;
        let cache_count = dimensions.layers.saturating_sub(2) as usize;
        let static_cache = (0..cache_count)
            .map(|_| allocate("native directional static cache"))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            dimensions,
            array,
            temporary,
            static_cache,
            cache_valid: vec![false; cache_count],
            cache_generation: vec![None; cache_count],
            layer_valid: vec![false; dimensions.layers as usize],
        })
    }

    fn validate_cache_sources(&self, layers: &[NativeShadowLayer]) -> Result<(), String> {
        for (index, layer) in layers.iter().enumerate() {
            if layer.cache == NativeShadowCacheOp::Restore
                && !self
                    .cache_valid
                    .get(index.wrapping_sub(1))
                    .copied()
                    .unwrap_or(false)
            {
                return Err(format!(
                    "native directional layer {index} has no completed static cache"
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct PipelineKey {
    layout: u64,
    depth_32_bits: bool,
    front_face: FrontFace,
    cull_mode: Option<Face>,
    program: NativeShadowProgram,
    alpha_uv: Option<u32>,
    draw_state: [u32; 9],
    stencil_reference: u32,
}

struct CachedPipeline {
    layout: MeshVertexBufferLayoutRef,
    pipeline: NativeOpaqueShadowPipeline,
    // One descriptor selects this program's records by dynamic ring offset.
    binding: Option<BindGroup>,
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
        local::install(app);
        app.init_resource::<NativeShadowSceneFrame>()
            .init_resource::<NativeShadowSceneCompletion>()
            .add_plugins(ExtractResourcePlugin::<NativeShadowSceneFrame>::default())
            .add_plugins(ExtractResourcePlugin::<NativeShadowSceneCompletion>::default())
            .add_plugins(ExtractResourcePlugin::<
                crate::shadow_sampling::NativeParallelSamplingBuffer,
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
    let layers = match &frame.state {
        NativeShadowSceneState::Ready { layers, .. } => layers.as_slice(),
        NativeShadowSceneState::WholeViewDepth { layer, .. } => std::slice::from_ref(layer),
        _ => return,
    };
    upload_native_shadow_uniform_ring(layers, &device, &queue, &mut gpu);
}

fn upload_native_shadow_uniform_ring(
    layers: &[NativeShadowLayer],
    device: &RenderDevice,
    queue: &RenderQueue,
    gpu: &mut NativeShadowSceneGpu,
) {
    let draw_count: usize = layers.iter().map(|layer| layer.instances.len()).sum();
    let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
    let stride = instance_uniform_stride(layers, alignment);
    // World shader indexing is CB.offset + instance_index. Moving that exact
    // integer offset into first_instance leaves every shader read unchanged
    // and lets packets with the same view/UV data share a constant record.
    let compact_world = std::env::var("DH2_COMPACT_SHADOW_UNIFORMS").as_deref() != Ok("0");
    gpu.uniform_bytes.clear();
    gpu.uniform_records.clear();
    gpu.uniform_records.reserve(draw_count);
    let mut world_slots = HashMap::new();
    for instance in layers.iter().flat_map(|layer| &layer.instances) {
        let record = pack_shadow_uniform(
            instance,
            stride as usize,
            compact_world,
            &mut gpu.uniform_bytes,
            &mut world_slots,
        );
        gpu.uniform_records.push(record);
    }
    let needed = (gpu.uniform_bytes.len() / stride as usize).max(1);
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
            cached.binding = None;
        }
    }
    let Some(buffer) = gpu.uniform_ring.as_ref().cloned() else {
        return;
    };
    if !gpu.uniform_bytes.is_empty() {
        queue.write_buffer(&buffer, 0, &gpu.uniform_bytes);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ShadowUniformRecord {
    slot: usize,
    instances: std::ops::Range<u32>,
}

fn pack_shadow_uniform(
    instance: &NativeShadowInstance,
    stride: usize,
    compact_world: bool,
    bytes: &mut Vec<u8>,
    world_slots: &mut HashMap<[u8; NATIVE_WORLD_SHADOW_INSTANCE_BYTES], usize>,
) -> ShadowUniformRecord {
    let append = |bytes: &mut Vec<u8>, constants: &[u8]| {
        let slot = bytes.len() / stride;
        bytes.resize(bytes.len() + stride, 0);
        bytes[slot * stride..slot * stride + constants.len()].copy_from_slice(constants);
        slot
    };
    let normalize = compact_world
        && matches!(
            instance.program,
            NativeShadowProgram::WorldOpaque
                | NativeShadowProgram::WorldAlphaTested
                | NativeShadowProgram::WorldVistaAlpha
        );
    if let (
        true,
        NativeShadowInstanceConstants::World {
            constants,
            instance_count,
            ..
        },
    ) = (normalize, &instance.per_instance_cb)
    {
        let start = u32::from_le_bytes(constants[64..68].try_into().unwrap());
        // Preserve the original shader's wrapping addition if the range cannot
        // be expressed by a draw's non-wrapping first/end instance arguments.
        if let Some(end) = start.checked_add(*instance_count) {
            let mut canonical = *constants;
            canonical[64..68].fill(0);
            let slot = *world_slots
                .entry(canonical)
                .or_insert_with(|| append(bytes, &canonical));
            return ShadowUniformRecord {
                slot,
                instances: start..end,
            };
        }
    }
    let count = match &instance.per_instance_cb {
        NativeShadowInstanceConstants::World { instance_count, .. }
        | NativeShadowInstanceConstants::WorldVegetation { instance_count, .. } => *instance_count,
        _ => 1,
    };
    ShadowUniformRecord {
        slot: append(bytes, instance.per_instance_cb.as_bytes()),
        instances: 0..count,
    }
}

fn render_native_shadow_scene(
    frame: Res<NativeShadowSceneFrame>,
    completion: Res<NativeShadowSceneCompletion>,
    mut gpu: ResMut<NativeShadowSceneGpu>,
    allocator: Res<MeshAllocator>,
    meshes: Res<RenderAssets<RenderMesh>>,
    images: Res<RenderAssets<GpuImage>>,
    sampling_handle: Option<Res<crate::shadow_sampling::NativeParallelSamplingBuffer>>,
    sampling_buffers: Res<RenderAssets<GpuShaderBuffer>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    adapter: Res<RenderAdapter>,
    mut context: RenderContext,
    profile: Option<Res<crate::frame_profile::CpuFrameProfile>>,
) {
    let _span = profile.as_deref().map(|p| p.measure(27));
    let sampling = sampling_handle
        .as_ref()
        .and_then(|handle| sampling_buffers.get(handle.buffer.id()));
    let (target_image, dimensions, receiver, layers, first_layer) = match &frame.state {
        NativeShadowSceneState::Ready {
            target_image,
            dimensions,
            receiver,
            layers,
        } => (
            target_image,
            dimensions,
            Some(receiver),
            layers.as_slice(),
            0,
        ),
        NativeShadowSceneState::WholeViewDepth {
            target_image,
            dimensions,
            layer,
        } => (
            target_image,
            dimensions,
            None,
            std::slice::from_ref(layer),
            dimensions.layers.saturating_sub(1),
        ),
        _ => {
            publish_sampling(
                &mut gpu,
                sampling,
                &queue,
                crate::shadow_sampling::NativeParallelSamplingUniform::default(),
            );
            gpu.last_revision = None;
            return;
        }
    };
    if std::env::var_os("DH2_TRACE_SHADOW_SUBMISSION").is_some()
        && gpu.last_revision != Some(frame.revision)
    {
        eprintln!(
            "Native shadow GPU revision={} layers={} draws={} pipelines={} target_ready={}",
            frame.revision,
            layers.len(),
            layers
                .iter()
                .map(|layer| layer.instances.len())
                .sum::<usize>(),
            gpu.pipelines.len(),
            gpu.targets.contains_key(&target_image.id())
        );
        if let Some(target) = gpu.targets.get(&target_image.id()) {
            eprintln!("Native shadow valid layers: {:?}", target.layer_valid);
        }
    }
    if gpu.last_revision == Some(frame.revision) {
        return;
    }
    // No new receiver rows may sample an incomplete or failed atlas submission.
    publish_sampling(
        &mut gpu,
        sampling,
        &queue,
        crate::shadow_sampling::NativeParallelSamplingUniform::default(),
    );
    if receiver.is_some() && sampling.is_none() {
        return; // Complete receiver publication retries after PrepareAssets.
    }
    let validation = match receiver {
        Some(receiver) => validate_frame(*dimensions, receiver, layers),
        None => validate_whole_view_depth(*dimensions, &layers[0]),
    };
    if let Err(error) = validation {
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
                    match CachedTarget::new(&device, *dimensions, array) {
                        Ok(target) => target,
                        Err(error) => {
                            bevy::log::error!(
                                "native directional cache allocation failed: {error}"
                            );
                            return;
                        }
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
        bevy::log::error!(
            "native shadow target handle was reused with different dimensions; refusing per-frame recreation"
        );
        return;
    }

    if let Err(error) = target.validate_cache_sources(layers) {
        bevy::log::error!("native shadow cache unavailable: {error}");
        return;
    }
    if !target
        .array
        .texture
        .usage()
        .contains(bevy::render::render_resource::TextureUsages::COPY_DST)
    {
        bevy::log::error!(
            "native shadow atlas owner did not allocate final-copy destination usage"
        );
        return;
    }

    let Some(uniform_buffer) = gpu.uniform_ring.as_ref().cloned() else {
        return;
    };
    let uniform_stride = gpu.uniform_stride;
    let records_span = profile.as_deref().map(|p| p.measure(29));
    let Some(records) = prepare_native_shadow_records(
        layers,
        dimensions.depth_32_bits,
        true,
        uniform_buffer,
        uniform_stride,
        &mut gpu,
        &allocator,
        &meshes,
        &images,
        &sampling_buffers,
        &device,
        &adapter,
    ) else {
        return;
    };
    drop(records_span);
    let _encoding_span = profile.as_deref().map(|p| p.measure(30));

    let NativeShadowSceneGpu {
        targets, pipelines, ..
    } = &mut *gpu;
    let target = targets
        .get_mut(&target_id)
        .expect("target prepared before draws");
    let diagnostics = context.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let span = diagnostics.time_span(context.command_encoder(), "native_directional_shadows");
    let mut complete = true;
    if layers
        .iter()
        .all(|layer| layer.cache == NativeShadowCacheOp::None)
        && std::env::var("DH2_PARALLEL_SHADOW_ENCODING").as_deref() != Ok("0")
    {
        // Distinct physical slices share read-only meshes and constants. Record
        // their commands independently, then submit in the publisher's order.
        let shared_target = &*target;
        let pipelines = &*pipelines;
        let records = &records;
        let device = &*device;
        let mut encoded = bevy::tasks::ComputeTaskPool::get().scope(|scope| {
            for (layer_index, layer) in layers.iter().enumerate() {
                scope.spawn(async move {
                    let physical = first_layer as usize + layer_index;
                    let draws = directional_layer_draws(records, pipelines, layer_index);
                    let skipped = draws.is_empty()
                        && matches!(layer.ops.depth, NativeShadowLoad::Load)
                        && matches!(layer.ops.stencil, NativeStencilLoad::Load);
                    if skipped {
                        let valid = shared_target.layer_valid[physical]
                            || receiver.is_some_and(|receiver| {
                                !crate::render_shadow::parallel_shadow_layer_can_be_selected(
                                    receiver.cascade_fars,
                                    physical,
                                )
                            });
                        return (physical, valid, Ok::<_, String>(None));
                    }
                    let result = (|| {
                        let mut encoder = device.create_command_encoder(&Default::default());
                        let temporary = &shared_target.temporary[physical];
                        draw_native_shadow_groups(
                            &mut encoder,
                            temporary,
                            pipelines,
                            draws,
                            layer.ops,
                        )?;
                        shared_target.array.copy_layer_from(
                            &mut encoder,
                            physical as u32,
                            temporary,
                            0,
                        )?;
                        Ok(Some(encoder.finish()))
                    })();
                    (physical, true, result)
                });
            }
        });
        encoded.sort_by_key(|entry| entry.0);
        for (physical, valid, result) in encoded {
            match result {
                Ok(Some(commands)) => {
                    context.add_command_buffer(commands);
                    target.layer_valid[physical] = true;
                    complete &= valid;
                }
                Ok(None) => complete &= valid,
                Err(error) => {
                    bevy::log::error!("parallel directional submission failed: {error}");
                    span.end(context.command_encoder());
                    return;
                }
            }
        }
    } else {
        for (layer_index, layer) in layers.iter().enumerate() {
            let physical_layer = first_layer as usize + layer_index;
            let temporary = &target.temporary[physical_layer];
            let mut remaining = directional_layer_draws(&records, pipelines, layer_index);
            // A skipped native submission preserves the atlas, not stale scratch.
            if remaining.is_empty()
                && layer.cache == NativeShadowCacheOp::None
                && matches!(layer.ops.depth, NativeShadowLoad::Load)
                && matches!(layer.ops.stencil, NativeStencilLoad::Load)
            {
                // Collapsed native intervals deliberately receive no draw/clear.
                // The shader's sum(step(fars,Z)) cannot select those layers, so
                // their undefined contents do not prevent a real receiver frame.
                complete &= target.layer_valid[physical_layer]
                    || receiver.is_some_and(|receiver| {
                        !crate::render_shadow::parallel_shadow_layer_can_be_selected(
                            receiver.cascade_fars,
                            physical_layer,
                        )
                    });
                continue;
            }
            let load = NativeShadowLayerOps {
                depth: NativeShadowLoad::Load,
                stencil: NativeStencilLoad::Load,
            };
            let result = (|| -> Result<(), String> {
                match layer.cache {
                    NativeShadowCacheOp::None => {
                        draw_native_shadow_groups(
                            context.command_encoder(),
                            temporary,
                            pipelines,
                            remaining,
                            layer.ops,
                        )?;
                    }
                    NativeShadowCacheOp::Restore => {
                        temporary.copy_resource_from(
                            context.command_encoder(),
                            &target.static_cache[layer_index - 1],
                        )?;
                        draw_native_shadow_groups(
                            context.command_encoder(),
                            temporary,
                            pipelines,
                            remaining,
                            load,
                        )?;
                    }
                    NativeShadowCacheOp::Store { static_instances } => {
                        let dynamic = remaining.split_off(static_instances);
                        draw_native_shadow_groups(
                            context.command_encoder(),
                            temporary,
                            pipelines,
                            remaining,
                            layer.ops,
                        )?;
                        target.static_cache[layer_index - 1]
                            .copy_resource_from(context.command_encoder(), temporary)?;
                        target.cache_valid[layer_index - 1] = true;
                        target.cache_generation[layer_index - 1] = None;
                        draw_native_shadow_groups(
                            context.command_encoder(),
                            temporary,
                            pipelines,
                            dynamic,
                            load,
                        )?;
                    }
                    NativeShadowCacheOp::Retained {
                        generation,
                        static_instances,
                    } => {
                        let dynamic = remaining.split_off(static_instances);
                        let cache = layer_index - 1;
                        if target.cache_valid[cache]
                            && target.cache_generation[cache] == Some(generation)
                        {
                            temporary.copy_resource_from(
                                context.command_encoder(),
                                &target.static_cache[cache],
                            )?;
                        } else {
                            draw_native_shadow_groups(
                                context.command_encoder(),
                                temporary,
                                pipelines,
                                remaining,
                                layer.ops,
                            )?;
                            target.static_cache[cache]
                                .copy_resource_from(context.command_encoder(), temporary)?;
                            target.cache_valid[cache] = true;
                            target.cache_generation[cache] = Some(generation);
                        }
                        draw_native_shadow_groups(
                            context.command_encoder(),
                            temporary,
                            pipelines,
                            dynamic,
                            load,
                        )?;
                    }
                }
                // 32db10 full-rectangle copy fast path, AFTER the dynamic pass.
                target.array.copy_layer_from(
                    context.command_encoder(),
                    physical_layer as u32,
                    temporary,
                    0,
                )
            })();
            if let Err(error) = result {
                bevy::log::error!("native directional cache/draw submission failed: {error}");
                span.end(context.command_encoder());
                return;
            }
            target.layer_valid[physical_layer] = true;
        }
    }
    span.end(context.command_encoder());
    if matches!(frame.state, NativeShadowSceneState::WholeViewDepth { .. }) {
        if let Ok(mut done) = completion.0.lock() {
            *done = Some((target_id, *dimensions));
        }
    }
    if !complete {
        return;
    } // A skipped first submission cannot expose undefined atlas texels.
      // Only the fully submitted owner generation may become a forward receiver.
      // Texture and bind-group handles stay stable while these 432 bytes change.
    if let Some(receiver) = receiver {
        let sampling_value =
            crate::shadow_sampling::NativeParallelSamplingUniform::from_owner_receiver(receiver)
                .expect("receiver route validated before submitting any layer");
        publish_sampling(&mut gpu, sampling, &queue, sampling_value);
    } // Whole-view depth submission never enables an incomplete receiver.
      // Do not cache missing assets or failed submissions; retry that same owner revision.
    gpu.last_revision = Some(frame.revision);
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
enum ShadowResourceKey {
    World(
        PipelineKey,
        bevy::render::render_resource::BufferId,
        bevy::render::render_resource::BufferId,
        Option<bevy::render::render_resource::BufferId>,
        Option<bevy::render::render_resource::TextureViewId>,
    ),
    Skin(
        PipelineKey,
        bevy::render::render_resource::BufferId,
        Option<bevy::render::render_resource::TextureViewId>,
    ),
}

struct ShadowMeshAllocation {
    layout: MeshVertexBufferLayoutRef,
    layout_key: u64,
    vertex: Buffer,
    base_vertex: i32,
    index: Buffer,
    indices: std::ops::Range<u32>,
    index_format: bevy::render::render_resource::IndexFormat,
}

struct PreparedShadowRecord {
    layer: usize,
    scissor: NativeShadowScissor,
    key: PipelineKey,
    pipeline: PipelineId,
    uniform_offset: u32,
    instances: std::ops::Range<u32>,
    vertex: Buffer,
    base_vertex: i32,
    index: Buffer,
    indices: std::ops::Range<u32>,
    index_format: bevy::render::render_resource::IndexFormat,
    resources: Option<BindGroup>,
}

impl PreparedShadowRecord {
    fn draw<'a>(&'a self, pipelines: &'a ShadowPipelineStore) -> NativeOpaqueShadowDraw<'a> {
        NativeOpaqueShadowDraw {
            model_state_offset: self.uniform_offset,
            model_state: pipelines
                .by_id(self.pipeline)
                .binding
                .as_ref()
                .expect("prepared constant ring binding"),
            program_resources: self.resources.as_ref(),
            vertex_buffer: &self.vertex,
            index_buffer: &self.index,
            index_format: self.index_format,
            indices: self.indices.clone(),
            base_vertex: self.base_vertex,
            instances: self.instances.clone(),
            scissor: self.scissor,
        }
    }
}

/// Shared asset validation and program binding for directional and local
/// submissions. A missing resource rejects the entire prepared submission.
fn prepare_native_shadow_records(
    layers: &[NativeShadowLayer],
    depth_32_bits: bool,
    allow_cache: bool,
    uniform_buffer: Buffer,
    uniform_stride: u64,
    gpu: &mut NativeShadowSceneGpu,
    allocator: &Res<MeshAllocator>,
    meshes: &Res<RenderAssets<RenderMesh>>,
    images: &Res<RenderAssets<GpuImage>>,
    buffers: &Res<RenderAssets<GpuShaderBuffer>>,
    device: &RenderDevice,
    adapter: &RenderAdapter,
) -> Option<Arc<Vec<PreparedShadowRecord>>> {
    // Cache/clear operations govern submission, not descriptor preparation.
    let enabled = allow_cache && std::env::var("DH2_CACHE_PREPARED_SHADOWS").as_deref() != Ok("0");
    let verify = std::env::var("DH2_VERIFY_PREPARED_SHADOWS").as_deref() == Ok("1");
    let geometry_epoch = (allocator.last_changed().get(), meshes.last_changed().get());
    let resource_epoch = (images.last_changed().get(), buffers.last_changed().get());
    let uniforms = &gpu.uniform_records;
    let cached = if enabled {
        gpu.prepared_records.as_mut().and_then(|cached| {
            cached
                .matches(
                    layers,
                    geometry_epoch,
                    resource_epoch,
                    uniform_buffer.id(),
                    uniform_stride,
                    depth_32_bits,
                    uniforms,
                    images,
                    buffers,
                )
                .then(|| cached.records.clone())
        })
    } else {
        gpu.prepared_records = None;
        None
    };
    if enabled {
        if cached.is_some() {
            gpu.prepared_hits += 1;
        } else {
            gpu.prepared_misses += 1;
        }
        if gpu
            .last_revision
            .is_some_and(|revision| revision % 300 == 0)
        {
            eprintln!(
                "Prepared shadow descriptors: hits={} misses={}",
                gpu.prepared_hits, gpu.prepared_misses
            );
        }
    }
    if !verify {
        if let Some(cached) = cached {
            return Some(cached);
        }
    }
    let ring_id = uniform_buffer.id();
    let fresh = prepare_native_shadow_records_fresh(
        layers,
        depth_32_bits,
        uniform_buffer,
        uniform_stride,
        gpu,
        allocator,
        meshes,
        images,
        buffers,
        device,
        adapter,
    );
    let Some(fresh) = fresh else {
        gpu.prepared_records = None;
        return None;
    };
    if let Some(cached) = cached {
        prepared_cache::verify_records(&cached, &fresh);
        return Some(cached);
    }
    let fresh = Arc::new(fresh);
    if enabled {
        gpu.prepared_records = prepared_cache::PreparedRecordCache::new(
            layers,
            geometry_epoch,
            resource_epoch,
            ring_id,
            uniform_stride,
            depth_32_bits,
            &gpu.uniform_records,
            fresh.clone(),
            images,
            buffers,
        );
    }
    Some(fresh)
}

fn prepare_native_shadow_records_fresh(
    layers: &[NativeShadowLayer],
    depth_32_bits: bool,
    uniform_buffer: Buffer,
    uniform_stride: u64,
    gpu: &mut NativeShadowSceneGpu,
    allocator: &Res<MeshAllocator>,
    meshes: &Res<RenderAssets<RenderMesh>>,
    images: &RenderAssets<GpuImage>,
    sampling_buffers: &RenderAssets<GpuShaderBuffer>,
    device: &RenderDevice,
    adapter: &RenderAdapter,
) -> Option<Vec<PreparedShadowRecord>> {
    // Keep only descriptors actually used by this submission. GPU identities
    // invalidate uploads, streamed textures and reallocated buffers naturally.
    let mut previous = std::mem::take(&mut gpu.resource_bindings);
    let mut current = HashMap::with_capacity(previous.len());
    let mut allocations = std::mem::take(&mut gpu.mesh_allocations);
    let cache_meshes = std::env::var("DH2_CACHE_SHADOW_MESH_ALLOCATIONS").as_deref() != Ok("0");
    allocations.begin_frame(
        (allocator.last_changed().get(), meshes.last_changed().get()),
        cache_meshes,
    );
    let mut timings = (std::env::var("DH2_PROFILE_SHADOW_RECORDS").as_deref() == Ok("1"))
        .then(|| ([0u128; 3], HashMap::<NativeShadowProgram, usize>::new()));
    let result = prepare_shadow_records_with_bindings(
        layers,
        depth_32_bits,
        uniform_buffer,
        uniform_stride,
        gpu,
        allocator,
        meshes,
        images,
        sampling_buffers,
        device,
        adapter,
        &mut previous,
        &mut current,
        &mut allocations,
        timings.as_mut(),
    );
    gpu.resource_bindings = current;
    allocations.finish_frame(result.is_some());
    if std::env::var("DH2_TRACE_SHADOW_MESH_ALLOCATIONS").as_deref() == Ok("1")
        && gpu
            .last_revision
            .is_some_and(|revision| revision % 300 == 0)
    {
        eprintln!(
            "Shadow mesh allocation cache: hits={} misses={}",
            allocations.hits, allocations.misses
        );
    }
    gpu.mesh_allocations = allocations;
    if gpu
        .last_revision
        .is_some_and(|revision| revision % 300 == 0)
    {
        if let Some((times, counts)) = timings {
            eprintln!(
                "Shadow record preparation metadata/resources/packet ms={:?}; programs={counts:?}",
                times.map(|ns| ns as f64 / 1e6)
            );
        }
    }
    result
}

fn reuse_shadow_resource_binding(
    current: &mut HashMap<ShadowResourceKey, BindGroup>,
    previous: &mut HashMap<ShadowResourceKey, BindGroup>,
    key: ShadowResourceKey,
    create: impl FnOnce() -> Result<BindGroup, String>,
) -> Result<BindGroup, String> {
    match current.entry(key) {
        bevy::platform::collections::hash_map::Entry::Occupied(entry) => Ok(entry.get().clone()),
        bevy::platform::collections::hash_map::Entry::Vacant(entry) => {
            let binding = match previous.remove(entry.key()) {
                Some(binding) => binding,
                None => create()?,
            };
            entry.insert(binding.clone());
            Ok(binding)
        }
    }
}

fn prepare_shadow_records_with_bindings(
    layers: &[NativeShadowLayer],
    depth_32_bits: bool,
    uniform_buffer: Buffer,
    uniform_stride: u64,
    mut gpu: &mut NativeShadowSceneGpu,
    allocator: &MeshAllocator,
    meshes: &RenderAssets<RenderMesh>,
    images: &RenderAssets<GpuImage>,
    sampling_buffers: &RenderAssets<GpuShaderBuffer>,
    device: &RenderDevice,
    adapter: &RenderAdapter,
    previous: &mut HashMap<ShadowResourceKey, BindGroup>,
    resource_bindings: &mut HashMap<ShadowResourceKey, BindGroup>,
    mesh_allocations: &mut mesh_allocations::MeshAllocationCache,
    mut timings: Option<&mut ([u128; 3], HashMap<NativeShadowProgram, usize>)>,
) -> Option<Vec<PreparedShadowRecord>> {
    let mut records = Vec::with_capacity(layers.iter().map(|layer| layer.instances.len()).sum());
    let mut last_pipeline = None;
    // World packets vary their CB/remap offset, but most share the very
    // same immutable remap/matrix buffers and diffuse image. One resource
    // binding per actual combination suffices throughout this submission.
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
        let uniform = &gpu.uniform_records[slot];
        if (uniform.slot as u64)
            .checked_mul(uniform_stride)
            .is_none_or(|offset| offset > u64::from(u32::MAX))
        {
            bevy::log::error!("shadow constant ring offset exceeds dynamic binding range");
            return None;
        }
        let mut started = timings.as_ref().map(|_| std::time::Instant::now());
        if let Some((_, counts)) = timings.as_deref_mut() {
            *counts.entry(instance.program).or_default() += 1;
        }
        let mesh_id = instance.mesh.id();
        let allocation = match mesh_allocations.get(mesh_id, allocator, meshes) {
            Ok(allocation) => allocation,
            Err(error) => {
                bevy::log::error!("{error}");
                return None;
            }
        };
        let key = PipelineKey {
            layout: allocation.layout_key,
            depth_32_bits,
            front_face: instance.front_face,
            cull_mode: instance.cull_mode,
            program: instance.program,
            alpha_uv: instance
                .alpha_material
                .as_ref()
                .map(|material| material.uv_set),
            draw_state: instance.draw_state,
            stencil_reference: instance.stencil_reference,
        };
        let pipeline = match last_pipeline {
            Some((previous, id)) if previous == key => id,
            _ => match ensure_pipeline(&mut gpu, key, allocation.layout.clone(), device, adapter) {
                Ok(id) => {
                    last_pipeline = Some((key, id));
                    id
                }
                Err(error) => {
                    bevy::log::error!("native shadow pipeline unavailable: {error}");
                    return None;
                }
            },
        };
        if let Some((times, _)) = timings.as_deref_mut() {
            times[0] += started.unwrap().elapsed().as_nanos();
            started = Some(std::time::Instant::now());
        }
        let alpha_sampler = instance.alpha_material.as_ref().map(|_| {
            if matches!(
                instance.program,
                NativeShadowProgram::WorldVistaAlpha | NativeShadowProgram::RigidVistaAlpha
            ) || matches!(
                instance.program,
                NativeShadowProgram::Skinned { dither: true, .. }
            ) {
                gpu.vista_sampler
                    .get_or_insert_with(|| native_shadow_vista_sampler(&device))
                    .clone()
            } else {
                gpu.alpha_sampler
                    .get_or_insert_with(|| native_shadow_alpha_sampler(&device))
                    .clone()
            }
        });
        let cached = gpu.pipelines.by_id_mut(pipeline);
        let alpha_binding = if let NativeShadowInstanceConstants::Skinned { palette, .. } =
            &instance.per_instance_cb
        {
            let Some(palette) = sampling_buffers.get(palette.id()) else {
                return None;
            };
            let diffuse = if let Some(material) = &instance.alpha_material {
                let Some(image) = images.get(material.diffuse.id()) else {
                    return None;
                };
                Some((&image.texture_view, alpha_sampler.as_ref().unwrap()))
            } else {
                None
            };
            let resource_key = ShadowResourceKey::Skin(
                key,
                palette.buffer.id(),
                diffuse.map(|(view, _)| view.id()),
            );
            let binding =
                reuse_shadow_resource_binding(resource_bindings, previous, resource_key, || {
                    cached
                        .pipeline
                        .bind_skinned_instances(device, &palette.buffer, diffuse)
                });
            match binding {
                Ok(binding) => Some(binding),
                Err(error) => {
                    bevy::log::error!("native skin resources unavailable: {error}");
                    return None;
                }
            }
        } else if let NativeShadowInstanceConstants::WorldVegetation {
            constants,
            visible_indices,
            matrices,
            winds,
            instance_count,
        } = &instance.per_instance_cb
        {
            let (Some(ids), Some(transforms)) = (
                sampling_buffers.get(visible_indices.id()),
                sampling_buffers.get(matrices.id()),
            ) else {
                return None;
            };
            let wind_gpu = if let Some(handle) = winds {
                let Some(gpu) = sampling_buffers.get(handle.id()) else {
                    return None;
                };
                Some(gpu)
            } else {
                None
            };
            let wind_range = wind_gpu.map(|gpu| {
                (
                    u32::from_le_bytes(constants[204..208].try_into().unwrap()),
                    gpu.buffer.size(),
                    32u64,
                )
            });
            for (offset, buffer_size, stride) in std::iter::once((
                u32::from_le_bytes(constants[200..204].try_into().unwrap()),
                ids.buffer.size(),
                4u64,
            ))
            .chain(wind_range)
            {
                if u64::from(offset)
                    .checked_add(u64::from(*instance_count))
                    .and_then(|end| end.checked_mul(stride))
                    .is_none_or(|end| end > buffer_size)
                {
                    bevy::log::error!("native world vegetation packet exceeds its owner buffer");
                    return None;
                }
            }
            let Some(material) = &instance.alpha_material else {
                return None;
            };
            let Some(diffuse) = images.get(material.diffuse.id()) else {
                return None;
            };
            let resource_key = ShadowResourceKey::World(
                key,
                ids.buffer.id(),
                transforms.buffer.id(),
                wind_gpu.map(|wind| wind.buffer.id()),
                Some(diffuse.texture_view.id()),
            );
            let binding =
                reuse_shadow_resource_binding(resource_bindings, previous, resource_key, || {
                    if let Some(winds) = wind_gpu {
                        cached.pipeline.bind_world_vegetation_instances(
                            &device,
                            &ids.buffer,
                            &transforms.buffer,
                            &winds.buffer,
                            &diffuse.texture_view,
                            alpha_sampler.as_ref().unwrap(),
                        )
                    } else {
                        cached.pipeline.bind_world_alpha_instances(
                            &device,
                            &ids.buffer,
                            &transforms.buffer,
                            &diffuse.texture_view,
                            alpha_sampler.as_ref().unwrap(),
                        )
                    }
                });
            match binding {
                Ok(binding) => Some(binding),
                Err(error) => {
                    bevy::log::error!("native world vegetation unavailable: {error}");
                    return None;
                }
            }
        } else if let NativeShadowInstanceConstants::World {
            constants,
            visible_indices,
            matrices,
            instance_count,
        } = &instance.per_instance_cb
        {
            let (Some(ids), Some(transforms)) = (
                sampling_buffers.get(visible_indices.id()),
                sampling_buffers.get(matrices.id()),
            ) else {
                if std::env::var_os("DH2_TRACE_SHADOW_SUBMISSION").is_some() {
                    eprintln!(
                        "Native shadow pending IDs={:?} matrices={:?}",
                        visible_indices.id(),
                        matrices.id()
                    );
                }
                return None;
            }; // Retry after PrepareAssets; never drop world casters.
            let offset = u32::from_le_bytes(constants[64..68].try_into().unwrap()) as u64;
            if offset
                .checked_add(u64::from(*instance_count))
                .and_then(|end| end.checked_mul(4))
                .is_none_or(|end| end > ids.buffer.size())
            {
                bevy::log::error!("native world shadow packet exceeds visible-ID buffer");
                return None;
            }
            let diffuse = if let Some(material) = &instance.alpha_material {
                let Some(diffuse) = images.get(material.diffuse.id()) else {
                    return None;
                };
                Some(diffuse)
            } else {
                None
            };
            let resource_key = ShadowResourceKey::World(
                key,
                ids.buffer.id(),
                transforms.buffer.id(),
                None,
                diffuse.map(|image| image.texture_view.id()),
            );
            let binding =
                reuse_shadow_resource_binding(resource_bindings, previous, resource_key, || {
                    if let Some(diffuse) = diffuse {
                        cached.pipeline.bind_world_alpha_instances(
                            &device,
                            &ids.buffer,
                            &transforms.buffer,
                            &diffuse.texture_view,
                            alpha_sampler.as_ref().unwrap(),
                        )
                    } else {
                        cached.pipeline.bind_world_instances(
                            &device,
                            &ids.buffer,
                            &transforms.buffer,
                        )
                    }
                });
            match binding {
                Ok(binding) => Some(binding),
                Err(error) => {
                    bevy::log::error!("native world shadow buffers unavailable: {error}");
                    return None;
                }
            }
        } else if let Some(material) = &instance.alpha_material {
            let Some(diffuse) = images.get(material.diffuse.id()) else {
                bevy::log::error!(
                    "native shadow diffuse {:?} has no GPU image; retrying whole layer",
                    material.diffuse.id()
                );
                return None;
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
                    return None;
                }
            }
        } else {
            None
        };
        if let Some((times, _)) = timings.as_deref_mut() {
            times[1] += started.unwrap().elapsed().as_nanos();
            started = Some(std::time::Instant::now());
        }
        if let Err(error) = ensure_uniform_binding(cached, &uniform_buffer, &device) {
            bevy::log::error!("native shadow uniform binding unavailable: {error}");
            return None;
        }
        records.push(PreparedShadowRecord {
            layer: layer_index,
            scissor: instance.scissor,
            key,
            pipeline,
            uniform_offset: (gpu.uniform_records[slot].slot as u64 * uniform_stride) as u32,
            instances: gpu.uniform_records[slot].instances.clone(),
            vertex: allocation.vertex.clone(),
            base_vertex: allocation.base_vertex,
            index: allocation.index.clone(),
            indices: allocation.indices.clone(),
            index_format: allocation.index_format,
            resources: alpha_binding,
        });
        if let Some((times, _)) = timings.as_deref_mut() {
            times[2] += started.unwrap().elapsed().as_nanos();
        }
    }

    Some(records)
}

fn directional_layer_draws<'a>(
    records: &'a [PreparedShadowRecord],
    pipelines: &'a ShadowPipelineStore,
    layer_index: usize,
) -> Vec<(PipelineId, NativeOpaqueShadowDraw<'a>)> {
    let start = records.partition_point(|record| record.layer < layer_index);
    let end = records.partition_point(|record| record.layer <= layer_index);
    records[start..end]
        .iter()
        .map(|record| (record.pipeline, record.draw(pipelines)))
        .collect()
}

/// Preserve the publisher's native order. Grouping unrelated instances in a
/// HashMap reordered tied-depth and stencil draws, and crossed static/dynamic
/// cache boundaries. Pipeline changes share the attachment pass in that order.
fn draw_native_shadow_groups<'a>(
    encoder: &mut bevy::render::render_resource::CommandEncoder,
    temporary: &NativeShadowDepthArray,
    pipelines: &ShadowPipelineStore,
    draws: Vec<(PipelineId, NativeOpaqueShadowDraw<'a>)>,
    requested: NativeShadowLayerOps,
) -> Result<(), String> {
    if draws.is_empty() {
        if matches!(requested.depth, NativeShadowLoad::Clear(_))
            || matches!(requested.stencil, NativeStencilLoad::Clear(_))
        {
            clear_empty_layer(encoder, temporary, 0, requested);
        }
        return Ok(());
    }
    let sequence: Vec<_> = draws
        .iter()
        .map(|(key, draw)| (&pipelines.by_id(*key).pipeline, draw))
        .collect();
    let ops = layer_operations(
        requested,
        true,
        temporary.texture.format().has_stencil_aspect(),
    );
    NativeOpaqueShadowPipeline::draw_layer_sequence(encoder, temporary, 0, ops.0, ops.1, &sequence)
}

fn publish_sampling(
    gpu: &mut NativeShadowSceneGpu,
    prepared: Option<&GpuShaderBuffer>,
    queue: &RenderQueue,
    value: crate::shadow_sampling::NativeParallelSamplingUniform,
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
    receiver.receiver_maximum_distance()?;
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
    crate::shadow_sampling::NativeParallelSamplingUniform::from_owner_receiver(receiver)?;
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
    for (index, layer) in layers.iter().enumerate() {
        match layer.cache {
            NativeShadowCacheOp::None => {}
            NativeShadowCacheOp::Restore
            | NativeShadowCacheOp::Store { .. }
            | NativeShadowCacheOp::Retained { .. }
                if index == 0 || index >= cascade_count =>
            {
                return Err("native static cache excludes cascade0 and the auxiliary layer".into());
            }
            NativeShadowCacheOp::Restore => {
                if !matches!(layer.ops.depth, NativeShadowLoad::Load)
                    || !matches!(layer.ops.stencil, NativeStencilLoad::Load)
                {
                    return Err(
                        "native cache restore must precede a preserve-depth dynamic pass".into(),
                    );
                }
            }
            NativeShadowCacheOp::Store { static_instances }
            | NativeShadowCacheOp::Retained {
                static_instances, ..
            } => {
                if static_instances > layer.instances.len()
                    || !matches!(layer.ops.depth, NativeShadowLoad::Clear(_))
                {
                    return Err(
                        "native cache refresh needs a clear and a valid static/dynamic boundary"
                            .into(),
                    );
                }
            }
        }
    }
    validate_instances(dimensions, layers)
}

fn validate_whole_view_depth(
    dimensions: NativeParallelShadowTargetDimensions,
    layer: &NativeShadowLayer,
) -> Result<(), String> {
    if dimensions.width == 0 || dimensions.height == 0 || !(2..=5).contains(&dimensions.layers) {
        return Err("whole-view depth needs the actual directional target dimensions".into());
    }
    if layer.cache != NativeShadowCacheOp::None {
        return Err("the whole-view shadow has no per-cascade static-cache entry".into());
    }
    validate_instances(dimensions, std::slice::from_ref(layer))
}

fn validate_instances(
    dimensions: NativeParallelShadowTargetDimensions,
    layers: &[NativeShadowLayer],
) -> Result<(), String> {
    for layer in layers {
        for instance in &layer.instances {
            match (instance.program, &instance.alpha_material) {
                (
                    NativeShadowProgram::RigidOpaque
                    | NativeShadowProgram::WorldOpaque
                    | NativeShadowProgram::Skinned { alpha: false, .. },
                    None,
                ) => {}
                (
                    NativeShadowProgram::RigidAlphaTested
                    | NativeShadowProgram::RigidVistaAlpha
                    | NativeShadowProgram::Vegetation
                    | NativeShadowProgram::WorldAlphaTested
                    | NativeShadowProgram::WorldVistaAlpha
                    | NativeShadowProgram::WorldVegetation
                    | NativeShadowProgram::WorldVegetationFixed
                    | NativeShadowProgram::Skinned { alpha: true, .. },
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
                    ));
                }
            }
            match (&instance.per_instance_cb, instance.program) {
                (
                    NativeShadowInstanceConstants::Skinned { constants, .. },
                    NativeShadowProgram::Skinned {
                        influences,
                        alpha,
                        dither,
                    },
                ) => {
                    if !matches!(influences, 4 | 8) || (dither && !alpha) {
                        return Err("unsupported native skin class".into());
                    }
                    if let Some(material) = &instance.alpha_material {
                        for (i, value) in material.uv_scale_bias.iter().enumerate() {
                            if constants[96 + i * 4..100 + i * 4] != value.to_le_bytes() {
                                return Err("native skin UV parameters disagree".into());
                            }
                        }
                    }
                }
                (
                    NativeShadowInstanceConstants::WorldVegetation {
                        constants,
                        instance_count,
                        winds,
                        ..
                    },
                    NativeShadowProgram::WorldVegetation
                    | NativeShadowProgram::WorldVegetationFixed,
                ) => {
                    if winds.is_some() != (instance.program == NativeShadowProgram::WorldVegetation)
                    {
                        return Err(
                            "world vegetation program and wind-buffer ownership differ".into()
                        );
                    }
                    if *instance_count == 0 {
                        return Err("empty world vegetation packet".into());
                    }
                    let material = instance.alpha_material.as_ref().unwrap();
                    for (word, value) in material.uv_scale_bias.iter().enumerate() {
                        if constants[64 + word * 4..68 + word * 4] != value.to_le_bytes() {
                            return Err(
                                "world wind uniform and material UV scale/bias differ".into()
                            );
                        }
                    }
                }
                (
                    NativeShadowInstanceConstants::World {
                        constants,
                        instance_count,
                        ..
                    },
                    NativeShadowProgram::WorldOpaque
                    | NativeShadowProgram::WorldAlphaTested
                    | NativeShadowProgram::WorldVistaAlpha,
                ) => {
                    if let Some(material) = &instance.alpha_material {
                        for (word, value) in material.uv_scale_bias.iter().enumerate() {
                            if constants[80 + word * 4..84 + word * 4] != value.to_le_bytes() {
                                return Err(
                                    "world cutout uniform and source UV scale/bias differ".into()
                                );
                            }
                        }
                    }
                    if *instance_count == 0 {
                        return Err("native world shadow packet must contain instances".into());
                    }
                }
                (
                    NativeShadowInstanceConstants::Rigid(_),
                    NativeShadowProgram::RigidOpaque
                    | NativeShadowProgram::RigidAlphaTested
                    | NativeShadowProgram::RigidVistaAlpha,
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
                    );
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

fn hash_layout(layout: &MeshVertexBufferLayoutRef) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    layout.hash(&mut hasher);
    hasher.finish()
}

fn native_caster_position(program: NativeShadowProgram) -> MeshVertexAttribute {
    match program {
        NativeShadowProgram::WorldOpaque
        | NativeShadowProgram::WorldAlphaTested
        | NativeShadowProgram::WorldVistaAlpha
        | NativeShadowProgram::WorldVegetation
        | NativeShadowProgram::WorldVegetationFixed
        | NativeShadowProgram::Vegetation
        | NativeShadowProgram::RigidOpaque
        | NativeShadowProgram::RigidAlphaTested
        | NativeShadowProgram::RigidVistaAlpha
        | NativeShadowProgram::Skinned { .. } => NATIVE_VEGETATION_POSITION,
        _ => Mesh::ATTRIBUTE_POSITION,
    }
}

fn ensure_pipeline(
    gpu: &mut NativeShadowSceneGpu,
    key: PipelineKey,
    layout: MeshVertexBufferLayoutRef,
    device: &RenderDevice,
    adapter: &RenderAdapter,
) -> Result<PipelineId, String> {
    if let Some(id) = gpu.pipelines.id(&key) {
        return Ok(id);
    }
    if std::env::var_os("DH2_TRACE_SHADOW_SUBMISSION").is_some() {
        eprintln!(
            "Native shadow GPU creating pipeline {} {:?} {:?}",
            gpu.pipelines.len(),
            key.program,
            key.draw_state
        );
    }
    let position = native_caster_position(key.program);
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
    if matches!(
        key.program,
        NativeShadowProgram::Vegetation
            | NativeShadowProgram::WorldVegetation
            | NativeShadowProgram::WorldVegetationFixed
    ) {
        attributes.push(NATIVE_VEGETATION_COLOR.at_shader_location(2));
        attributes.push(NATIVE_VEGETATION_NORMAL.at_shader_location(3));
    }
    if let NativeShadowProgram::Skinned { .. } = key.program {
        if key.alpha_uv.is_none() {
            attributes.push(Mesh::ATTRIBUTE_UV_0.at_shader_location(1));
        }
        attributes.extend([
            NATIVE_SKIN_INDICES_0.at_shader_location(2),
            NATIVE_SKIN_WEIGHTS_0.at_shader_location(3),
            NATIVE_SKIN_INDICES_1.at_shader_location(4),
            NATIVE_SKIN_WEIGHTS_1.at_shader_location(5),
        ]);
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
    let program = match key.program {
        NativeShadowProgram::Skinned {
            influences,
            alpha,
            dither,
        } => NativeShadowDepthProgram::Skinned {
            influences,
            alpha,
            dither,
        },
        NativeShadowProgram::RigidOpaque => NativeShadowDepthProgram::Opaque,
        NativeShadowProgram::WorldOpaque => NativeShadowDepthProgram::WorldOpaque,
        NativeShadowProgram::WorldAlphaTested => NativeShadowDepthProgram::WorldAlpha,
        NativeShadowProgram::WorldVistaAlpha => NativeShadowDepthProgram::WorldVistaAlpha,
        NativeShadowProgram::WorldVegetation => NativeShadowDepthProgram::WorldVegetation,
        NativeShadowProgram::WorldVegetationFixed => NativeShadowDepthProgram::WorldVegetationFixed,
        NativeShadowProgram::RigidAlphaTested | NativeShadowProgram::RigidVistaAlpha => {
            NativeShadowDepthProgram::RigidAlpha
        }
        NativeShadowProgram::Vegetation => NativeShadowDepthProgram::Vegetation,
        NativeShadowProgram::Unsupported(_) => {
            return Err("unresolved native shadow program".into());
        }
    };
    let pipeline = NativeOpaqueShadowPipeline::new_prepared_shadow_depth(
        device,
        adapter,
        &raw,
        &key.draw_state,
        key.stencil_reference,
        parallel_target_format(key.depth_32_bits),
        key.front_face,
        key.cull_mode,
        program,
    )?;
    let id = gpu.pipelines.insert(
        key,
        CachedPipeline {
            layout,
            pipeline,
            binding: None,
            alpha_materials: HashMap::new(),
        },
    );
    if std::env::var_os("DH2_TRACE_SHADOW_SUBMISSION").is_some() {
        eprintln!(
            "Native shadow GPU prepared {} pipelines",
            gpu.pipelines.len()
        );
    }
    Ok(id)
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
            NativeShadowProgram::RigidAlphaTested | NativeShadowProgram::RigidVistaAlpha => {
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

fn ensure_uniform_binding(
    cached: &mut CachedPipeline,
    buffer: &Buffer,
    device: &RenderDevice,
) -> Result<(), String> {
    if cached.binding.is_none() {
        let binding = cached.pipeline.bind_prepared_model_state(device, buffer)?;
        cached.binding = Some(binding);
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
    #[test]
    fn canonical_rigid_constants_match_installed_liquid_and_vista_positions() {
        let mut count = 0;
        for line in include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/liquid_vista_shadow_positions.txt"
        ))
        .lines()
        {
            let w = line
                .split_whitespace()
                .map(|v| u32::from_str_radix(v, 16).unwrap())
                .collect::<Vec<_>>();
            let f = |i| f32::from_bits(w[i]);
            let rows = std::array::from_fn(|r| std::array::from_fn(|c| f(r * 4 + c)));
            let cb = native_shadow_instance_cb(&rows, f(16));
            let p = [f(17), f(18), f(19), 1.0];
            let dot = |offset: usize| {
                let row: [f32; 4] = std::array::from_fn(|i| {
                    f32::from_le_bytes(cb[offset + i * 4..offset + i * 4 + 4].try_into().unwrap())
                });
                ((row[0] * p[0] + row[1] * p[1]) + row[2] * p[2]) + row[3] * p[3]
            };
            let depth = f32::from_le_bytes(cb[64..68].try_into().unwrap());
            assert_eq!(
                [dot(48), dot(32), dot(16) * depth, dot(0)].map(f32::to_bits),
                w[20..],
                "native liquid/Vista position{count}"
            );
            count += 1;
        }
        assert_eq!(count, 768);
    }
    use super::*;

    #[test]
    fn pipeline_cache_does_not_alias_native_depth_stencil_or_raster_state() {
        let mut state = [0; 9];
        crate::render_state::reset_native_render_state(&mut state);
        let key = PipelineKey {
            layout: 0,
            depth_32_bits: false,
            front_face: FrontFace::Ccw,
            cull_mode: Some(Face::Back),
            program: NativeShadowProgram::WorldOpaque,
            alpha_uv: None,
            draw_state: state,
            stencil_reference: 0,
        };
        for word in 0..9 {
            let mut changed = key;
            changed.draw_state[word] ^= 1;
            assert_ne!(
                key, changed,
                "native state word {word} must not alias a cached pipeline"
            );
        }
        let mut changed = key;
        changed.stencil_reference = 71;
        assert_ne!(
            key, changed,
            "native stencil reference must survive submission"
        );
    }

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
            stencil_reference: 0,
            scissor: NativeShadowScissor {
                x: 0,
                y: 0,
                width: 32,
                height: 32,
            },
        };
        let mut layers = vec![
            NativeShadowLayer {
                cache: NativeShadowCacheOp::None,
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
            stencil_reference: 0,
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
                cache: NativeShadowCacheOp::None,
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
                cache: NativeShadowCacheOp::None,
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
        let mut prepared_state = [0; 9];
        crate::render_state::reset_native_render_state(&mut prepared_state);
        for program in [
            NativeShadowProgram::RigidOpaque,
            NativeShadowProgram::RigidAlphaTested,
            NativeShadowProgram::RigidVistaAlpha,
            NativeShadowProgram::Vegetation,
        ] {
            let key = PipelineKey {
                layout: hash_layout(&layout),
                depth_32_bits: false,
                front_face: FrontFace::Ccw,
                cull_mode: None,
                program,
                alpha_uv: (program != NativeShadowProgram::RigidOpaque).then_some(0),
                draw_state: prepared_state,
                stencil_reference: 0,
            };
            ensure_pipeline(&mut gpu, key.clone(), layout.clone(), device, adapter).unwrap();
            let cached = gpu.pipelines.get_mut(&key).unwrap();
            ensure_uniform_binding(cached, &ring, device).unwrap();
            let shared = cached.binding.as_ref().unwrap().id();
            // All ring offsets share a descriptor, including late frame slots.
            ensure_uniform_binding(cached, &ring, device).unwrap();
            assert_eq!(cached.binding.as_ref().unwrap().id(), shared);
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
                    matches!(
                        program,
                        NativeShadowProgram::RigidAlphaTested
                            | NativeShadowProgram::RigidVistaAlpha
                    )
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
            4,
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
            draw_state: prepared_state,
            stencil_reference: 0,
        };
        assert!(
            ensure_pipeline(&mut gpu, key, missing_raw, device, adapter,).is_err(),
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
            offset: crate::shadow_sampling::NativeShadowOffsetParameters {
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
    fn receiver_maximum_distance_selects_last_active_updated_far() {
        let fars = [4.0, 12.0, 30.0, 75.0];
        for cascade_count in 1..=4 {
            let mut vars = receiver();
            vars.world_to_shadow = vec![[[0.0; 4]; 4]; cascade_count + 1];
            vars.cascade_fars = fars;
            assert_eq!(
                vars.receiver_maximum_distance().unwrap(),
                fars[cascade_count - 1]
            );
        }
    }

    #[test]
    fn parallel_target_uses_actual_16_or_24_plus_stencil_format() {
        use bevy::render::render_resource::TextureFormat;
        let layer = NativeShadowLayer {
            cache: NativeShadowCacheOp::None,
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
    fn world_programs_reject_forward_baked_positions() {
        let mut mesh = Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[40.0f32, 50.0, 60.0]; 3]);
        let mut layouts = bevy::mesh::MeshVertexBufferLayouts::default();
        let baked = mesh.get_mesh_vertex_buffer_layout(&mut layouts);
        let programs = [
            NativeShadowProgram::WorldOpaque,
            NativeShadowProgram::WorldAlphaTested,
            NativeShadowProgram::WorldVistaAlpha,
            NativeShadowProgram::WorldVegetation,
            NativeShadowProgram::WorldVegetationFixed,
            NativeShadowProgram::RigidOpaque,
            NativeShadowProgram::RigidAlphaTested,
        ];
        for program in programs {
            let attribute = native_caster_position(program).at_shader_location(0);
            assert!(
                baked.0.get_layout(&[attribute]).is_err(),
                "{program:?} must not apply its matrix buffer to baked positions"
            );
        }
        mesh.insert_attribute(NATIVE_VEGETATION_POSITION, vec![[1.0f32, 2.0, 3.0]; 3]);
        let raw = mesh.get_mesh_vertex_buffer_layout(&mut layouts);
        for program in programs {
            raw.0
                .get_layout(&[native_caster_position(program).at_shader_location(0)])
                .unwrap();
        }
        baked
            .0
            .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])
            .unwrap();
    }

    #[test]
    fn native_static_cache_requires_real_cascade_clear_and_split() {
        let dims = NativeParallelShadowTargetDimensions {
            width: 32,
            height: 32,
            layers: 3,
            depth_32_bits: true,
        };
        let mut vars = receiver();
        vars.world_to_shadow.push(vars.world_to_shadow[0]);
        vars.cascade_fars = [10.0, 20.0, f32::MAX, f32::MAX];
        let mut layers = vec![
            NativeShadowLayer {
                cache: NativeShadowCacheOp::None,
                ops: NativeShadowLayerOps {
                    depth: NativeShadowLoad::Load,
                    stencil: NativeStencilLoad::Load,
                },
                instances: vec![],
            };
            3
        ];
        layers[1].cache = NativeShadowCacheOp::Restore;
        validate_frame(dims, &vars, &layers).unwrap();
        layers[0].cache = NativeShadowCacheOp::Restore;
        assert!(validate_frame(dims, &vars, &layers)
            .unwrap_err()
            .contains("excludes cascade0"));
        layers[0].cache = NativeShadowCacheOp::None;
        layers[2].cache = NativeShadowCacheOp::Restore;
        assert!(validate_frame(dims, &vars, &layers)
            .unwrap_err()
            .contains("auxiliary"));
        layers[2].cache = NativeShadowCacheOp::None;
        layers[1].ops.depth = NativeShadowLoad::Clear(1.0);
        assert!(validate_frame(dims, &vars, &layers)
            .unwrap_err()
            .contains("preserve-depth"));
        layers[1].cache = NativeShadowCacheOp::Store {
            static_instances: 1,
        };
        assert!(validate_frame(dims, &vars, &layers)
            .unwrap_err()
            .contains("boundary"));
        layers[1].cache = NativeShadowCacheOp::Store {
            static_instances: 0,
        };
        validate_frame(dims, &vars, &layers).unwrap();
        layers[1].ops.depth = NativeShadowLoad::Load;
        assert!(validate_frame(dims, &vars, &layers)
            .unwrap_err()
            .contains("needs a clear"));
    }

    #[test]
    fn screen_receiver_can_produce_atlas_without_enabling_direct_hook() {
        let dims = NativeParallelShadowTargetDimensions {
            width: 32,
            height: 32,
            layers: 2,
            depth_32_bits: true,
        };
        let layers = vec![
            NativeShadowLayer {
                cache: NativeShadowCacheOp::None,
                ops: NativeShadowLayerOps {
                    depth: NativeShadowLoad::Clear(1.0),
                    stencil: NativeStencilLoad::Clear(0),
                },
                instances: vec![],
            };
            2
        ];
        let mut vars = receiver();
        vars.path = crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace;
        validate_frame(dims, &vars, &layers).unwrap();
        let packed =
            crate::shadow_sampling::NativeParallelSamplingUniform::from_owner_receiver(&vars)
                .unwrap();
        assert_eq!(
            packed.config.x, 2,
            "the screen producer owns opaque visibility"
        );
        assert!(
            crate::shadow_sampling::NativeParallelSamplingUniform::from_receiver(&vars).is_err()
        );
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
                    cache: NativeShadowCacheOp::None,
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
            cache: NativeShadowCacheOp::None,
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
                stencil_reference: 0,
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
                cache: NativeShadowCacheOp::None,
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

#[cfg(test)]
#[path = "shadow_pass/uniform_tests.rs"]
mod uniform_tests;

#[path = "shadow_pass/mesh_allocations.rs"]
mod mesh_allocations;
