//! Dishonored 2 Rust game client using assets from a local installation.
#[path = "../archive.rs"]
mod archive;
#[path = "../bmd6.rs"]
mod bmd6;
#[path = "../bmodel.rs"]
mod bmodel;
#[path = "../bprobe.rs"]
mod bprobe;
#[path = "../entities.rs"]
mod entities;
#[path = "../material.rs"]
mod material;
#[path = "../material_surface.rs"]
mod material_surface;
#[path = "../surface_gpu.rs"]
mod surface_gpu;
#[path = "../navigation.rs"]
mod navigation;
#[path = "../physworld.rs"]
mod physworld;
#[path = "../pnetwork.rs"]
mod pnetwork;
#[path = "../probe_lighting.rs"]
mod probe_lighting;
#[path = "../render_light.rs"]
mod render_light;
#[path = "../render_model.rs"]
mod render_model;
#[path = "../runtime.rs"]
mod runtime;
#[path = "../texture.rs"]
mod texture;
#[path = "../tome.rs"]
mod tome;
#[path = "../world_mesh.rs"]
mod world_mesh;

use bevy::app::AppExit;
use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::camera::{PerspectiveProjection, Projection};
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::light::{
    CascadeShadowConfigBuilder, DirectionalLightShadowMap, PointLightShadowMap, RectLight,
};
use bevy::math::cubic_splines::LinearSpline;
use bevy::mesh::{Indices, MeshTag, MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
    MaterialPlugin,
};
use bevy::post_process::auto_exposure::{
    AutoExposure, AutoExposureCompensationCurve, AutoExposurePlugin,
};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, PrimitiveTopology, RenderPipelineDescriptor,
    SpecializedMeshPipelineError, TexelCopyBufferLayout, TextureDimension, TextureFormat, VertexFormat,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::storage::ShaderBuffer;
use bevy::render::texture::GpuImage;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::{Shader, ShaderRef};
use bevy::window::{CursorGrabMode, CursorOptions};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Resource)]
struct SceneData {
    source_path: PathBuf,
    world: world_mesh::WorldMesh,
    player_start: Option<entities::PlayerStart>,
    placed_models: Vec<entities::PlacedModel>,
    placed_lights: Vec<entities::PlacedLight>,
    environment_lighting: Option<entities::EnvironmentLighting>,
    environment_probes: Option<probe_lighting::ProbeLightingSet>,
    pickups: Vec<entities::Pickup>,
    catalog: Option<archive::Catalog>,
}

#[derive(Component)]
struct PickupVisual(String);

struct PickupInstance {
    entity_id: String,
    position: Vec3,
}

#[derive(Resource)]
struct PickupState {
    items: Vec<PickupInstance>,
    world: runtime::WorldState,
}

#[derive(Resource)]
struct NavigationWorld(navigation::CollisionWorld);

#[derive(Resource)]
struct TomeWorld {
    bytes: Vec<u8>,
    tree: tome::SpatialTree,
    gate_states: Vec<u32>,
    query_data: tome::DirectTomeQueryData,
    sphere_graph: Result<tome::SphereCellGraph, String>,
    query_job_count: u32,
}

#[derive(Resource)]
struct NavigationState {
    walking: bool,
    walker: navigation::Walker,
}

#[derive(Component)]
struct CoordinateOverlay;

#[derive(Clone)]
struct FocusTarget {
    center: Vec3,
    distance: f32,
    label: String,
    spawn: Option<entities::PlayerStart>,
}

#[derive(Resource)]
struct FocusTargets {
    targets: Vec<FocusTarget>,
    active: usize,
}

#[derive(Resource)]
struct SnapshotPath(PathBuf);

#[derive(Resource)]
struct BenchmarkSeconds(f32);

#[derive(Resource)]
struct OriginalLocalLightCullEnabled(bool);

#[derive(Resource, Default)]
struct OriginalLocalLightCullStats {
    total: usize,
    zero_preset_rgb: usize,
    proximity_culled: usize,
    proximity_unverified: usize,
    projection_bounds_unverified: usize,
    source_class_counts: [usize; 2],
    static_name_hash_matches: usize,
    static_id_culled: usize,
    static_id_binding_active: bool,
    world_query_bound: bool,
    world_query_unverified: usize,
    world_query_error: Option<String>,
    sphere_candidates: usize,
    sphere_culled: usize,
    sphere_unverified: usize,
    sphere_query_ns: u128,
    sphere_error: Option<String>,
    clip_culled: usize,
    area_culled: usize,
    total_by_kind: [usize; 4],
    visible_by_kind: [usize; 4],
}

#[derive(Component, Clone, Copy)]
struct OriginalLocalLightCull {
    range: f32,
    kind: entities::LightType,
    culling: entities::LightCulling,
    definition_name_hash: u32,
    linear_color: [f32; 3],
    projection_bounds: Option<(Vec3, Vec3)>,
    proximity_bounds: Option<(Vec3, f32)>,
    initial_sphere: Option<[f32; 4]>,
    clip_to_world: Option<Mat4>,
}

// Native gather retains candidate storage between calls too. Keep the point
// lookup and traversal in one batch, rather than allocating per-light masks.
#[derive(Default)]
struct LocalLightSphereBatch {
    worker: tome::SphereCellWorker,
    spheres: Vec<[f32; 4]>,
    point_cells: Vec<Option<u32>>,
    entities: Vec<Entity>,
}

// The non-Umbra branch uses 128 / viewport area (32 physical pixels).
// The world-query branch instead consumes the native normalized area metric.
const ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS: f32 = 32.0;
const ORIGINAL_LIGHT_MIN_RANGE: f32 = 0.1;
const ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS: f32 = 32_768.0;

#[derive(Resource, Default)]
struct VisibilityElapsedTiming {
    samples: u64,
    total_ns: u128,
    max_ns: u128,
    query_ns: u128,
    mapping_ns: u128,
    update_ns: u128,
}

// Keep exact per-frame inputs in memory, but flush ordinary disk samples only
// once per second or on failure/large visibility changes. No rendering fallback.
#[derive(Resource)]
struct StreamingDiagnostics {
    frame: u64,
    pending: String,
    history: VecDeque<String>,
    log: Option<fs::File>,
    log_path: PathBuf,
    status: String,
    previous_status: String,
    previous_active: usize,
    last_logged: Instant,
}

impl StreamingDiagnostics {
    fn new(source_path: &std::path::Path) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let directory = source_path
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("streaming-reports");
        let log_path = directory.join(format!("streaming-{stamp}-{}.log", std::process::id()));
        let log = fs::create_dir_all(&directory).and_then(|()| fs::File::create(&log_path));
        let log = match log {
            Ok(file) => {
                println!(
                    "Streaming diagnostics: {} (F8 captures the last 120 frames)",
                    log_path.display()
                );
                Some(file)
            }
            Err(error) => {
                eprintln!(
                    "Streaming diagnostics {}: {error}; keeping the in-memory F8 history",
                    log_path.display()
                );
                None
            }
        };
        Self {
            frame: 0,
            pending: String::new(),
            history: VecDeque::with_capacity(120),
            log,
            log_path,
            status: String::new(),
            previous_status: String::new(),
            previous_active: 0,
            last_logged: Instant::now(),
        }
    }

    fn record(&mut self, sample: String, active: usize, capture: bool) {
        let changed = self.status != self.previous_status;
        let cliff = active.abs_diff(self.previous_active) > active.max(self.previous_active) / 2;
        if changed || cliff || capture || self.last_logged.elapsed().as_secs_f32() >= 1.0 {
            if changed || cliff {
                println!(
                    "STREAM frame={} status={} active={} (was {})",
                    self.frame, self.status, active, self.previous_active
                );
            }
            let write_result = self
                .log
                .as_mut()
                .map(|log| writeln!(log, "{sample}").and_then(|()| log.flush()));
            if let Some(Err(error)) = write_result {
                eprintln!(
                    "Streaming log {}: {error}; continuing in-memory capture",
                    self.log_path.display()
                );
                self.log = None;
            }
            self.last_logged = Instant::now();
        }
        if self.history.len() == 120 {
            self.history.pop_front();
        }
        self.history.push_back(sample);
        self.previous_status.clone_from(&self.status);
        self.previous_active = active;
    }
}

struct RenderInstance {
    mesh: Handle<Mesh>,
    material: MaterialHandle,
    diffuse_image_slot: Option<usize>,
    probe_index: u32,
    transform: Transform,
    bounds: Aabb,
    pickup_id: Option<String>,
    entity: Option<Entity>,
    visible: bool,
}

type DecalMaterial = ExtendedMaterial<StandardMaterial, DecalExtension>;
type ProbeMaterial = ExtendedMaterial<StandardMaterial, ProbeExtension>;
const PROBE_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("169be3e1-273e-4607-aadf-e0d572cbf41d");
const SURFACE_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("b7018b4d-b81d-44c0-9dd0-0b64cc879316");
const DECAL_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("9165b294-a09d-46cb-af56-27edbe334a1b");
const ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES: f32 = 80.0;
const ORIGINAL_DEFAULT_NEAR: f32 = 0.01;
const ORIGINAL_DEFAULT_FAR: f32 = 7000.0;

#[derive(Clone, PartialEq, Eq, Hash)]
enum MaterialHandle {
    Probe(Handle<ProbeMaterial>),
    Decal(Handle<DecalMaterial>),
}

#[derive(Clone)]
struct ResolvedMaterial {
    handle: MaterialHandle,
    diffuse_image_slot: Option<usize>,
    probe_index: u32,
}

#[derive(Clone)]
struct CachedMaterial {
    handle: MaterialHandle,
    diffuse_image_slot: Option<usize>,
    images: Vec<(usize, surface_gpu::ImageRole)>,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
struct ProbeExtension {
    #[storage(100, read_only)]
    sh: Handle<ShaderBuffer>,
    #[uniform(101)]
    surface: surface_gpu::SurfaceUniform,
    #[texture(102)]
    #[sampler(117)]
    diffuse0: Option<Handle<Image>>,
    #[texture(103)]
    diffuse1: Option<Handle<Image>>,
    #[texture(104)]
    diffuse2: Option<Handle<Image>>,
    #[texture(105)]
    bump0: Option<Handle<Image>>,
    #[texture(106)]
    bump1: Option<Handle<Image>>,
    #[texture(107)]
    bump2: Option<Handle<Image>>,
    #[texture(108)]
    #[sampler(118)]
    gloss0: Option<Handle<Image>>,
    #[texture(109)]
    gloss1: Option<Handle<Image>>,
    #[texture(110)]
    gloss2: Option<Handle<Image>>,
    #[texture(111)]
    metallic0: Option<Handle<Image>>,
    #[texture(112)]
    metallic1: Option<Handle<Image>>,
    #[texture(113)]
    metallic2: Option<Handle<Image>>,
    #[texture(114)]
    mask0: Option<Handle<Image>>,
    #[texture(115)]
    mask1: Option<Handle<Image>>,
    #[texture(116)]
    mask2: Option<Handle<Image>>,
}

// Original common vertex shader transforms N/T/B independently with the
// model's linear rows. A separate bitangent avoids replacing that rule with
// inverse-transpose normals or a post-transform cross product under shear.
const DH2_BITANGENT: MeshVertexAttribute = MeshVertexAttribute::new("DH2_Bitangent", 0x4448_3200, VertexFormat::Float32x3);

fn authored_basis_pipeline(descriptor: &mut RenderPipelineDescriptor, layout: &MeshVertexBufferLayoutRef) -> Result<(), SpecializedMeshPipelineError> {
    if layout.0.contains(DH2_BITANGENT) {
        let extra = layout.0.get_layout(&[DH2_BITANGENT.at_shader_location(8)])?;
        descriptor.vertex.buffers[0].attributes.extend(extra.attributes);
        descriptor.vertex.shader_defs.push("DH2_AUTHORED_BASIS".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_AUTHORED_BASIS".into());
        }
    }
    Ok(())
}

impl MaterialExtension for ProbeExtension {
    fn fragment_shader() -> ShaderRef {
        PROBE_SHADER_HANDLE.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        authored_basis_pipeline(descriptor, layout)
    }

    // This path evaluates baked SH in the forward fragment shader. Disabling
    // the deferred prepass keeps its authored probe contribution in that path.
    fn enable_prepass() -> bool {
        false
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(DecalKey)]
struct DecalExtension {
    bias: i32,
    #[uniform(101)]
    surface: surface_gpu::SurfaceUniform,
    #[texture(102)]
    #[sampler(117)]
    diffuse0: Option<Handle<Image>>,
    #[texture(103)]
    diffuse1: Option<Handle<Image>>,
    #[texture(104)]
    diffuse2: Option<Handle<Image>>,
    #[texture(105)]
    bump0: Option<Handle<Image>>,
    #[texture(106)]
    bump1: Option<Handle<Image>>,
    #[texture(107)]
    bump2: Option<Handle<Image>>,
    #[texture(108)]
    #[sampler(118)]
    gloss0: Option<Handle<Image>>,
    #[texture(109)]
    gloss1: Option<Handle<Image>>,
    #[texture(110)]
    gloss2: Option<Handle<Image>>,
    #[texture(111)]
    metallic0: Option<Handle<Image>>,
    #[texture(112)]
    metallic1: Option<Handle<Image>>,
    #[texture(113)]
    metallic2: Option<Handle<Image>>,
    #[texture(114)]
    mask0: Option<Handle<Image>>,
    #[texture(115)]
    mask1: Option<Handle<Image>>,
    #[texture(116)]
    mask2: Option<Handle<Image>>,
}

trait NativeImageAccess {
    fn image_mut(&mut self, role: surface_gpu::ImageRole) -> &mut Option<Handle<Image>>;
}
macro_rules! native_image_access {
    ($ty:ty) => {
        impl NativeImageAccess for $ty {
            fn image_mut(&mut self, role: surface_gpu::ImageRole) -> &mut Option<Handle<Image>> {
                match role.0 {
                    0 => &mut self.diffuse0,
                    1 => &mut self.diffuse1,
                    2 => &mut self.diffuse2,
                    3 => &mut self.bump0,
                    4 => &mut self.bump1,
                    5 => &mut self.bump2,
                    6 => &mut self.gloss0,
                    7 => &mut self.gloss1,
                    8 => &mut self.gloss2,
                    9 => &mut self.metallic0,
                    10 => &mut self.metallic1,
                    11 => &mut self.metallic2,
                    12 => &mut self.mask0,
                    13 => &mut self.mask1,
                    14 => &mut self.mask2,
                    _ => panic!("invalid authored image role"),
                }
            }
        }
    };
}
native_image_access!(ProbeExtension);
native_image_access!(DecalExtension);

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct DecalKey {
    bias: i32,
}

impl From<&DecalExtension> for DecalKey {
    fn from(extension: &DecalExtension) -> Self {
        Self {
            bias: extension.bias,
        }
    }
}

impl MaterialExtension for DecalExtension {
    fn fragment_shader() -> ShaderRef { DECAL_SHADER_HANDLE.into() }

    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            // Bevy's StandardMaterial.depth_bias only sorts transparent draws;
            // the Void arkDecal z-bias needs an actual GPU depth offset.
            depth.bias.constant = key.bind_group_data.bias;
        }
        authored_basis_pipeline(descriptor, layout)
    }
}

#[derive(Resource, Default)]
struct SceneRenderQueue {
    instances: Vec<RenderInstance>,
    active: usize,
    tome_render_instances: TomeRenderInstanceMap,
    // FUN_14037ef90's sorted authored-ID output. Its render-entity consumer
    // is still unverified; never equate these IDs with unmatched BWM records.
    visible_authored_ids: Vec<u32>,
    // FUN_14037ef90 -> FUN_140a73580/FUN_140a70570 light-query inputs.
    visible_cell_indices: Vec<u32>,
    visible_cell_mask: Vec<u32>,
    light_query_state: Option<tome::RenderQueryState>,
    visibility_suppression_groups: Vec<RenderSuppressionGroup>,
    diffuse_image_demands: Vec<texture::ProjectedAreaDemand>,
    diffuse_image_paths: Vec<String>,
    diffuse_image_materials: Vec<Vec<(MaterialHandle, surface_gpu::ImageRole)>>,
    native_material_images: HashMap<MaterialHandle, Vec<usize>>,
    diffuse_image_handles: Vec<Option<Handle<Image>>>,
    diffuse_image_pending: Vec<Option<Handle<Image>>>,
    diffuse_mip_states: Vec<DiffuseMipState>,
    diffuse_image_attempted: Vec<bool>,
    diffuse_image_last_visible: Vec<u64>,
    texture_stream_frame: u64,
    texture_viewport_pixels: u32,
    diffuse_mip_upgrades: u64,
    preferred_mip: Option<u8>,
}

impl SceneRenderQueue {
    fn publish_light_query_outputs(
        &mut self,
        world: &TomeWorld,
        output: &tome::RenderQueryObjectOutput,
    ) -> Result<(), String> {
        let ids = self
            .tome_render_instances
            .visible_authored_ids(&world.bytes, output.indices())?;
        let mask =
            tome::cell_visibility_mask(world.query_data.cell_count(), output.cell_indices())?;
        self.visible_authored_ids = ids;
        self.visible_cell_indices.clear();
        self.visible_cell_indices
            .extend_from_slice(output.cell_indices());
        self.visible_cell_mask = mask;
        self.light_query_state = output.state.clone();
        Ok(())
    }
}

#[derive(Resource, Default)]
struct PlacedModelQueries(Vec<PlacedModelQueryGroup>);

struct PlacedModelQueryGroup {
    indices: std::ops::Range<usize>,
    input: render_model::ModelQueryInput,
}

struct RenderSuppressionGroup {
    parent: Vec<usize>,
    children: Vec<usize>,
}

impl RenderSuppressionGroup {
    fn apply_all(groups: &[Self], visible: &mut [bool]) {
        for group in groups {
            if group
                .parent
                .iter()
                .any(|&index| visible.get(index) == Some(&true))
            {
                for &index in &group.children {
                    if let Some(value) = visible.get_mut(index) {
                        *value = false;
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DiffuseMipLevel {
    resource_mip: Option<u8>,
    next_separate_suffix: Option<u8>,
    width: u32,
    height: u32,
    format: texture::CompressedFormat,
}

#[derive(Default)]
struct DiffuseMipState {
    resident: Option<DiffuseMipLevel>,
    pending: Option<DiffuseMipLevel>,
    minimum: Option<DiffuseMipLevel>,
    minimum_mip_count: u32,
    pending_reduction: bool,
    unavailable_finer: Option<u8>,
}

struct StreamedDiffuse {
    image: Image,
    uploads: Vec<CompressedMipUpload>,
    level: DiffuseMipLevel,
}

struct CompressedMipUpload {
    mip_level: u32,
    width: u32,
    height: u32,
    payload: Vec<u8>,
}

// Registered executable defaults: r_textureStreamingPerFrameBudgetKB and
// r_maxFramesBeforeTextureUnload. Bevy uploads changed images after this
// system, so the byte limit bounds staged payloads, not completed GPU work.
const TEXTURE_UPLOAD_BUDGET_KB: f32 = 4096.0;
const TEXTURE_REDUCE_AFTER_FRAMES: u64 = 60;
const TEXTURE_READ_QUEUE_CAPACITY: usize = 16;

#[derive(Default)]
struct PreparedImageState {
    pending: Vec<PendingGpuImageUpload>,
    completed: Vec<AssetId<Image>>,
    failed: Vec<(AssetId<Image>, String)>,
}

struct PendingGpuImageUpload {
    id: AssetId<Image>,
    previous: Option<AssetId<Image>>,
    copy: GpuMipCopy,
    uploads: Vec<CompressedMipUpload>,
}

#[derive(Clone, Copy)]
enum GpuMipCopy {
    None,
    Extend,
    Reduce { dropped: u32 },
}

#[derive(Resource, Clone, Default)]
struct PreparedImageMailbox(Arc<Mutex<PreparedImageState>>);

fn report_prepared_diffuse_images(
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
fn upload_compressed_mips(
    render_device: &RenderDevice,
    render_queue: &RenderQueue,
    image: &GpuImage,
    previous: Option<&GpuImage>,
    copy: GpuMipCopy,
    uploads: &[CompressedMipUpload],
) -> Result<(), String> {
    let descriptor = &image.texture_descriptor;
    let block_bytes = streamed_block_bytes(descriptor.format).ok_or("unsupported streamed GPU image format")?;
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
        let expected = width.div_ceil(4) as usize * height.div_ceil(4) as usize * block_bytes;
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
                bytes_per_row: Some(upload.width.div_ceil(4) * block_bytes as u32),
                rows_per_image: Some(upload.height.div_ceil(4)),
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

#[derive(Clone, Copy)]
enum TextureReadTarget {
    Initial(Option<u8>),
    Exact(u8, texture::CompressedFormat),
}

type TextureReadResult = (
    usize,
    TextureReadTarget,
    Result<Option<StreamedDiffuse>, String>,
);

#[derive(Resource)]
struct TextureReadWorker {
    requests: SyncSender<(usize, String, TextureReadTarget)>,
    completed: Mutex<Receiver<TextureReadResult>>,
    ready: VecDeque<TextureReadResult>,
    inflight: Vec<bool>,
    inflight_count: usize,
}

impl TextureReadWorker {
    fn new(catalog: archive::Catalog, image_count: usize) -> Self {
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

fn read_streamed_diffuse_image(
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

fn read_streamed_diffuse_at_target(
    catalog: &archive::Catalog,
    stem: &str,
    target: TextureReadTarget,
) -> Result<Option<StreamedDiffuse>, String> {
    if let TextureReadTarget::Exact(mip, format) = target {
        let path = format!("{stem}_mip{mip}");
        let Some(bytes) = catalog.read(&path).map_err(|error| error.to_string())? else { return Ok(None); };
        let compressed = texture::compressed_mip(&bytes, u32::from(mip), format)?;
        let uploads = vec![CompressedMipUpload {
            mip_level: 0, width: compressed.width, height: compressed.height, payload: compressed.payload.to_vec(),
        }];
        return Ok(Some(StreamedDiffuse {
            image: empty_streamed_image(compressed.width, compressed.height, 1, gpu_image_format(format)),
            uploads,
            level: DiffuseMipLevel { resource_mip: Some(mip), next_separate_suffix: mip.checked_sub(1),
                width: compressed.width, height: compressed.height, format },
        }));
    }
    let Some(bytes) = catalog.read(stem).map_err(|error| error.to_string())? else {
        return Err(format!("missing base BIM format descriptor {stem}"));
    };
    let inline = texture::compressed_inline_chain(&bytes)?;
    let format = inline.info.format;
    let highest_separate = (0..=15).rev().find(|mip| catalog.contains(&format!("{stem}_mip{mip}")));
    let selected_separate = match target {
        TextureReadTarget::Initial(None) => None,
        TextureReadTarget::Initial(Some(preferred)) => (0..=preferred).rev().find(|mip| catalog.contains(&format!("{stem}_mip{mip}"))),
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
            let bytes = catalog.read(&path).map_err(|error| error.to_string())?.ok_or_else(|| format!("missing authored lower mip {path}"))?;
            let compressed = texture::compressed_mip(&bytes, u32::from(mip), format)?;
            if let Some((width, height)) = previous {
                if compressed.width != (width / 2).max(1) || compressed.height != (height / 2).max(1) {
                    return Err("authored separate mip dimensions are not contiguous".into());
                }
            }
            top.get_or_insert((compressed.width, compressed.height));
            previous = Some((compressed.width, compressed.height));
            uploads.push(CompressedMipUpload { mip_level: mip_count, width: compressed.width, height: compressed.height, payload: compressed.payload.to_vec() });
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
        let size = width.div_ceil(4) as usize * height.div_ceil(4) as usize * format.block_bytes();
        uploads.push(CompressedMipUpload { mip_level: mip_count + index, width, height, payload: inline.payload[offset..offset + size].to_vec() });
        offset += size;
    }
    mip_count += inline.info.mip_count;
    let (width, height) = top.ok_or("authored image has no mip payload")?;
    Ok(Some(StreamedDiffuse {
        image: empty_streamed_image(width, height, mip_count, gpu_image_format(format)),
        uploads,
        level: DiffuseMipLevel { resource_mip: selected_separate,
            next_separate_suffix: selected_separate.and_then(|mip| mip.checked_sub(1)).or_else(|| if selected_separate.is_none() { highest_separate } else { None }),
            width, height, format },
    }))
}

fn gpu_image_format(format: texture::CompressedFormat) -> TextureFormat {
    use texture::CompressedFormat::*;
    match format {
        Bc1 => TextureFormat::Bc1RgbaUnorm, Bc1Srgb => TextureFormat::Bc1RgbaUnormSrgb,
        Bc3 => TextureFormat::Bc3RgbaUnorm, Bc3Srgb => TextureFormat::Bc3RgbaUnormSrgb,
        Bc4 => TextureFormat::Bc4RUnorm, Bc4Snorm => TextureFormat::Bc4RSnorm,
        Bc5 => TextureFormat::Bc5RgUnorm, Bc5Snorm => TextureFormat::Bc5RgSnorm,
        Bc6Unsigned => TextureFormat::Bc6hRgbUfloat, Bc6Signed => TextureFormat::Bc6hRgbFloat,
        Bc7 => TextureFormat::Bc7RgbaUnorm, Bc7Srgb => TextureFormat::Bc7RgbaUnormSrgb,
    }
}

fn streamed_block_bytes(format: TextureFormat) -> Option<usize> {
    match format {
        TextureFormat::Bc1RgbaUnorm | TextureFormat::Bc1RgbaUnormSrgb | TextureFormat::Bc4RUnorm | TextureFormat::Bc4RSnorm => Some(8),
        TextureFormat::Bc3RgbaUnorm | TextureFormat::Bc3RgbaUnormSrgb | TextureFormat::Bc5RgUnorm | TextureFormat::Bc5RgSnorm
        | TextureFormat::Bc6hRgbUfloat | TextureFormat::Bc6hRgbFloat | TextureFormat::Bc7RgbaUnorm | TextureFormat::Bc7RgbaUnormSrgb => Some(16),
        _ => None,
    }
}

fn empty_streamed_image(width: u32, height: u32, mip_count: u32, format: TextureFormat) -> Image {
    let mut image = Image::new_uninit(Extent3d { width, height, depth_or_array_layers: 1 },
        TextureDimension::D2, format, RenderAssetUsages::RENDER_WORLD);
    image.texture_descriptor.mip_level_count = mip_count;
    let mut sampler = ImageSamplerDescriptor::linear();
    sampler.set_address_mode(ImageAddressMode::Repeat);
    image.sampler = ImageSampler::Descriptor(sampler);
    image
}

fn attach_prepared_diffuse_image(
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
                        if role.0 == 0 { material.base.base_color_texture = Some(handle.clone()); }
                    }
                }
            }
            MaterialHandle::Decal(material) => {
                if let Some(mut material) = decal_materials.get_mut(material) {
                    if *role == surface_gpu::ImageRole::occlusion() {
                        material.base.occlusion_texture = Some(handle.clone());
                    } else {
                        *material.extension.image_mut(*role) = Some(handle.clone());
                        if role.0 == 0 { material.base.base_color_texture = Some(handle.clone()); }
                    }
                }
            }
        }
    }
    if let Some(old) = queue.diffuse_image_handles[slot].replace(handle) {
        images.remove(old.id());
    }
}

fn commit_prepared_diffuse_level(
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
fn stream_visible_textures(
    worker: Option<ResMut<TextureReadWorker>>,
    prepared_images: Option<Res<PreparedImageMailbox>>,
    mut queue: ResMut<SceneRenderQueue>,
    mut images: ResMut<Assets<Image>>,
    mut probe_materials: ResMut<Assets<ProbeMaterial>>,
    mut decal_materials: ResMut<Assets<DecalMaterial>>,
) {
    let Some(mut worker) = worker else {
        return;
    };
    queue.texture_stream_frame += 1;
    let frame = queue.texture_stream_frame;
    let mut requested = vec![false; queue.diffuse_image_paths.len()];
    for instance in &queue.instances {
        if instance.visible {
            if let Some(slots) = queue.native_material_images.get(&instance.material) {
                for &slot in slots { requested[slot] = true; }
            } else if let Some(slot) = instance.diffuse_image_slot {
                if let Some(needed) = requested.get_mut(slot) { *needed = true; }
            }
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
            } else {
                queue.diffuse_mip_states[slot].pending_reduction = false;
                images.remove(id);
                if queue.diffuse_image_handles[slot].is_none() {
                    queue.diffuse_image_attempted[slot] = false;
                }
            }
        }
    }
    for (slot, &needed) in requested.iter().enumerate() {
        if needed {
            queue.diffuse_image_last_visible[slot] = frame;
        } else if frame.saturating_sub(queue.diffuse_image_last_visible[slot])
            > TEXTURE_REDUCE_AFTER_FRAMES
        {
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
            let handle = images.add(empty_streamed_image(
                minimum.width, minimum.height, minimum_count, gpu_image_format(minimum.format),
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
        if !requested[slot] {
            if queue.diffuse_image_handles[slot].is_none() {
                queue.diffuse_image_attempted[slot] = false;
            }
            continue;
        }
        let path = &queue.diffuse_image_paths[slot];
        let mut streamed = match result {
            Ok(Some(image)) => image,
            Ok(None) => {
                eprintln!("Streamed diffuse missing: {path}");
                if let TextureReadTarget::Exact(mip, _) = target {
                    queue.diffuse_mip_states[slot].unavailable_finer = Some(mip);
                }
                continue;
            }
            Err(error) => {
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
        budget.record_upload(upload_bytes as i32);
        let mip_count = streamed.image.texture_descriptor.mip_level_count;
        let handle = images.add(streamed.image);
        if let Some(prepared_images) = prepared_images.as_ref() {
            queue.diffuse_image_pending[slot] = Some(handle.clone());
            queue.diffuse_mip_states[slot].pending = Some(streamed.level);
            prepared_images
                .0
                .lock()
                .unwrap()
                .pending
                .push(PendingGpuImageUpload {
                    id: handle.id(),
                    previous,
                    copy: if previous.is_some() {
                        GpuMipCopy::Extend
                    } else {
                        GpuMipCopy::None
                    },
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
            commit_prepared_diffuse_level(&mut queue, slot, streamed.level, mip_count);
        }
    }
    let request_order =
        texture::projected_area_request_order(&requested, &queue.diffuse_image_demands)
            .expect("diffuse request flags align with image demands");
    for slot in request_order {
        if worker.inflight[slot]
            || queue.diffuse_image_pending[slot].is_some()
            || worker.inflight_count == TEXTURE_READ_QUEUE_CAPACITY
        {
            continue;
        }
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
                // The original scheduler clears accumulated area only when
                // it accepts work for that image, not on every frame.
                queue.diffuse_image_demands[slot].take();
                if matches!(target, TextureReadTarget::Initial(_)) {
                    queue.diffuse_image_attempted[slot] = true;
                }
                worker.inflight[slot] = true;
                worker.inflight_count += 1;
            }
            Err(mpsc::TrySendError::Full(_)) => break,
            Err(mpsc::TrySendError::Disconnected(_)) => {
                eprintln!("Texture read worker stopped");
                break;
            }
        }
    }
}

/// Flatten the authored Tome object -> BWM visibility record -> client
/// render-instance mapping once, preserving the order and multiplicity of
/// both original association tables.
#[derive(Debug, Default)]
struct TomeRenderInstanceMap {
    ranges: Vec<std::ops::Range<usize>>,
    indices: Vec<usize>,
    empty_bwm_groups: Vec<bool>,
    spatial_query_instances: Vec<SpatialQueryRenderInstance>,
    zero_metric_instances: HashSet<usize>,
}

#[derive(Debug)]
struct SpatialQueryRenderInstance {
    bounds: [[f32; 3]; 2],
    render_indices: Vec<usize>,
}

impl TomeRenderInstanceMap {
    fn from_world_mapping(
        world: &world_mesh::WorldMesh,
        bwm_visibility_render_instances: &[Vec<usize>],
    ) -> Self {
        let mut map = Self::from_authored_bwm_mapping(
            &world.tome_object_instances,
            bwm_visibility_render_instances,
        );
        for &index in &world.spatial_query_instances {
            let render_indices = bwm_visibility_render_instances[index].clone();
            map.zero_metric_instances
                .extend(render_indices.iter().copied());
            map.spatial_query_instances
                .push(SpatialQueryRenderInstance {
                    bounds: world.visibility[index].bounds,
                    render_indices,
                });
        }
        map
    }

    fn from_authored_bwm_mapping(
        tome_object_bwm_instances: &[Vec<usize>],
        bwm_visibility_render_instances: &[Vec<usize>],
    ) -> Self {
        let index_count = tome_object_bwm_instances
            .iter()
            .flatten()
            .filter_map(|&bwm_index| bwm_visibility_render_instances.get(bwm_index))
            .map(Vec::len)
            .sum();
        let mut map = Self {
            ranges: Vec::with_capacity(tome_object_bwm_instances.len()),
            indices: Vec::with_capacity(index_count),
            empty_bwm_groups: Vec::with_capacity(tome_object_bwm_instances.len()),
            spatial_query_instances: Vec::new(),
            zero_metric_instances: HashSet::new(),
        };
        for bwm_indices in tome_object_bwm_instances {
            map.empty_bwm_groups.push(bwm_indices.is_empty());
            let start = map.indices.len();
            for &bwm_index in bwm_indices {
                if let Some(render_indices) = bwm_visibility_render_instances.get(bwm_index) {
                    map.indices.extend_from_slice(render_indices);
                }
            }
            map.ranges.push(start..map.indices.len());
        }
        map
    }

    fn get(&self, tome_object: usize) -> &[usize] {
        self.ranges
            .get(tome_object)
            .map(|range| &self.indices[range.start..range.end])
            .unwrap_or_default()
    }

    /// Apply successful FUN_14037e1d0 query results to their BWM render
    /// instances. The original caller marks every instance in each accepted
    /// Tome object's serialized group; an empty group follows a separate
    /// authored-ID path and therefore marks no BWM instance here.
    fn mark_visible_instances(
        &self,
        visible_tome_indices: &[u32],
        visible_render_instances: &mut [bool],
    ) -> Result<(), String> {
        for &tome_object in visible_tome_indices {
            if tome_object as usize >= self.ranges.len() {
                return Err("visible Tome index exceeds render-instance mapping".into());
            }
            for &render_index in self.get(tome_object as usize) {
                let Some(visible) = visible_render_instances.get_mut(render_index) else {
                    return Err("Tome render-instance mapping exceeds visibility output".into());
                };
                *visible = true;
            }
        }
        Ok(())
    }

    /// FUN_14037ef90 overwrites the separate +0x418 BWM list's visibility
    /// after worker/state merge, and clears both of those instances' metrics.
    fn apply_spatial_queries(
        &self,
        state: Option<&tome::RenderQueryState>,
        visible_render_instances: &mut [bool],
    ) -> Result<usize, String> {
        let mut accepted = 0;
        // Original query descriptor setup precedes all spatial-instance tests;
        // do not rebuild its eight finite frustum corners for every object.
        let query = if self.spatial_query_instances.is_empty() {
            None
        } else {
            state.map(tome::NoDepthAabbQuery::from_state).transpose()?
        };
        for instance in &self.spatial_query_instances {
            let visible = match &query {
                Some(query) => query.visible(instance.bounds)?,
                None => true,
            };
            accepted += usize::from(visible);
            for &index in &instance.render_indices {
                let target = visible_render_instances
                    .get_mut(index)
                    .ok_or("Tome spatial-instance mapping exceeds visibility output")?;
                *target = visible;
            }
        }
        Ok(accepted)
    }

    /// FUN_14037e1d0 emits IDs only for an empty authored BWM group, not
    /// for a nonempty group whose LOD surfaces are absent from the client.
    /// FUN_14037ef90 appends the job lists and sorts unsigned IDs; duplicates
    /// remain. Resolving these IDs to render entities is still unsupported.
    fn visible_authored_ids(
        &self,
        bytes: &[u8],
        visible_tome_indices: &[u32],
    ) -> Result<Vec<u32>, String> {
        let mut ids = Vec::new();
        for &index in visible_tome_indices {
            let empty = self
                .empty_bwm_groups
                .get(index as usize)
                .ok_or("visible Tome index exceeds authored BWM group mapping")?;
            if *empty {
                ids.extend(tome::object_ids_for_index(bytes, index)?);
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    /// Forward the original accepted Tome indices and their projected-area
    /// outputs to the diffuse images on the associated static BWM instances.
    fn submit_texture_demands(
        &self,
        visible_tome_indices: &[u32],
        area_metrics: &[Option<f32>],
        render_instances: &[RenderInstance],
        image_demands: &[texture::ProjectedAreaDemand],
    ) -> Result<usize, String> {
        self.submit_material_texture_demands(visible_tome_indices, area_metrics, render_instances, image_demands, &HashMap::new())
    }

    fn submit_material_texture_demands(
        &self,
        visible_tome_indices: &[u32],
        area_metrics: &[Option<f32>],
        render_instances: &[RenderInstance],
        image_demands: &[texture::ProjectedAreaDemand],
        bindings: &HashMap<MaterialHandle, Vec<usize>>,
    ) -> Result<usize, String> {
        if visible_tome_indices.len() != area_metrics.len() {
            return Err("Tome visibility indices and area outputs have different lengths".into());
        }
        let mut submitted = 0;
        for (&tome_object, area_metric) in visible_tome_indices.iter().zip(area_metrics) {
            if tome_object as usize >= self.ranges.len() {
                return Err("visible Tome index exceeds render-instance mapping".into());
            }
            let Some(area_metric) = area_metric else {
                continue;
            };
            for &render_index in self.get(tome_object as usize) {
                let Some(instance) = render_instances.get(render_index) else {
                    return Err("Tome render-instance mapping exceeds scene render queue".into());
                };
                let primary = instance.diffuse_image_slot.into_iter().collect::<Vec<_>>();
                let slots = bindings.get(&instance.material).unwrap_or(&primary);
                for &image_slot in slots {
                    let Some(demand) = image_demands.get(image_slot) else {
                        return Err("render instance references an unknown material image".into());
                    };
                    // Spatial suppression applies to every shader image role.
                    demand.submit(if self.zero_metric_instances.contains(&render_index) { 0.0 } else { *area_metric });
                    submitted += 1;
                }
            }
        }
        Ok(submitted)
    }
}

fn render_bounds(points: &[Vec3]) -> Aabb {
    let mut low = Vec3::splat(f32::INFINITY);
    let mut high = Vec3::splat(f32::NEG_INFINITY);
    for &point in points {
        low = low.min(point);
        high = high.max(point);
    }
    Aabb::from_min_max(low, high)
}

fn authored_world_bounds(record: &world_mesh::WorldVisibility) -> Aabb {
    let [low, high] = record.bounds;
    Aabb::from_min_max(
        Vec3::new(low[0], low[2], -high[1]),
        Vec3::new(high[0], high[2], -low[1]),
    )
}

struct Bounds {
    low: Vec3,
    high: Vec3,
    points: usize,
}

impl Bounds {
    fn empty() -> Self {
        Self {
            low: Vec3::splat(f32::INFINITY),
            high: Vec3::splat(f32::NEG_INFINITY),
            points: 0,
        }
    }
    fn add(&mut self, point: Vec3) {
        self.low = self.low.min(point);
        self.high = self.high.max(point);
        self.points += 1;
    }
    fn focus(&self, label: String) -> FocusTarget {
        FocusTarget {
            center: (self.low + self.high) * 0.5,
            distance: ((self.high - self.low).length() * 0.8).max(12.0),
            label,
            spawn: None,
        }
    }
}

fn view_of(target: &FocusTarget) -> Transform {
    if let Some(start) = target.spawn {
        let eye = game_to_view(start.position) + Vec3::Y * 1.7;
        let forward =
            game_to_view([start.forward_xy[0], start.forward_xy[1], 0.0]).normalize_or_zero();
        return Transform::from_translation(eye).looking_at(eye + forward * 10.0, Vec3::Y);
    }
    let distance = target.distance;
    Transform::from_translation(
        target.center + Vec3::new(distance * 0.45, distance * 0.4, distance),
    )
    .looking_at(target.center, Vec3::Y)
}

fn original_light_preset(
    light: &entities::PlacedLight,
    linear_color: [f32; 3],
) -> Option<render_light::PresetOutput> {
    let (cutoff, falloff) = original_light_distance(light.distance_cutoff, light.distance_falloff)?;
    let preset = render_light::preset_index(light.preset.as_deref())?;
    let reflector = if preset == 11 {
        render_light::reflector_index(light.reflector_preset.as_deref())?
    } else {
        0 // The native dispatcher does not read this input for other presets.
    };
    let kind = match light.kind {
        entities::LightType::Point => 0,
        entities::LightType::HalfPoint => 1,
        entities::LightType::Spot => 2,
        entities::LightType::Area => 3,
        entities::LightType::Parallel => 4,
    };
    render_light::convert(render_light::PresetInput {
        kind,
        preset,
        reflector,
        area_energy_conservative: light.area_energy_conservative,
        cutoff,
        falloff,
        cutoff_cosine: authored_light_cosines(light.angular_cutoff, light.angular_falloff).0,
        back_reflection: light.back_reflection?,
        area_size: light.area_size,
        color: linear_color,
    })
}

fn light_backend_intensity(
    normalized_intensity: f32,
    kind: entities::LightType,
    rect_size: [f32; 2],
) -> Option<f32> {
    let intensity = if kind == entities::LightType::Area {
        if !rect_size.iter().all(|size| size.is_finite() && *size > 0.0) {
            return None;
        }
        let area = rect_size[0] * rect_size[1];
        if !(area * std::f32::consts::PI).is_finite() || area <= 0.0 {
            return None;
        }
        // Cancel Bevy's extra emitter-area divisor; the native preset already
        // handles conservative area energy. Absolute photometry is unverified.
        (normalized_intensity as f64 * area as f64 * 2_000.0) as f32
    } else {
        10_000.0 * (normalized_intensity / 5.0)
    };
    (intensity.is_finite() && intensity >= 0.0).then_some(intensity)
}

fn authored_light_cosines(cutoff_degrees: f32, falloff_degrees: f32) -> (f32, f32) {
    // FUN_140881640 passes m_AngularFalloff as FUN_140359b30 param_2 and
    // m_AngularCutoff as param_3. The helper clamps cutoff, then raises falloff.
    const MAX_COSINE: f32 = 0.999_847_7;
    const GAP: f64 = 0.000_152_304_841_3;
    let cutoff = (cutoff_degrees * std::f32::consts::PI / 360.0)
        .cos()
        .clamp(-MAX_COSINE, MAX_COSINE);
    let mut falloff = (falloff_degrees * std::f32::consts::PI / 360.0).cos();
    if (falloff as f64) <= cutoff as f64 + GAP {
        falloff = (cutoff as f64 + GAP) as f32;
    }
    (cutoff, falloff)
}

fn original_light_distance(cutoff: f32, falloff: f32) -> Option<(f32, f32)> {
    if !cutoff.is_finite() || !falloff.is_finite() {
        return None;
    }
    Some(render_light::distance_fields(cutoff, falloff))
}

fn preserved_half_point_angular_scale(cutoff_degrees: f32, falloff_degrees: f32) -> f32 {
    // Keep the pre-existing viewer ramp until the original HalfPoint shader is
    // verified; this is not the source-backed Spot projection threshold.
    const MAX_SINE: f32 = 0.999_847_7;
    const GAP: f32 = 0.000_152_304_84;
    let falloff_sine = (falloff_degrees * std::f32::consts::PI / 360.0)
        .sin()
        .clamp(-MAX_SINE, MAX_SINE);
    let cutoff_sine = (cutoff_degrees * std::f32::consts::PI / 360.0).sin();
    let boundary_angle = cutoff_sine.max(falloff_sine + GAP).asin();
    -1.0 / boundary_angle.sin().max(0.000_001)
}

// FUN_140359990 builds the light projection; FUN_140232bc0 stores its inverse
// at +0x130. FUN_14031bcc0 multiplies it by camera clip-from-world and tests the
// transformed [0,1]^3 light volume through FUN_140109dd0 / FUN_14010a0d0.
fn original_local_light_clip_to_world(
    position: Vec3,
    rotation: [[f32; 3]; 3],
    kind: entities::LightType,
    range: f32,
    cutoff_cosine: Option<f32>,
) -> Option<Mat4> {
    if !position.is_finite() || !range.is_finite() || range <= 0.0 {
        return None;
    }

    // Ghidra data: DAT_141ab3764=0.5, DAT_141f697b0=1.0. The spot path uses
    // 1/range longitudinally and 0.5/(range*tan(half-angle)) transversely.
    let spot = kind == entities::LightType::Spot;
    let depth_scale = (if spot { 1.0 } else { 0.5 }) / range;
    let side_scale = if spot {
        let cosine = cutoff_cosine?;
        let tangent = (1.0 - cosine * cosine).max(0.0).sqrt() / cosine;
        0.5 / (range * tangent)
    } else {
        depth_scale
    };
    if !side_scale.is_finite() {
        return None;
    }

    let projection = Mat4::from_cols(
        Vec4::new(depth_scale, 0.0, 0.0, 0.0),
        Vec4::new(0.0, side_scale, 0.0, 0.0),
        Vec4::new(0.0, 0.0, side_scale, 0.0),
        Vec4::new(0.5, 0.5, 0.5, 1.0),
    );
    let axes = rotation.map(game_to_view);
    let light_to_world = Mat4::from_cols(
        axes[0].extend(0.0),
        axes[1].extend(0.0),
        axes[2].extend(0.0),
        position.extend(1.0),
    );
    let world_to_clip = projection * light_to_world.inverse();
    let clip_to_world = world_to_clip.inverse();
    clip_to_world
        .to_cols_array()
        .iter()
        .all(|value| value.is_finite())
        .then_some(clip_to_world)
}

fn original_light_volume_intersects_camera(
    clip_from_world: Mat4,
    clip_to_world: Mat4,
) -> Option<bool> {
    let clip_from_light = clip_from_world * clip_to_world;
    let mut corners = [Vec4::ZERO; 8];
    for (index, corner) in corners.iter_mut().enumerate() {
        let point = Vec3::new(
            (index & 1) as f32,
            ((index >> 1) & 1) as f32,
            ((index >> 2) & 1) as f32,
        );
        *corner = clip_from_light * point.extend(1.0);
        if !corner.is_finite() {
            return None;
        }
    }

    // FUN_14010a0d0 rejects when all volume corners are outside one clip plane.
    let all_outside = [
        corners.iter().all(|p| p.x < -p.w),
        corners.iter().all(|p| p.x > p.w),
        corners.iter().all(|p| p.y < -p.w),
        corners.iter().all(|p| p.y > p.w),
        corners.iter().all(|p| p.z < 0.0),
        corners.iter().all(|p| p.z > p.w),
    ];
    Some(!all_outside.into_iter().any(|outside| outside))
}

fn game_to_view([x, y, z]: [f32; 3]) -> Vec3 {
    Vec3::new(x, z, -y)
}

fn environment_sun_ray(
    environment: entities::EnvironmentLighting,
    authored_light_azimuth: Option<f32>,
) -> Option<Vec3> {
    let theta = environment.sun_theta_degrees?.to_radians();
    let phi = authored_light_azimuth
        .or(environment.sun_phi_degrees)?
        .to_radians();
    // The EOTW authored parallel-light axis uses [cos(phi), -sin(phi)] in
    // game XY. Use its azimuth when available; otherwise use environment phi.
    // Treat environment theta as elevation and aim directional rays downward.
    Some(
        game_to_view([
            theta.cos() * phi.cos(),
            -theta.cos() * phi.sin(),
            -theta.sin(),
        ])
        .normalize_or_zero(),
    )
}

fn view_to_game(point: Vec3) -> [f32; 3] {
    [point.x, -point.z, point.y]
}

fn transformed(matrix: &[f32; 12], [x, y, z]: [f32; 3]) -> Vec3 {
    game_to_view([
        matrix[0] * x + matrix[1] * y + matrix[2] * z + matrix[3],
        matrix[4] * x + matrix[5] * y + matrix[6] * z + matrix[7],
        matrix[8] * x + matrix[9] * y + matrix[10] * z + matrix[11],
    ])
}

fn transformed_direction(matrix: &[f32; 12], v: Vec3) -> Vec3 {
    let [x, y, z] = v.to_array();
    game_to_view([
        matrix[0] * x + matrix[1] * y + matrix[2] * z,
        matrix[4] * x + matrix[5] * y + matrix[6] * z,
        matrix[8] * x + matrix[9] * y + matrix[10] * z,
    ])
}

fn placed_direction(placement: &entities::PlacedModel, v: Vec3) -> Vec3 {
    let scaled = std::array::from_fn::<_, 3, _>(|i| v[i] * placement.scale[i]);
    game_to_view(std::array::from_fn(|row| {
        (0..3).map(|i| placement.rotation[i][row] * scaled[i]).sum()
    }))
}

fn placed_vertex(placement: &entities::PlacedModel, vertex: [f32; 3]) -> Vec3 {
    let scaled = [
        vertex[0] * placement.scale[0],
        vertex[1] * placement.scale[1],
        vertex[2] * placement.scale[2],
    ];
    game_to_view(std::array::from_fn(|row| {
        placement.position[row]
            + (0..3)
                // idMat3 stores its three basis axes as mat[0..2]. A local
                // vertex component weights the corresponding authored axis.
                .map(|column| placement.rotation[column][row] * scaled[column])
                .sum::<f32>()
    }))
}

fn game_affine_to_view(matrix: &[f32; 12]) -> Transform {
    let game = Mat4::from_cols(
        Vec4::new(matrix[0], matrix[4], matrix[8], 0.0),
        Vec4::new(matrix[1], matrix[5], matrix[9], 0.0),
        Vec4::new(matrix[2], matrix[6], matrix[10], 0.0),
        Vec4::new(matrix[3], matrix[7], matrix[11], 1.0),
    );
    let axes = Mat4::from_cols(
        Vec4::X,
        Vec4::new(0.0, 0.0, -1.0, 0.0),
        Vec4::new(0.0, 1.0, 0.0, 0.0),
        Vec4::W,
    );
    Transform::from_matrix(axes * game * axes.inverse())
}

fn placed_transform(placement: &entities::PlacedModel) -> Transform {
    let rows = render_model::affine_rows(placement.position, placement.rotation, placement.scale);
    let matrix = std::array::from_fn(|index| rows[index / 4][index % 4]);
    game_affine_to_view(&matrix)
}

fn initial_placed_model_query(
    placement: &entities::PlacedModel,
    local_bounds: [[f32; 3]; 2],
) -> Option<render_model::ModelQueryInput> {
    let range = placement
        .max_visible_range
        .unwrap_or(render_model::INITIAL_MODEL_MAX_VISIBLE_RANGE);
    let bounds = render_model::transformed_bounds(
        local_bounds,
        placement.position,
        placement.rotation,
        placement.scale,
    );
    Some(render_model::ModelQueryInput {
        // FUN_140233a30 starts frame byte at1; initial bounds preparation sets4.
        // Surface bit40 is unbound: positive regular-path results are also
        // accepted by its bypass. Negative results remain unsupported there.
        frame_flags: 1 | 4,
        shape_flags: 0, // Inert with prepared frame bit4, regardless of its value.
        region: render_model::range_region(bounds, range),
        bounds,
    })
}

fn apply_placed_model_queries(
    visible: &mut [bool],
    groups: &[PlacedModelQueryGroup],
    state: &tome::RenderQueryState,
    camera: [f32; 3],
    viewport: [u32; 2],
) -> Result<usize, String> {
    if groups
        .iter()
        .any(|group| group.indices.start > group.indices.end || group.indices.end > visible.len())
    {
        return Err("placed model query indices exceed render instances".into());
    }
    let query = tome::NoDepthAabbQuery::from_state(state)?;
    let mut accepted = 0;
    for group in groups {
        if render_model::model_visible(&query, camera, viewport, group.input)? {
            visible[group.indices.clone()].fill(true);
            accepted += 1;
        }
    }
    Ok(accepted)
}

fn expanded_model_placements(
    placements: &[entities::PlacedModel],
    catalog: &archive::Catalog,
) -> Vec<entities::PlacedModel> {
    let mut expanded = placements.to_vec();
    let mut index = 0;
    while index < expanded.len() {
        let placement = &expanded[index];
        if placement.resource_path.ends_with(".blueprint.decl") {
            if placement.id.matches("#blueprint[").count() >= 16 {
                eprintln!("Blueprint nesting exceeds 16: {}", placement.id);
            } else {
                match catalog.read(&placement.resource_path) {
                    Ok(Some(bytes)) => match entities::expand_blueprint(
                        placement,
                        &String::from_utf8_lossy(&bytes),
                    ) {
                        Ok(children) => expanded.extend(children),
                        Err(error) => eprintln!(
                            "Blueprint parse failed {}: {error}",
                            placement.resource_path
                        ),
                    },
                    Ok(None) => eprintln!("Blueprint not indexed: {}", placement.resource_path),
                    Err(error) => {
                        eprintln!("Blueprint read failed {}: {error}", placement.resource_path)
                    }
                }
            }
        }
        index += 1;
    }
    expanded
}

fn model_havok_path(placement: &entities::PlacedModel, kind: &str, hd: bool) -> Option<String> {
    if physworld::physics_shape_kind(kind) == 4 && !kind.eq_ignore_ascii_case("cvd") {
        return None;
    }
    let source = if let Some(model) = placement.resource_path.strip_prefix("generated/model/") {
        model.strip_suffix(".bmodel")?;
        physworld::replace_shape_extension(model, "lwo")
    } else {
        // The root renderer retains .blueprint; its children have no bodies.
        placement.resource_path.strip_prefix("generated/decls/blueprint/")?
            .strip_suffix(".decl")?.to_owned()
    };
    if placement.scale.iter().any(|scale| !(scale * 100.0).is_finite()) {
        return None;
    }
    let key = physworld::physics_shape_key(&source, physworld::physics_shape_kind(kind), placement.scale, 0.01, hd)?;
    Some(physworld::physics_shape_asset_path(&key))
}

fn placed_collision_vertex(placement: &entities::PlacedModel, vertex: [f32; 3]) -> Vec3 {
    // The HKX filename's three percentages identify geometry already baked at
    // the placed model scale. Apply only the instance orientation and origin.
    game_to_view(std::array::from_fn(|row| {
        placement.position[row]
            + (0..3)
                .map(|column| placement.rotation[column][row] * vertex[column])
                .sum::<f32>()
    }))
}

fn load_model_havok(
    catalog: &archive::Catalog,
    path: &str,
    depth: usize,
) -> Result<Vec<[[f32; 3]; 3]>, String> {
    if depth > 4 {
        return Err("HKX alias chain exceeds four links".into());
    }
    let bytes = catalog
        .read(path)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("HKX target not indexed: {path}"))?;
    if let Some((target, factor)) = physworld::standalone_hkx_alias(&bytes) {
        let mut triangles = load_model_havok(catalog, &target, depth + 1)?;
        for triangle in &mut triangles {
            for vertex in triangle {
                for axis in 0..3 {
                    vertex[axis] *= factor;
                }
            }
        }
        return Ok(triangles);
    }
    let physics = physworld::parse_hkx(&bytes)?;
    let geometry = physics.standalone_shapes(&bytes)?;
    if !geometry.compressed_decode_failures.is_empty() || !geometry.unknown_shapes.is_empty() {
        return Err(format!(
            "incomplete HKX geometry: {:?}, {:?}",
            geometry.compressed_decode_failures, geometry.unknown_shapes
        ));
    }
    Ok(geometry
        .triangles
        .into_iter()
        .map(|triangle| triangle.vertices)
        .collect())
}

fn static_model_collision(
    catalog: &archive::Catalog,
    placements: &[entities::PlacedModel],
    bodies: &[entities::RigidBodySettings],
) -> (Vec<([Vec3; 3], u32)>, Vec<String>, usize, usize) {
    let roots: HashMap<_, _> = placements.iter()
        .map(|placement| (placement.id.as_str(), placement)).collect();
    let mut cached = HashMap::<String, Option<Vec<[[f32; 3]; 3]>>>::new();
    let mut triangles = Vec::new();
    let mut owners = Vec::new();
    let mut matched = 0usize;
    let mut failed = 0usize;
    for body in bodies {
        let Some(&placement) = roots.get(body.entity_id.as_str()) else { continue; };
        // Dynamic/keyframed bodies need simulation, not frozen walk colliders.
        if body.physics_behavior.as_deref().and_then(entities::rigid_body_behavior_kind) != Some(0) { continue; }
        let kind = match body.shape_type.as_deref().and_then(entities::rigid_body_shape_kind) {
            Some(index) => ["hkt", "cvx", "msh", "box", "cvd"][index as usize],
            other => {
                failed += 1;
                eprintln!("Collision {}: unsupported declared shape {:?} ({other:?})", placement.id, body.shape_type);
                continue;
            }
        };
        let Some(path) = model_havok_path(placement, kind, false) else {
            failed += 1;
            eprintln!("Collision {}: unsupported resource recipe", placement.id);
            continue;
        };
        if body.extra_hd_material.is_some() {
            eprintln!("Collision {}: primary shape only; ExtraHD acquisition still unsupported", placement.id);
        }
        let local = cached.entry(path.clone()).or_insert_with(|| {
            let parsed = load_model_havok(catalog, &path, 0);
            match parsed {
                Ok(triangles) => Some(triangles),
                Err(error) => {
                    eprintln!("Model collision {path}: {error}");
                    None
                }
            }
        });
        let Some(local) = local else {
            failed += 1;
            continue;
        };
        matched += 1;
        let owner = owners.len() as u32;
        owners.push(placement.id.clone());
        // FUN_140aa0360 is applied to the entity basis before native body creation.
        let body_basis = physworld::rigid_body_basis(placement.rotation.map(|[x, y, z]| [x, y, z, 0.0]));
        let body_pose = entities::PlacedModel {
            rotation: body_basis.map(|[x, y, z, _]| [x, y, z]),
            ..placement.clone()
        };
        triangles.extend(local.iter().map(|triangle| {
            (
                triangle.map(|vertex| placed_collision_vertex(&body_pose, vertex)),
                owner,
            )
        }));
    }
    (triangles, owners, matched, failed)
}

fn mesh_from_points(points: &[Vec3], uvs: &[[f32; 2]], indices: &[u16]) -> Mesh {
    let mut normals = vec![Vec3::ZERO; points.len()];
    for face in indices.chunks_exact(3) {
        let [a, b, c] = [face[0] as usize, face[1] as usize, face[2] as usize];
        let normal = (points[c] - points[a]).cross(points[b] - points[a]);
        normals[a] += normal;
        normals[b] += normal;
        normals[c] += normal;
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        points
            .iter()
            .map(|point| point.to_array())
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        normals
            .into_iter()
            .map(|normal| normal.normalize_or_zero().to_array())
            .collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec())
    .with_inserted_indices(Indices::U16(indices.to_vec()))
}

fn mean_game_position(points: &[Vec3]) -> [f32; 3] {
    let mean = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
    view_to_game(mean)
}

fn authored_mesh<const N: usize>(
    points: &[Vec3], uvs: &[[f32; 2]], indices: &[u16], packed: &[[u8; N]],
    transform_direction: impl Fn(Vec3) -> Vec3,
) -> Mesh {
    assert_eq!(points.len(), packed.len(), "authored vertex channel count");
    assert!(N == 12 || N == 16);
    let mut normals = Vec::with_capacity(points.len());
    let mut tangents = Vec::with_capacity(points.len());
    let mut bitangents = Vec::with_capacity(points.len());
    let mut uv1 = Vec::with_capacity(points.len());
    let mut colors = Vec::with_capacity(points.len());
    for p in packed {
        let snorm = |b: u8| ((b as i8 as f32) / 127.0).max(-1.0);
        let n = Vec3::new(snorm(p[4]), snorm(p[5]), snorm(p[6]));
        let t = Vec3::new(snorm(p[8]), snorm(p[9]), snorm(p[10]));
        let w = snorm(p[11]);
        normals.push(transform_direction(n).to_array());
        tangents.push(transform_direction(t).extend(w).to_array());
        bitangents.push(transform_direction(n.cross(t) * w).to_array());
        uv1.push([
            half::f16::from_bits(u16::from_le_bytes([p[0], p[1]])).to_f32(),
            half::f16::from_bits(u16::from_le_bytes([p[2], p[3]])).to_f32(),
        ]);
        if N == 16 { colors.push(std::array::from_fn::<_, 4, _>(|i| p[12 + i] as f32 / 255.0)); }
    }
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, points.iter().map(|p| p.to_array()).collect::<Vec<_>>())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec())
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uv1)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tangents)
        .with_inserted_attribute(DH2_BITANGENT, bitangents)
        .with_inserted_indices(Indices::U16(indices.to_vec()));
    if N == 16 { mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors); }
    mesh
}

fn source_mesh<const N: usize>(positions: &[[f32; 3]], uvs: &[[f32; 2]], indices: &[u16], packed: &[[u8; N]]) -> Mesh {
    let points: Vec<Vec3> = positions.iter().copied().map(game_to_view).collect();
    authored_mesh(&points, uvs, indices, packed, |n| game_to_view(n.to_array()))
}

fn needs_exact_world_mesh(instance: &Transform, positions: &[[f32; 3]], exact: &[Vec3]) -> bool {
    let approximate = instance.to_matrix();
    positions.iter().zip(exact).any(|(&local, &world)| {
        approximate
            .transform_point3(game_to_view(local))
            .distance(world)
            > 0.002
    })
}

struct MaterialResolver<'a> {
    catalog: Option<&'a archive::Catalog>,
    probe_materials: &'a mut Assets<ProbeMaterial>,
    decal_materials: &'a mut Assets<DecalMaterial>,
    probe_sh_buffers: &'a mut Assets<ShaderBuffer>,
    probe_sh_buffer: Handle<ShaderBuffer>,
    probe_sh_records: Vec<[Vec4; 9]>,
    probes: Option<&'a probe_lighting::ProbeLightingSet>,
    cache: HashMap<String, Option<CachedMaterial>>,
    diffuse_image_slots: HashMap<String, usize>,
    diffuse_image_paths: Vec<String>,
    preferred_mip: Option<u8>,
    misses: usize,
    suppressed_transparent: usize,
    cutout: usize,
    blended: usize,
    probe_samples: usize,
    probe_sample_misses: usize,
    lit: bool,
}

impl MaterialResolver<'_> {
    fn append_probe_record(&mut self, game_position: [f32; 3]) -> u32 {
        let sh = if self.lit {
            match self
                .probes
                .and_then(|probes| probes.interpolate_static_sh(game_position))
            {
                Some(lighting) => {
                    self.probe_samples += 1;
                    lighting.static_lighting_sh
                }
                None => {
                    if self.probes.is_some() {
                        self.probe_sample_misses += 1;
                    }
                    [[0.0; 3]; 9]
                }
            }
        } else {
            [[0.0; 3]; 9]
        };
        let record =
            sh.map(|coefficient| Vec4::new(coefficient[0], coefficient[1], coefficient[2], 0.0));
        let index = self.probe_sh_records.len() as u32;
        self.probe_sh_records.push(record);
        index
    }

    fn upload_probe_records(&mut self) {
        if let Some(mut buffer) = self.probe_sh_buffers.get_mut(&self.probe_sh_buffer) {
            buffer.set_data(std::mem::take(&mut self.probe_sh_records));
        }
    }

    fn register_image(&mut self, name: &str) -> Option<usize> {
        let catalog = self.catalog?;
        let mut stem = material::compiled_image_stem(catalog, name)?;
        stem.make_ascii_lowercase();
        if !catalog.contains(&stem)
            && !(0..=self.preferred_mip.unwrap_or(15)).any(|mip| catalog.contains(&format!("{stem}_mip{mip}")))
        { return None; }
        if let Some(&slot) = self.diffuse_image_slots.get(&stem) { return Some(slot); }
        let slot = self.diffuse_image_paths.len();
        self.diffuse_image_paths.push(stem.clone());
        self.diffuse_image_slots.insert(stem, slot);
        Some(slot)
    }

    fn get(&mut self, path: &str, _color_seed: usize, game_position: [f32; 3]) -> Option<ResolvedMaterial> {
        if let Some(cached) = self.cache.get(path) {
            let cached = cached.clone()?;
            let probe_index = self.append_probe_record(game_position);
            return Some(ResolvedMaterial { handle: cached.handle, diffuse_image_slot: cached.diffuse_image_slot, probe_index });
        }
        let key = path.to_owned();
        let bytes = self.catalog.and_then(|c| c.read(&format!("generated/decls/material/{path}.decl")).ok().flatten());
        let bytes = bytes.as_deref().unwrap_or(&[]);
        if material::is_screen_gate_impostor(path, bytes) { self.cache.insert(key, None); return None; }
        let surface = material_surface::Surface::parse(bytes);
        if surface.extra_layers > 2 || surface.layers.iter().any(|l| l.mask_channel > 3 || l.mask_mode > 2) {
            eprintln!("Unsupported material layer inputs {path}");
            self.cache.insert(key, None);
            return None;
        }
        let mut uniform = surface_gpu::SurfaceUniform::from(&surface);
        uniform.misc.z = u32::from(surface.main_program.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("arkDefault") || p.eq_ignore_ascii_case("arkDecal")));
        let program = surface.main_program.as_ref().and_then(|name| self.catalog.and_then(|c|
            c.read(&format!("generated/decls/renderprog/{}.decl", name.to_ascii_lowercase())).ok().flatten()));
        let program = program.as_deref().unwrap_or(&[]);
        let transparency = material::transparency_with_program(bytes, program);
        let program_is_decal = material::program_option(bytes, program, "isDecal");
        uniform.misc.w = u32::from(surface.alpha_test) | (u32::from(program_is_decal) << 1);
        let decal_bias = material::decal_depth_bias(bytes);
        let mut images = Vec::new();
        for index in 0..=surface.extra_layers {
            let layer = &surface.layers[index];
            let candidates = [
                (surface_gpu::ImageRole::diffuse(index), layer.diffuse.as_deref(), index == 0 || layer.reuse[0] == 0),
                (surface_gpu::ImageRole::bump(index), layer.bump.as_deref(), surface.has_bump && (index == 0 || layer.reuse[1] == 0)),
                (surface_gpu::ImageRole::gloss(index), layer.gloss.as_deref(), surface.has_gloss && !surface.gloss_in_alpha && (index == 0 || layer.reuse[2] == 0)),
                (surface_gpu::ImageRole::metallic(index), layer.metallic.as_deref(), surface.has_metallic && (index == 0 || layer.reuse[3] == 0)),
                (surface_gpu::ImageRole::mask(index), layer.mask.as_deref(), (index != 0 || layer.use_color) && layer.mask_mode != 0 && layer.reuse[4] == 0),
            ];
            for (role, name, enabled) in candidates {
                if !enabled { continue; }
                if let Some(name) = name {
                    if let Some(slot) = self.register_image(name) { images.push((slot, role)); }
                    else { eprintln!("Material image missing {path} role={} name={name}", role.0); }
                }
            }
        }
        // HQ/AQ AO samples the red channel at layer-zero UVs. LQ decals
        // return 1.0 and do not request this texture.
        if surface.has_occlusion && !program_is_decal {
            if let Some(name) = surface.occlusion.as_deref() {
                if let Some(slot) = self.register_image(name) { images.push((slot, surface_gpu::ImageRole::occlusion())); }
                else { eprintln!("Material image missing {path} role=15 name={name}"); }
            }
        }
        let diffuse_image_slot = images.iter().find(|(_, role)| role.0 == 0).map(|&(slot, _)| slot);
        if diffuse_image_slot.is_none() { self.misses += 1; }
        let alpha_mode = match transparency {
            material::Transparency::Opaque => AlphaMode::Opaque,
            material::Transparency::Cutout => { self.cutout += 1; AlphaMode::Mask(surface.alpha_cutoff) }
            material::Transparency::Coverage => {
                if surface.main_program.as_deref().is_some_and(|p| p.eq_ignore_ascii_case("arkDecal")) { self.blended += 1; AlphaMode::Blend }
                else { self.cutout += 1; AlphaMode::AlphaToCoverage }
            }
            material::Transparency::Blend => { self.blended += 1; AlphaMode::Blend }
        };
        let is_decal = program_is_decal || decal_bias.is_some();
        let base = StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: None,
            alpha_mode,
            depth_bias: 0.0,
            unlit: !self.lit || path.contains("/vista/") || path.contains("/skies/"),
            perceptual_roughness: 1.0 - surface.ward[1],
            metallic: surface.metallic_min_max[1],
            cull_mode: None,
            ..default()
        };
        let handle = if is_decal {
            MaterialHandle::Decal(self.decal_materials.add(DecalMaterial {
                base,
                extension: DecalExtension {
                    bias: decal_bias.unwrap_or(1678.0).round() as i32,
                    surface: uniform,
                    diffuse0: None,
                    diffuse1: None,
                    diffuse2: None,
                    bump0: None,
                    bump1: None,
                    bump2: None,
                    gloss0: None,
                    gloss1: None,
                    gloss2: None,
                    metallic0: None,
                    metallic1: None,
                    metallic2: None,
                    mask0: None,
                    mask1: None,
                    mask2: None,
                },
            }))
        } else {
            MaterialHandle::Probe(self.probe_materials.add(ProbeMaterial {
                base,
                extension: ProbeExtension {
                    sh: self.probe_sh_buffer.clone(),
                    surface: uniform,
                    diffuse0: None,
                    diffuse1: None,
                    diffuse2: None,
                    bump0: None,
                    bump1: None,
                    bump2: None,
                    gloss0: None,
                    gloss1: None,
                    gloss2: None,
                    metallic0: None,
                    metallic1: None,
                    metallic2: None,
                    mask0: None,
                    mask1: None,
                    mask2: None,
                },
            }))
        };
        let cached = CachedMaterial { handle, diffuse_image_slot, images };
        self.cache.insert(key, Some(cached.clone()));
        let probe_index = self.append_probe_record(game_position);
        Some(ResolvedMaterial { handle: cached.handle, diffuse_image_slot: cached.diffuse_image_slot, probe_index })
    }
}

fn setup(
    mut commands: Commands,
    mut scene: ResMut<SceneData>,
    navigation: Option<Res<NavigationWorld>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut probe_materials: ResMut<Assets<ProbeMaterial>>,
    mut probe_sh_buffers: ResMut<Assets<ShaderBuffer>>,
    mut decal_materials: ResMut<Assets<DecalMaterial>>,
    mut exposure_curves: ResMut<Assets<AutoExposureCompensationCurve>>,
) {
    // Temporarily take the catalog to avoid borrowing the scene while setup
    // mutates its parsed world. Restore it after setup for later streamed reads.
    let catalog_storage = scene.catalog.take();
    let catalog = catalog_storage.as_ref();
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                padding: UiRect::all(px(8)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            ZIndex(100),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Game XYZ: loading"),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                CoordinateOverlay,
            ));
        });
    let preferred_mip = env::var("DH2_TEXTURE_MIP")
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|&value| value <= 15);
    let lit = env::var("DH2_LIT").as_deref() != Ok("0");
    // Debug opt-in; each light still honors the authored m_NoShadows flag.
    let authored_shadows = env::var("DH2_SHADOWS").as_deref() == Ok("1");
    // Profiling switch: leave probe/environment lighting on while isolating scene lights.
    let scene_lights_enabled = env::var("DH2_SCENE_LIGHTS").as_deref() != Ok("0");
    let trace_startup = env::var("DH2_TRACE_STARTUP").as_deref() == Ok("1");
    if trace_startup {
        eprintln!(
            "Startup trace: beginning world setup ({} BWM objects, {} world meshes, {} placed models)",
            scene.world.objects.len(),
            scene.world.meshes.len(),
            scene.placed_models.len()
        );
    }
    let probe_sh_buffer = probe_sh_buffers.add(ShaderBuffer::from(vec![[Vec4::ZERO; 9]]));
    let mut resolver = MaterialResolver {
        catalog,
        probe_materials: &mut probe_materials,
        decal_materials: &mut decal_materials,
        probe_sh_buffers: &mut probe_sh_buffers,
        probe_sh_buffer,
        probe_sh_records: vec![[Vec4::ZERO; 9]],
        probes: scene.environment_probes.as_ref(),
        cache: HashMap::new(),
        diffuse_image_slots: HashMap::new(),
        diffuse_image_paths: Vec::new(),
        preferred_mip,
        misses: 0,
        suppressed_transparent: 0,
        cutout: 0,
        blended: 0,
        probe_samples: 0,
        probe_sample_misses: 0,
        lit,
    };
    let mut whole_world = Bounds::empty();
    let mut regions = BTreeMap::<i32, Bounds>::new();
    let mut world_mesh_handles = HashMap::<usize, Handle<Mesh>>::new();
    let mut baked_shear_instances = 0usize;
    let mut authored_lod_instances = BTreeMap::<usize, usize>::new();
    let mut render_instances = Vec::new();
    let mut bwm_visibility_render_instances = vec![Vec::new(); scene.world.visibility.len()];
    for (object_number, object) in scene.world.objects.iter().enumerate() {
        *authored_lod_instances.entry(object.lod).or_default() +=
            object.matrix_end - object.matrix_start;
        if trace_startup {
            eprintln!(
                "Startup trace: BWM object {}/{} mesh={} lod={} matrices={}..{} material={}",
                object_number + 1,
                scene.world.objects.len(),
                object.mesh_index,
                object.lod,
                object.matrix_start,
                object.matrix_end,
                object.material_path
            );
        }
        // Build every authored representation. FUN_140a82710 selects the
        // near/far distance tier before FUN_14037e1d0 marks its serialized
        // BWM group; a blanket LOD-0 filter here discards selected far meshes.
        if material::is_editor_helper(&object.material_path) {
            continue;
        }
        // This BWM surface is a boat volume proxy (stageSort sortboatvolume),
        // not the boat's visible hull. Its yellow debug material has no place
        // in the normal scene render.
        if object.material_path == "models/campaign/boat_shared/boat_gabarit_01.material" {
            continue;
        }
        let source = &scene.world.meshes[object.mesh_index];
        // layer0_vtxcolormaskchannel blends material layers, not final opacity.
        // Using it as vertex alpha made 1,059 authored world objects translucent.
        let mesh = world_mesh_handles
            .entry(object.mesh_index)
            .or_insert_with(|| {
                meshes.add(source_mesh(&source.positions, &source.uvs, &source.indices, &source.packed_channels))
            })
            .clone();
        for matrix_index in object.matrix_start..object.matrix_end {
            let matrix = &scene.world.matrices[matrix_index];
            let visibility_record = &scene.world.visibility[matrix_index];
            let points: Vec<Vec3> = source
                .positions
                .iter()
                .map(|&position| transformed(matrix, position))
                .collect();
            if trace_startup {
                eprintln!(
                    "Startup trace: resolving BWM material object={} matrix={} path={}",
                    object_number, matrix_index, object.material_path
                );
            }
            let Some(mat) = resolver.get(
                &object.material_path,
                object_number,
                mean_game_position(&points),
            ) else {
                continue;
            };
            for &point in &points {
                whole_world.add(point);
                regions
                    .entry((point.x / 35.0).floor() as i32)
                    .or_insert_with(Bounds::empty)
                    .add(point);
            }
            let instance = game_affine_to_view(matrix);
            let bounds = authored_world_bounds(visibility_record);
            let render_instance = if needs_exact_world_mesh(&instance, &source.positions, &points) {
                // Bevy's Transform cannot express shear. Preserve the exact
                // authored affine transform in this instance's vertex data.
                let baked = meshes.add(authored_mesh(&points, &source.uvs, &source.indices, &source.packed_channels, |v| transformed_direction(matrix, v)));
                let render_instance = RenderInstance {
                    mesh: baked,
                    material: mat.handle.clone(),
                    diffuse_image_slot: mat.diffuse_image_slot,
                    probe_index: mat.probe_index,
                    transform: Transform::IDENTITY,
                    bounds,
                    pickup_id: None,
                    entity: None,
                    visible: false,
                };
                baked_shear_instances += 1;
                render_instance
            } else {
                RenderInstance {
                    mesh: mesh.clone(),
                    material: mat.handle.clone(),
                    diffuse_image_slot: mat.diffuse_image_slot,
                    probe_index: mat.probe_index,
                    transform: instance,
                    bounds,
                    pickup_id: None,
                    entity: None,
                    visible: false,
                }
            };
            let render_index = render_instances.len();
            bwm_visibility_render_instances[matrix_index].push(render_index);
            render_instances.push(render_instance);
        }
    }
    if trace_startup {
        eprintln!(
            "Startup trace: BWM setup finished ({} render instances, {} diffuse paths discovered); beginning placed-model setup",
            render_instances.len(),
            resolver.diffuse_image_paths.len()
        );
    }
    let expanded_placements = catalog
        .as_ref()
        .map(|catalog| expanded_model_placements(&scene.placed_models, catalog))
        .unwrap_or_else(|| scene.placed_models.clone());
    let mut model_cache = HashMap::<String, Option<bmodel::Model>>::new();
    let mut md6_cache = HashMap::<String, Option<bmd6::ModelInfo>>::new();
    let mut md6_rest_pose_instances = 0usize;
    let mut model_mesh_handles = HashMap::<(String, usize), Handle<Mesh>>::new();
    let pickup_ids: HashSet<&str> = scene
        .pickups
        .iter()
        .map(|pickup| pickup.entity_id.as_str())
        .collect();
    let mut model_instances = 0usize;
    let mut model_surfaces = 0usize;
    let mut placed_model_queries = Vec::new();
    let mut placed_model_default_ranges = 0usize;
    for (number, placement) in expanded_placements.iter().enumerate() {
        if trace_startup {
            eprintln!(
                "Startup trace: placed model {}/{} id={} resource={}",
                number + 1,
                expanded_placements.len(),
                placement.id,
                placement.resource_path
            );
        }
        if placement.resource_path.ends_with(".blueprint.decl") {
            continue;
        }
        if placement.resource_path.ends_with(".bmd6model") {
            let model = md6_cache
                .entry(placement.resource_path.clone())
                .or_insert_with(|| {
                    let catalog = catalog.as_ref()?;
                    let mesh_path =
                        match bmd6::resolve_compiled_mesh(&placement.resource_path, |decl| {
                            catalog
                                .read(decl)
                                .ok()
                                .flatten()
                                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                        }) {
                            Ok(path) => path,
                            Err(error) => {
                                eprintln!(
                                    "MD6 declaration failed {}: {error}",
                                    placement.resource_path
                                );
                                return None;
                            }
                        };
                    let bytes = catalog.read(&mesh_path).ok()??;
                    match bmd6::inspect(&bytes) {
                        Ok(model) if model.has_decoded_geometry => Some(model),
                        Ok(_) => None,
                        Err(error) => {
                            eprintln!("MD6 parse failed {}: {error}", placement.resource_path);
                            None
                        }
                    }
                });
            if let Some(model) = model {
                model_instances += 1;
                if model.has_skeleton {
                    md6_rest_pose_instances += 1;
                }
                for (surface_number, surface) in model.surfaces.iter().enumerate() {
                    let material_path = placement
                        .custom_materials
                        .get(surface_number)
                        .and_then(Option::as_deref)
                        .unwrap_or(&surface.material_path);
                    if material::is_editor_helper(material_path) {
                        continue;
                    }
                    let points: Vec<Vec3> = surface
                        .positions
                        .iter()
                        .map(|&vertex| placed_vertex(placement, vertex))
                        .collect();
                    if points.is_empty() || surface.indices.is_empty() {
                        continue;
                    }
                    let Some(mat) = resolver.get(
                        material_path,
                        number + scene.world.objects.len(),
                        mean_game_position(&points),
                    ) else {
                        continue;
                    };
                    let bounds = render_bounds(&points);
                    let instance = placed_transform(placement);
                    let (mesh, transform) =
                        if needs_exact_world_mesh(&instance, &surface.positions, &points) {
                            (
                                meshes.add(authored_mesh(
                                    &points,
                                    &surface.uvs,
                                    &surface.indices,
                                    &surface.packed_channels,
                                    |v| placed_direction(placement, v),
                                )),
                                Transform::IDENTITY,
                            )
                        } else {
                            (
                                model_mesh_handles
                                    .entry((placement.resource_path.clone(), surface_number))
                                    .or_insert_with(|| {
                                        meshes.add(source_mesh(
                                            &surface.positions,
                                            &surface.uvs,
                                            &surface.indices,
                                            &surface.packed_channels,
                                        ))
                                    })
                                    .clone(),
                                instance,
                            )
                        };
                    render_instances.push(RenderInstance {
                        mesh,
                        material: mat.handle,
                        diffuse_image_slot: mat.diffuse_image_slot,
                        probe_index: mat.probe_index,
                        transform,
                        bounds,
                        pickup_id: None,
                        entity: None,
                        visible: false,
                    });
                    model_surfaces += 1;
                }
            }
            continue;
        }
        // Authoring helpers carry debug colours but are not visible objects.
        if placement.resource_path.contains("/collisions/")
            || placement.resource_path.contains("/templates/occlusion/")
            || placement.resource_path.contains("/small_boat_gabarit_")
            || placement.resource_path.contains("/skiff_01_noswim")
        {
            continue;
        }
        let model = model_cache
            .entry(placement.resource_path.clone())
            .or_insert_with(|| {
                let catalog = catalog.as_ref()?;
                let bytes = match catalog.read(&placement.resource_path) {
                    Ok(Some(bytes)) => bytes,
                    Ok(None) => {
                        eprintln!("Model not indexed: {}", placement.resource_path);
                        return None;
                    }
                    Err(error) => {
                        eprintln!("Model read failed {}: {error}", placement.resource_path);
                        return None;
                    }
                };
                match bmodel::parse(&bytes) {
                    Ok(model) => Some(model),
                    Err(error) => {
                        eprintln!("Model parse failed {}: {error}", placement.resource_path);
                        None
                    }
                }
            });
        let Some(model) = model else {
            continue;
        };
        model_instances += 1;
        let query_input = initial_placed_model_query(placement, model.authored_bounds);
        placed_model_default_ranges += usize::from(placement.max_visible_range.is_none());
        let model_render_start = render_instances.len();
        for (surface_number, surface) in model.surfaces.iter().enumerate() {
            let material_path = placement
                .custom_materials
                .get(surface_number)
                .and_then(Option::as_deref)
                .unwrap_or(&surface.material_path);
            if material::is_editor_helper(material_path) {
                continue;
            }
            let points: Vec<Vec3> = surface
                .positions
                .iter()
                .map(|&vertex| placed_vertex(placement, vertex))
                .collect();
            for &point in &points {
                whole_world.add(point);
                regions
                    .entry((point.x / 35.0).floor() as i32)
                    .or_insert_with(Bounds::empty)
                    .add(point);
            }
            let Some(mat) = resolver.get(
                material_path,
                number + scene.world.objects.len(),
                mean_game_position(&points),
            ) else {
                continue;
            };
            let bounds = render_bounds(&points);
            let instance = placed_transform(placement);
            let (mesh, transform) =
                if needs_exact_world_mesh(&instance, &surface.positions, &points) {
                    baked_shear_instances += 1;
                    (
                        meshes.add(authored_mesh(&points, &surface.uvs, &surface.indices, &surface.packed_channels, |v| placed_direction(placement, v))),
                        Transform::IDENTITY,
                    )
                } else {
                    (
                        model_mesh_handles
                            .entry((placement.resource_path.clone(), surface_number))
                            .or_insert_with(|| {
                                meshes.add(source_mesh(
                                    &surface.positions,
                                    &surface.uvs,
                                    &surface.indices,
                                    &surface.packed_channels,
                                ))
                            })
                            .clone(),
                        instance,
                    )
                };
            render_instances.push(RenderInstance {
                mesh,
                material: mat.handle,
                diffuse_image_slot: mat.diffuse_image_slot,
                probe_index: mat.probe_index,
                transform,
                bounds,
                pickup_id: pickup_ids
                    .contains(placement.id.as_str())
                    .then(|| placement.id.clone()),
                entity: None,
                visible: false,
            });
            model_surfaces += 1;
        }
        if let Some(input) = query_input {
            if model_render_start < render_instances.len() {
                placed_model_queries.push(PlacedModelQueryGroup {
                    indices: model_render_start..render_instances.len(),
                    input,
                });
            }
        }
    }
    println!(
        "Original initial BML query groups: {}; constructor-default range models: {}; blueprint/MD6 and surface bit40 producers remain unsupported",
        placed_model_queries.len(),
        placed_model_default_ranges
    );
    if trace_startup {
        eprintln!(
            "Startup trace: placed-model setup finished ({} render instances, {} diffuse paths discovered); beginning scene finalization",
            render_instances.len(),
            resolver.diffuse_image_paths.len()
        );
    }
    let probe_sh_record_count = resolver.probe_sh_records.len().saturating_sub(1);
    resolver.upload_probe_records();
    println!(
        "Shared geometry: {} world meshes and {} placed-model surfaces; {} world instances baked for shear",
        world_mesh_handles.len(),
        model_mesh_handles.len(),
        baked_shear_instances
    );
    println!("BWM authored instance counts by LOD tag: {authored_lod_instances:?}");
    println!(
        "Placed models: {model_instances}/{} instances, {} unique resources, {model_surfaces} surfaces",
        scene.placed_models.len(),
        model_cache.len()
    );
    println!(
        "MD6 skeletal instances shown in compiled rest pose: {md6_rest_pose_instances}; joint animation is not implemented"
    );
    println!(
        "Materials: {} unique, {} diffuse textures deferred, {} without diffuse ({} transparent surfaces suppressed); initial mip {}",
        resolver.cache.len(),
        resolver.diffuse_image_paths.len(),
        resolver.misses,
        resolver.suppressed_transparent,
        preferred_mip.map_or_else(
            || "authored inline base".to_owned(),
            |mip| format!("separate <= {mip} override")
        )
    );
    println!(
        "Static probe SH storage: {probe_sh_record_count} per-instance records, {} interpolated, {} outside the PNW tetrahedra",
        resolver.probe_samples, resolver.probe_sample_misses
    );
    println!(
        "Transparency: {} cutout materials, {} blended materials",
        resolver.cutout, resolver.blended
    );
    let mut targets = vec![whole_world.focus("whole world".to_string())];
    let mut best = (0usize, 0usize);
    for (region, bounds) in regions {
        if bounds.points < 200 {
            continue;
        }
        let index = targets.len();
        if bounds.points > best.1 {
            best = (index, bounds.points);
        }
        targets.push(bounds.focus(format!("region {region}")));
    }
    let active = if let Some(start) = scene.player_start {
        targets.insert(
            0,
            FocusTarget {
                center: game_to_view(start.position),
                distance: 10.0,
                label: "player start".to_string(),
                spawn: Some(start),
            },
        );
        0
    } else {
        best.0
    };
    let mut camera = view_of(&targets[active]);
    let walking = navigation.is_some() && scene.player_start.is_some();
    let feet = camera.translation - Vec3::Y * 1.7;
    let walker = navigation
        .as_ref()
        .map(|world| navigation::Walker::new(feet, &world.0))
        .unwrap_or(navigation::Walker {
            feet,
            vertical_speed: 0.0,
            supported: false,
        });
    if walking {
        camera.translation = walker.feet + Vec3::Y * 1.7;
    }
    if let Some(probes) = &scene.environment_probes {
        let camera_game = view_to_game(camera.translation);
        if let Some(interpolated) = probes.interpolate_static_sh(camera_game) {
            println!(
                "Authored probe data at camera: tetrahedron {}, weights {:?}, static-light illuminance {:?}, sky/sun factors {}/{}, estimated illuminance {:?}, static/sun/sky-input SH L0 {:?}/{:?}/{:?}; material samples use each surface center",
                interpolated.tetrahedron_index,
                interpolated.probe_weights,
                interpolated.static_lights_illuminance,
                interpolated.sky_occlusion_factor,
                interpolated.sun_irradiance_factor,
                interpolated.illuminance_estimate,
                interpolated.static_lighting_sh[0],
                interpolated.sun_factor_sh[0],
                interpolated.sky_occlusion_sh[0],
            );
        } else {
            println!("No authored probe tetrahedron contains camera position {camera_game:?}");
        }
        if let Some((distance, probe)) = probes
            .probes
            .iter()
            .map(|probe| {
                let distance = (0..3)
                    .map(|axis| (probe.position[axis] - camera_game[axis]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                (distance, probe)
            })
            .min_by(|left, right| left.0.total_cmp(&right.0))
        {
            println!(
                "Nearest authored probe: {} at {:?}, {:.2} m; individual static SH L0 {:?}; secondary network entries {}",
                probe.resource,
                probe.position,
                distance,
                probe.static_lighting_sh[0],
                probes.secondary_resources.len(),
            );
        }
    }
    // These are the installed executable's registered perspective defaults:
    // ark_fieldOfView=80 degrees at its 16:9 reference, r_znear=0.01, and
    // r_zfar=7000. The vertical FOV is derived by FUN_140890b50; Bevy updates
    // the projection aspect when the actual camera viewport changes.
    let game_default_vertical_fov =
        tome::original_render_vertical_fov_radians(ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES)
            .expect("the registered game FOV default is valid");
    let mut camera_entity = commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: game_default_vertical_fov,
            near: ORIGINAL_DEFAULT_NEAR,
            far: ORIGINAL_DEFAULT_FAR,
            ..default()
        }),
        camera,
        Msaa::Sample4,
    ));
    if lit {
        if let Some(environment) = scene.environment_lighting {
            if environment.auto_exposure_ev.is_some()
                || environment.auto_exposure_adapt_speed_up.is_some()
                || environment.auto_exposure_adapt_speed_down.is_some()
            {
                let compensation = environment.auto_exposure_ev.unwrap_or(0.0);
                let curve = exposure_curves.add(
                    AutoExposureCompensationCurve::from_curve(LinearSpline::new([
                        Vec2::new(-16.0, compensation),
                        Vec2::new(16.0, compensation),
                    ]))
                    .expect("constant environment exposure compensation curve is valid"),
                );
                let defaults = AutoExposure::default();
                camera_entity.insert(AutoExposure {
                    // Raising the EV darkens the image; lowering it brightens it.
                    speed_brighten: environment
                        .auto_exposure_adapt_speed_down
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .unwrap_or(defaults.speed_brighten),
                    speed_darken: environment
                        .auto_exposure_adapt_speed_up
                        .filter(|value| value.is_finite() && *value >= 0.0)
                        .unwrap_or(defaults.speed_darken),
                    compensation_curve: curve,
                    ..default()
                });
            }
        }
    }
    commands.insert_resource(NavigationState { walking, walker });
    commands.insert_resource(PickupState {
        items: scene
            .pickups
            .iter()
            .map(|pickup| PickupInstance {
                entity_id: pickup.entity_id.clone(),
                position: game_to_view(pickup.position),
            })
            .collect(),
        world: runtime::WorldState::from_pickups(&scene.pickups)
            .expect("level contains invalid or duplicate pickup definitions"),
    });
    if lit {
        let ambient = scene.environment_lighting.and_then(|environment| {
            let high = environment
                .ambient_high
                .or_else(|| environment.ambient_high_scalar.map(|value| [value; 3]))?;
            let low = environment
                .ambient_low
                .or_else(|| environment.ambient_low_scalar.map(|value| [value; 3]))?;
            Some((high, low))
        });
        if let Some((high, low)) = ambient {
            commands.insert_resource(GlobalAmbientLight {
                // Void render parameters are authored as linear float values;
                // interpreting them as sRGB would gamma-decode and dim them.
                color: Color::linear_rgb(
                    (high[0] + low[0]) * 0.5,
                    (high[1] + low[1]) * 0.5,
                    (high[2] + low[2]) * 0.5,
                ),
                ..default()
            });
        } else {
            commands.insert_resource(GlobalAmbientLight::NONE);
        }
        let mut spawned = 0;
        let mut unsupported = 0;
        let mut energy_shortened_ranges = 0;
        if scene_lights_enabled {
            for light in &scene.placed_lights {
                if !light.visible {
                    continue;
                }
                let position = game_to_view(light.position);
                // FUN_140881640 applies FUN_1401644a0 to idLight.m_Color before
                // preset conversion. Its installed constants (0.04045, 0.055,
                // 1.055, 2.4, 12.92) exactly match Bevy's sRGB decode.
                let color = Color::srgb(
                    light.color[0].max(0.0),
                    light.color[1].max(0.0),
                    light.color[2].max(0.0),
                );
                let linear_color = color.to_linear();
                let Some(preset) = original_light_preset(
                    light,
                    [linear_color.red, linear_color.green, linear_color.blue],
                ) else {
                    unsupported += 1;
                    continue;
                };
                if !preset.normalized_intensity.is_finite()
                    || !preset.color.iter().all(|value| value.is_finite())
                    || !preset.range.is_finite()
                    || !preset.falloff.is_finite()
                {
                    unsupported += 1;
                    continue;
                }
                let range = preset.range;
                energy_shortened_ranges += usize::from(
                    original_light_distance(light.distance_cutoff, light.distance_falloff)
                        .is_some_and(|(cutoff, _)| range < cutoff),
                );
                let falloff_start = preset.falloff;
                let is_sun = light.preset.as_deref() == Some("LIGHT_PRESET_SUN");
                // Native normalized RGB is clamped before scaling. Retain the
                // existing 10,000-lumen/100W backend calibration (native 100W
                // intensity is 5); absolute Bevy photometry remains unverified.
                let color = Color::linear_rgb(
                    linear_color.red.clamp(0.0, 1.0),
                    linear_color.green.clamp(0.0, 1.0),
                    linear_color.blue.clamp(0.0, 1.0),
                );
                let rect_size = light.area_size.map(|size| size.max(0.01));
                let Some(intensity) =
                    light_backend_intensity(preset.normalized_intensity, light.kind, rect_size)
                else {
                    unsupported += 1;
                    continue;
                };
                let spot_cosines = (light.kind == entities::LightType::Spot)
                    .then(|| authored_light_cosines(light.angular_cutoff, light.angular_falloff));
                let cull = (light.kind != entities::LightType::Parallel).then(|| {
                    let clip_to_world = original_local_light_clip_to_world(
                        position,
                        light.rotation,
                        light.kind,
                        range,
                        spot_cosines.map(|(cutoff, _)| cutoff),
                    );
                    let projection_bounds =
                        clip_to_world.and_then(original_light_projection_bounds);
                    OriginalLocalLightCull {
                        range,
                        kind: light.kind,
                        culling: light.culling,
                        definition_name_hash: light.definition_name_hash,
                        linear_color: preset.color,
                        // +0x28 actor origin plus +0x4c preset offset. Area
                        // proxies share this source, not their Bevy positions.
                        initial_sphere: Some([
                            light.position[0] + preset.center_offset[0],
                            light.position[1] + preset.center_offset[1],
                            light.position[2] + preset.center_offset[2],
                            range,
                        ]),
                        projection_bounds,
                        proximity_bounds: original_light_proximity_from_cached_bounds(
                            light.max_visible_range,
                            projection_bounds,
                        ),
                        clip_to_world,
                    }
                });
                match light.kind {
                    entities::LightType::Point => {
                        commands.spawn((
                            PointLight {
                                color,
                                intensity,
                                decay: light.decay,
                                falloff_start,
                                range,
                                shadow_maps_enabled: authored_shadows && !light.no_shadows,
                                shadow_map_near_z: light
                                    .shadow_near_plane
                                    .unwrap_or(PointLight::DEFAULT_SHADOW_MAP_NEAR_Z),
                                ..default()
                            },
                            Transform::from_translation(position),
                            cull.expect("local light cull data"),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::HalfPoint => {
                        // The original renderer keeps these omnidirectional for
                        // shadowing, then applies a one-sided angular ramp in the
                        // shared point-light shader. Its signed reciprocal is
                        // carried through the vendored Bevy point-light shader.
                        let direction = game_to_view(light.rotation[0]).normalize_or_zero();
                        if direction == Vec3::ZERO {
                            unsupported += 1;
                            continue;
                        }
                        let angular_scale = preserved_half_point_angular_scale(
                            light.angular_cutoff,
                            light.angular_falloff,
                        );
                        commands.spawn((
                            PointLight {
                                color,
                                intensity,
                                decay: light.decay,
                                falloff_start,
                                range,
                                half_point_data: direction.extend(angular_scale),
                                shadow_maps_enabled: authored_shadows && !light.no_shadows,
                                shadow_map_near_z: light
                                    .shadow_near_plane
                                    .unwrap_or(PointLight::DEFAULT_SHADOW_MAP_NEAR_Z),
                                ..default()
                            },
                            Transform::from_translation(position),
                            cull.expect("local light cull data"),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Spot => {
                        let direction = game_to_view(light.rotation[0]).normalize_or_zero();
                        if direction == Vec3::ZERO {
                            unsupported += 1;
                            continue;
                        }
                        let up = game_to_view(light.rotation[1]).normalize_or_zero();
                        if up == Vec3::ZERO || up.cross(direction).length_squared() < 0.0001 {
                            unsupported += 1;
                            continue;
                        }
                        let (cutoff_cosine, falloff_cosine) =
                            spot_cosines.expect("spot light cosine thresholds");
                        // The game uses m_AngularCutoff for the projection/outer cone
                        // and m_AngularFalloff for its inner transition boundary.
                        let outer_angle = cutoff_cosine.acos();
                        let inner_angle = falloff_cosine.acos();
                        // Bevy's spot-light implementation requires a sub-90°
                        // outer cone; the authored mission cutoffs are within
                        // that range after the game's half-angle conversion.
                        let outer_angle =
                            outer_angle.clamp(0.01, std::f32::consts::FRAC_PI_2 - 0.01);
                        let inner_angle = inner_angle.min(outer_angle);
                        commands.spawn((
                            SpotLight {
                                color,
                                intensity,
                                decay: light.decay,
                                falloff_start,
                                range,
                                inner_angle,
                                outer_angle,
                                shadow_maps_enabled: authored_shadows && !light.no_shadows,
                                shadow_map_near_z: light
                                    .shadow_near_plane
                                    .unwrap_or(SpotLight::DEFAULT_SHADOW_MAP_NEAR_Z),
                                ..default()
                            },
                            Transform::from_translation(position).looking_to(direction, up),
                            cull.expect("local light cull data"),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Area => {
                        // LIGHT_AREA uses the separate paraboloid path in
                        // renderprog/lighting.decl; Bevy RectLight has no matching
                        // shadow path. The authored area profile/projection and
                        // area-specific shadow controls remain unported.
                        let direction = game_to_view(light.rotation[0]).normalize_or_zero();
                        let up = game_to_view(light.rotation[1]).normalize_or_zero();
                        if direction == Vec3::ZERO
                            || up == Vec3::ZERO
                            || up.cross(direction).length_squared() < 0.0001
                        {
                            unsupported += 1;
                            continue;
                        }
                        commands.spawn((
                            RectLight {
                                color,
                                intensity,
                                range,
                                width: rect_size[0],
                                height: rect_size[1],
                            },
                            Transform::from_translation(position).looking_to(direction, up),
                            cull.expect("local light cull data"),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Parallel => {
                        let sun_color = if light.preset.as_deref() == Some("LIGHT_PRESET_SUN") {
                            scene
                                .environment_lighting
                                .and_then(|environment| environment.sun_color)
                                .map(|rgb| Color::linear_rgb(rgb[0], rgb[1], rgb[2]))
                                .unwrap_or(color)
                        } else {
                            color
                        };
                        let authored_direction =
                            game_to_view(light.rotation[0]).normalize_or_zero();
                        let direction = if light.preset.as_deref() == Some("LIGHT_PRESET_SUN") {
                            let sun_axis = light.rotation[0];
                            let authored_sun_azimuth = (light.orientation_authored
                                && sun_axis[0] * sun_axis[0] + sun_axis[1] * sun_axis[1] > 0.0001)
                                .then(|| {
                                    (-sun_axis[1])
                                        .atan2(sun_axis[0])
                                        .to_degrees()
                                        .rem_euclid(360.0)
                                });
                            scene
                                .environment_lighting
                                .and_then(|environment| {
                                    environment_sun_ray(environment, authored_sun_azimuth)
                                })
                                .filter(|direction| *direction != Vec3::ZERO)
                                .unwrap_or(authored_direction)
                        } else {
                            authored_direction
                        };
                        let up = game_to_view(light.rotation[1]).normalize_or_zero();
                        if direction == Vec3::ZERO
                            || up == Vec3::ZERO
                            || up.cross(direction).length_squared() < 0.0001
                        {
                            unsupported += 1;
                            continue;
                        }
                        let mut cascade_shadow_config = CascadeShadowConfigBuilder::default();
                        if let Some(maximum_distance) = scene
                            .environment_lighting
                            .and_then(|environment| environment.parallel_light_extend)
                            .filter(|distance| {
                                distance.is_finite()
                                    && *distance > cascade_shadow_config.first_cascade_far_bound
                            })
                        {
                            // Mapping env/shadow/parallellightextend to Bevy's
                            // directional cascade extent is inferred from the
                            // authored field name and Bevy's distance control.
                            cascade_shadow_config.maximum_distance = maximum_distance;
                        }
                        commands.spawn((
                            DirectionalLight {
                                color: sun_color,
                                // Runtime normalization is now recovered. The
                                // absolute directional Bevy-lux conversion is
                                // still unverified; preserve its existing SUN
                                // calibration, not an inferred native unit.
                                illuminance: if is_sun { 8_000.0 } else { intensity },
                                shadow_maps_enabled: authored_shadows && !light.no_shadows,
                                ..default()
                            },
                            Transform::from_translation(position).looking_to(direction, up),
                            cascade_shadow_config.build(),
                        ));
                        spawned += 1;
                    }
                }
            }
        }
        println!(
            "Resolved authored lights spawned: {spawned}; unsupported shapes, presets, or invalid ranges: {unsupported}; native energy-shortened ranges: {energy_shortened_ranges}; scene lights {}; shadow maps {} (DH2_SHADOWS=1 enables)",
            if scene_lights_enabled {
                "enabled"
            } else {
                "disabled"
            },
            if authored_shadows {
                "enabled"
            } else {
                "disabled"
            }
        );
    }
    println!(
        "World bounds {:?} .. {:?}; {} camera regions; starting at {}",
        whole_world.low,
        whole_world.high,
        targets.len() - 1,
        targets[active].label
    );
    println!(
        "Controls: click to capture mouse, mouse/arrows look, Esc release; G toggles walk/free-fly; WASD move, Shift sprint, Space jump in walk; F pick up mapped items; F8 save collision report and screenshot; Q/E vertical in free-fly; PageUp/PageDown region, Home reset, End overview"
    );
    println!(
        "Mapped pickups with explicit item and quantity: {}",
        scene.pickups.len()
    );
    commands.insert_resource(FocusTargets { targets, active });
    let tome_render_instances =
        TomeRenderInstanceMap::from_world_mapping(&scene.world, &bwm_visibility_render_instances);
    let visibility_suppression_groups = scene
        .world
        .visibility_groups
        .iter()
        .map(|group| RenderSuppressionGroup {
            parent: bwm_visibility_render_instances[group.parent].clone(),
            children: group
                .children
                .iter()
                .flat_map(|&index| bwm_visibility_render_instances[index].iter().copied())
                .collect(),
        })
        .collect();
    let mut diffuse_image_materials = vec![Vec::new(); resolver.diffuse_image_paths.len()];
    let mut native_material_images = HashMap::new();
    for cached in resolver.cache.values().flatten() {
        native_material_images.insert(cached.handle.clone(), cached.images.iter().map(|&(slot, _)| slot).collect());
        for &(slot, role) in &cached.images {
            diffuse_image_materials[slot].push((cached.handle.clone(), role));
        }
    }
    let diffuse_image_count = resolver.diffuse_image_paths.len();
    commands.insert_resource(PlacedModelQueries(placed_model_queries));
    commands.insert_resource(SceneRenderQueue {
        instances: render_instances,
        active: 0,
        tome_render_instances,
        visible_authored_ids: Vec::new(),
        visible_cell_indices: Vec::new(),
        visible_cell_mask: Vec::new(),
        light_query_state: None,
        visibility_suppression_groups,
        diffuse_image_demands: (0..diffuse_image_count)
            .map(|_| texture::ProjectedAreaDemand::default())
            .collect(),
        diffuse_image_paths: std::mem::take(&mut resolver.diffuse_image_paths),
        diffuse_image_materials,
        native_material_images,
        diffuse_image_handles: vec![None; diffuse_image_count],
        diffuse_image_pending: vec![None; diffuse_image_count],
        diffuse_mip_states: (0..diffuse_image_count)
            .map(|_| DiffuseMipState::default())
            .collect(),
        diffuse_image_attempted: vec![false; diffuse_image_count],
        diffuse_image_last_visible: vec![0; diffuse_image_count],
        texture_stream_frame: 0,
        texture_viewport_pixels: 0,
        diffuse_mip_upgrades: 0,
        preferred_mip,
    });
    if let Some(catalog) = catalog_storage {
        commands.insert_resource(TextureReadWorker::new(catalog, diffuse_image_count));
    }
}

fn camera_controls(
    time: Res<Time>,
    benchmark: Option<Res<BenchmarkSeconds>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut focuses: ResMut<FocusTargets>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut cursor: Single<&mut CursorOptions>,
    mut navigation: ResMut<NavigationState>,
    collision: Option<Res<NavigationWorld>>,
) {
    if benchmark.is_some() {
        return;
    }
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    if mouse.just_pressed(MouseButton::Left) {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
    if keys.just_pressed(KeyCode::Home) {
        focuses.active = 0;
    }
    if keys.just_pressed(KeyCode::End) {
        focuses.active = if focuses.targets[0].spawn.is_some() {
            1
        } else {
            0
        };
    }
    if keys.just_pressed(KeyCode::PageDown) {
        focuses.active = (focuses.active + 1) % focuses.targets.len();
    }
    if keys.just_pressed(KeyCode::PageUp) {
        focuses.active = (focuses.active + focuses.targets.len() - 1) % focuses.targets.len();
    }
    if keys.any_just_pressed([
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageDown,
        KeyCode::PageUp,
    ]) {
        let target = &focuses.targets[focuses.active];
        *camera = view_of(target);
        navigation.walker = collision
            .as_ref()
            .map(|world| navigation::Walker::new(camera.translation - Vec3::Y * 1.7, &world.0))
            .unwrap_or(navigation::Walker {
                feet: camera.translation - Vec3::Y * 1.7,
                vertical_speed: 0.0,
                supported: false,
            });
        if target.spawn.is_none() {
            navigation.walking = false;
        }
        println!("Viewing {} at {:?}", target.label, target.center);
    }
    if keys.just_pressed(KeyCode::KeyG) && collision.is_some() {
        navigation.walking = !navigation.walking;
        navigation.walker = navigation::Walker::new(
            camera.translation - Vec3::Y * 1.7,
            &collision.as_ref().unwrap().0,
        );
        println!(
            "Navigation mode: {}",
            if navigation.walking {
                "grounded walk"
            } else {
                "free-fly"
            }
        );
    }
    let dt = time.delta_secs();
    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        if navigation.walking { 10.0 } else { 30.0 }
    } else {
        6.0
    };
    let forward = camera.forward().as_vec3();
    let right = camera.right().as_vec3();
    let mut movement = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        movement += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        movement -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        movement += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        movement -= right;
    }
    if keys.pressed(KeyCode::KeyE) {
        movement += Vec3::Y;
    }
    if keys.pressed(KeyCode::KeyQ) {
        movement -= Vec3::Y;
    }
    if navigation.walking {
        if let Some(world) = collision.as_ref() {
            navigation.walker.step(
                &world.0,
                movement,
                speed,
                dt,
                keys.just_pressed(KeyCode::Space),
            );
            camera.translation = navigation.walker.feet + Vec3::Y * 1.7;
        }
    } else {
        camera.translation += movement.normalize_or_zero() * speed * dt;
    }
    let mut look = Vec2::ZERO;
    if cursor.grab_mode == CursorGrabMode::Locked {
        look += motion.delta * 0.0025;
    }
    if keys.pressed(KeyCode::ArrowLeft) {
        look.x -= 0.9 * dt;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        look.x += 0.9 * dt;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        look.y -= 0.9 * dt;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        look.y += 0.9 * dt;
    }
    if look != Vec2::ZERO {
        let (yaw, pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
        camera.rotation = Quat::from_euler(
            EulerRot::YXZ,
            yaw - look.x,
            (pitch - look.y).clamp(-1.54, 1.54),
            0.0,
        );
    }
}

fn update_coordinate_overlay(
    cameras: Query<&Transform, With<Camera3d>>,
    navigation: Res<NavigationState>,
    mut overlays: Query<&mut Text, With<CoordinateOverlay>>,
) {
    let (Ok(camera), Ok(mut overlay)) = (cameras.single(), overlays.single_mut()) else {
        return;
    };
    let [x, y, z] = view_to_game(camera.translation);
    let [fx, fy, fz] = view_to_game(navigation.walker.feet);
    **overlay = if navigation.walking {
        format!(
            "Game XYZ eye: {x:.2}, {y:.2}, {z:.2} | feet: {fx:.2}, {fy:.2}, {fz:.2} | walking | F8 report"
        )
    } else {
        format!("Game XYZ camera: {x:.2}, {y:.2}, {z:.2} | free-fly | F8 report")
    };
}

fn pickup_in_view(
    camera: &Transform,
    state: &PickupState,
    collision: Option<&navigation::CollisionWorld>,
) -> Option<usize> {
    let forward = camera.forward().as_vec3();
    state
        .items
        .iter()
        .enumerate()
        .filter(|(_, item)| state.world.pickup_available(&item.entity_id))
        .filter_map(|(index, item)| {
            let direction = item.position - camera.translation;
            let distance = direction.length();
            if distance > 2.5 || distance < 0.05 || forward.dot(direction / distance) < 0.8 {
                return None;
            }
            // Leave a short margin for the pickup's own physics surface.
            if collision
                .and_then(|world| world.first_hit(camera.translation, item.position))
                .is_some_and(|hit| hit < distance - 0.25)
            {
                return None;
            }
            Some((index, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(index, _)| index)
}

fn interact_pickup(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    cameras: Query<&Transform, With<Camera3d>>,
    mut state: ResMut<PickupState>,
    mut scene: ResMut<SceneRenderQueue>,
    mut collision: Option<ResMut<NavigationWorld>>,
    visuals: Query<(Entity, &PickupVisual)>,
    mut windows: Query<&mut Window>,
) {
    if !keys.just_pressed(KeyCode::KeyF) {
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let Some(index) = pickup_in_view(camera, &state, collision.as_ref().map(|world| &world.0))
    else {
        return;
    };
    let id = state.items[index].entity_id.clone();
    let Ok(runtime::WorldEvent::PickupCollected {
        item_path: path,
        quantity,
        inventory_total,
        ..
    }) = state.world.collect_pickup(&id)
    else {
        return;
    };
    if let Some(world) = collision.as_mut() {
        world.0.set_owner_active(&id, false);
    }
    for (entity, visual) in visuals.iter() {
        if visual.0 == id {
            commands.entity(entity).despawn();
        }
    }
    let mut removed = 0;
    for instance in &mut scene.instances {
        if instance.pickup_id.as_deref() == Some(id.as_str()) && instance.entity.take().is_some() {
            if instance.visible {
                removed += 1;
            }
            instance.visible = false;
        }
    }
    scene.active = scene.active.saturating_sub(removed);
    if let Ok(mut window) = windows.single_mut() {
        window.title = format!("Dishonored 2 Rust | picked up {quantity} × {path}");
    }
    println!("Picked up {id}: {quantity} × {path}; inventory total {inventory_total}");
}

fn snapshot_once(
    mut commands: Commands,
    request: Option<Res<SnapshotPath>>,
    time: Res<Time>,
    mut captured: Local<bool>,
) {
    let Some(request) = request else {
        return;
    };
    if !*captured && time.elapsed_secs() > 6.0 {
        *captured = true;
        let destination = request.0.clone();
        println!("Capturing screenshot to {}", destination.display());
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(destination));
    }
}

fn ray_triangle(origin: Vec3, direction: Vec3, triangle: [Vec3; 3]) -> Option<f32> {
    let [a, b, c] = triangle;
    let edge1 = b - a;
    let edge2 = c - a;
    let cross = direction.cross(edge2);
    let determinant = edge1.dot(cross);
    if determinant.abs() < 1e-7 {
        return None;
    }
    let inverse = 1.0 / determinant;
    let from_a = origin - a;
    let u = from_a.dot(cross) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let v = direction.dot(from_a.cross(edge1)) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge2.dot(from_a.cross(edge1)) * inverse;
    (distance > 0.001).then_some(distance)
}

fn sightline_world_surface(
    world: &world_mesh::WorldMesh,
    origin: Vec3,
    direction: Vec3,
    limit: f32,
) -> Option<(f32, usize, usize, String)> {
    let mut nearest: Option<(f32, usize, usize, String)> = None;
    for object in world.objects.iter().filter(|object| object.lod == 0) {
        let mesh = &world.meshes[object.mesh_index];
        for matrix_index in object.matrix_start..object.matrix_end {
            let matrix = &world.matrices[matrix_index];
            for face in mesh.indices.chunks_exact(3) {
                let triangle = [face[0], face[1], face[2]]
                    .map(|index| transformed(matrix, mesh.positions[index as usize]));
                let Some(distance) = ray_triangle(origin, direction, triangle) else {
                    continue;
                };
                if distance > limit || nearest.as_ref().is_some_and(|hit| distance >= hit.0) {
                    continue;
                }
                nearest = Some((
                    distance,
                    object.mesh_index,
                    matrix_index,
                    object.material_path.clone(),
                ));
            }
        }
    }
    nearest
}

fn game_pose_from_view_transform(transform: &Transform) -> [[f32; 4]; 4] {
    let right = view_to_game(transform.rotation * Vec3::X);
    let up = view_to_game(transform.rotation * Vec3::Y);
    let backward = view_to_game(transform.rotation * Vec3::Z);
    let translation = view_to_game(transform.translation);
    [
        [right[0], up[0], backward[0], translation[0]],
        [right[1], up[1], backward[1], translation[1]],
        [right[2], up[2], backward[2], translation[2]],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

fn tome_camera_matrix_text(
    tome_world: &TomeWorld,
    camera: &Camera,
    transform: &Transform,
) -> String {
    let Some(viewport) = camera.physical_viewport_size() else {
        return "Tome camera matrix unavailable: camera has no physical viewport".to_owned();
    };
    // Supported full-view, unit-scale path captured in the original game.
    // Dynamic resolution and subrectangle ownership remain unsupported.
    let dimensions = (|| {
        let width = i32::try_from(viewport.x).map_err(|_| "viewport width overflow")?;
        let height = i32::try_from(viewport.y).map_err(|_| "viewport height overflow")?;
        let rect = tome::scaled_render_viewport([0, 0, width - 1, height - 1], [1.0; 2])?;
        tome::render_viewport_dimensions(rect)
    })();
    let [width, height] = match dimensions {
        Ok(dimensions) => dimensions,
        Err(error) => return format!("Tome camera viewport unavailable: {error}"),
    };
    let camera_pose = game_pose_from_view_transform(transform);
    let matrix = match tome::original_world_visibility_query_matrix(
        ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES,
        width as u32,
        height as u32,
        1.0,
        camera_pose,
        ORIGINAL_DEFAULT_NEAR,
        ORIGINAL_DEFAULT_FAR,
    ) {
        Ok(matrix) => matrix,
        Err(error) => return format!("Tome camera matrix unavailable: {error}"),
    };
    let planes = tome::query_planes_from_rows(matrix, matrix);
    let camera_position = view_to_game(transform.translation);
    match tome_world.tree.record_nodes_for_camera_query_prefilter(
        camera_position,
        &planes,
        [0.0; 3],
    ) {
        Ok(nodes) => format!(
            "Tome matrix from registered defaults: viewport={}x{}, pixel aspect=1, camera_game_xyz={camera_position:?}, rows={matrix:?}\nGlobal spatial-record candidates from geometric prefilter: {nodes:?}. This geometric prefilter is not the final visibility result; the live query's traversal, accepted-object output, mapping and upload diagnostics are in the matching streaming history.",
            viewport.x, viewport.y
        ),
        Err(error) => format!(
            "Tome matrix from registered defaults: viewport={}x{}, camera_game_xyz={camera_position:?}, rows={matrix:?}\nGlobal spatial-record prefilter failed: {error}",
            viewport.x, viewport.y
        ),
    }
}

fn tome_point_text(
    tome_world: &TomeWorld,
    world: &world_mesh::WorldMesh,
    render_queue: &SceneRenderQueue,
    label: &str,
    point: [f32; 3],
) -> String {
    let point_volume_records = tome_world
        .tree
        .record_leaves_containing_point(point)
        .into_iter()
        .map(|index| (index, tome_world.tree.record_offset(index)))
        .collect::<Vec<_>>();
    if point_volume_records.is_empty() {
        return format!("Tome {label} {point:?}: outside Tome bounds");
    }
    let mut lines = vec![format!(
        "Tome {label} {point:?}: point-containment prefilter touches {} authored spatial records (not the full visibility result); {} gate-state words; cell-slot rows {} × {} bits",
        point_volume_records.len(),
        tome_world.gate_states.len(),
        tome_world.query_data.slot_rows.spatial_record_count(),
        tome_world.query_data.slot_rows.bit_count()
    )];
    let point_cell =
        tome_world
            .query_data
            .point_cell_id(&tome_world.bytes, &tome_world.tree, point);
    let sphere_result = point_cell.and_then(|cell| {
        let graph = tome_world.sphere_graph.as_ref().map_err(Clone::clone)?;
        graph.query_spheres(
            &tome_world.bytes,
            &mut tome::SphereCellWorker::default(),
            &[[point[0], point[1], point[2], 0.0]],
            &[cell],
            &render_queue.visible_cell_mask,
            Some(&tome_world.gate_states),
            None,
            1,
        )
    });
    lines.push(format!(
        "  zero-radius sphere diagnostic (not light rejection): {sphere_result:?}"
    ));
    for (leaf, offset) in point_volume_records {
        let Some(offset) = offset.filter(|&offset| offset != 0) else {
            lines.push(format!("  spatial record {leaf}: no local Tome record"));
            continue;
        };
        let result = tome::CellTree::parse(&tome_world.bytes, offset).and_then(|cells| {
            let Some(cell) = cells.cell_for_point(&tome_world.bytes, point)? else {
                return Ok(None);
            };
            let candidates = cells.object_candidates(&tome_world.bytes, cell.cell_index)?;
            let authored_ids = candidates
                .iter()
                .map(|&index| tome::object_ids_for_index(&tome_world.bytes, index))
                .collect::<Result<Vec<_>, _>>()?;
            let bounds = candidates
                .iter()
                .map(|&index| tome::object_bounds(&tome_world.bytes, index))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some((cell, candidates, authored_ids, bounds)))
        });
        match result {
            Ok(Some((cell, candidates, authored_ids, bounds))) => {
                lines.push(format!(
                    "  point-containing spatial record {leaf}, cell ID {}, bounds {:?} .. {:?}, {} traversal links, {} point-query object candidates",
                    cell.cell_id, cell.bounds[0], cell.bounds[1], cell.link_count, candidates.len()));
                for ((index, authored_ids), bounds) in
                    candidates.into_iter().zip(authored_ids).zip(bounds)
                {
                    let instances = world.tome_object_instances.get(index as usize);
                    let materials = instances
                        .into_iter()
                        .flatten()
                        .filter_map(|&instance| {
                            world
                                .objects
                                .iter()
                                .find(|object| {
                                    object.matrix_start <= instance && instance < object.matrix_end
                                })
                                .map(|object| {
                                    format!("{} (LOD {})", object.material_path, object.lod)
                                })
                        })
                        .collect::<Vec<_>>();
                    let render_indices = render_queue.tome_render_instances.get(index as usize);
                    let active = render_indices
                        .iter()
                        .filter(|&&render_index| {
                            render_queue
                                .instances
                                .get(render_index)
                                .is_some_and(|instance| instance.visible)
                        })
                        .count();
                    lines.push(format!("    Tome index {index}, bounds {:?} .. {:?}, authored IDs {authored_ids:?}: BWM instances {}, rewrite render instances {} (currently active {}), materials {:?}",
                    bounds[0], bounds[1], instances.map_or(0, Vec::len),
                    render_indices.len(), active, materials));
                }
            }
            Ok(None) => lines.push(format!(
                "  point-candidate spatial record {leaf}: point is outside its local cells"
            )),
            Err(error) => lines.push(format!(
                "  point-candidate spatial record {leaf}: local lookup failed: {error}"
            )),
        }
    }
    lines.join("\n")
}

fn collision_report(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    scene: Res<SceneData>,
    navigation: Res<NavigationState>,
    collision: Option<Res<NavigationWorld>>,
    tome_world: Option<Res<TomeWorld>>,
    render_queue: Res<SceneRenderQueue>,
    cameras: Query<(&Camera, &Transform), With<Camera3d>>,
    diagnostics: Option<Res<StreamingDiagnostics>>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    let Ok((camera_component, camera)) = cameras.single() else {
        return;
    };
    let direction = camera.forward().as_vec3();
    let distance_limit = 40.0;
    let visual =
        sightline_world_surface(&scene.world, camera.translation, direction, distance_limit);
    let visual_text = match &visual {
        Some((distance, mesh, matrix, material)) => format!(
            "BWM surface: distance={distance:.3} m, game_xyz={:?}, mesh={mesh}, matrix={matrix}, material={material}",
            view_to_game(camera.translation + direction * *distance)
        ),
        None => format!("BWM surface: none along center ray within {distance_limit} m"),
    };
    let probe_feet = if navigation.walking {
        navigation.walker.feet
    } else {
        camera.translation - Vec3::Y * 1.7
    };
    let collision_text = if let Some(world) = collision.as_ref() {
        let details = world.0.first_hit_details(
            camera.translation,
            camera.translation + direction * distance_limit,
        );
        let hit = details.map(|hit| hit.0);
        let provenance = details.map(|(_, owner, vertices)| {
            (
                world.0.owner_name(owner).unwrap_or("level bphysworld"),
                vertices.map(view_to_game),
            )
        });
        let target = visual
            .as_ref()
            .map(|(distance, _, _, _)| camera.translation + direction * *distance)
            .unwrap_or(camera.translation + direction * 5.0);
        let hit_position =
            hit.map(|distance| view_to_game(camera.translation + direction * distance));
        let visual_gap = visual.as_ref().and_then(|(distance, _, _, _)| {
            hit.map(|collision_distance| collision_distance - *distance)
        });
        format!(
            "Collision ray hit: {hit:?} m; game_xyz={hit_position:?}; distance from visual surface={visual_gap:?} m\nCollision source and triangle game_xyz: {provenance:?}\nCollision triangles within 2 m of visual target: {}\nCollision triangles within 2 m of feet probe: {}\nFloor at feet probe: {:?}",
            world.0.nearby_triangle_count(target, 2.0),
            world.0.nearby_triangle_count(probe_feet, 2.0),
            world.0.floor_at(probe_feet, 0.5, 2.0)
        )
    } else {
        "Collision world: unavailable".to_owned()
    };
    let tome_text = if let Some(tome_world) = tome_world.as_ref() {
        format!(
            "{}\n{}\n{}",
            tome_point_text(
                tome_world,
                &scene.world,
                &render_queue,
                "eye",
                view_to_game(camera.translation)
            ),
            tome_point_text(
                tome_world,
                &scene.world,
                &render_queue,
                "feet",
                view_to_game(probe_feet)
            ),
            tome_camera_matrix_text(tome_world, camera_component, camera),
        )
    } else {
        "Tome: no matching extracted map file loaded".to_owned()
    };
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.as_millis())
        .unwrap_or(0);
    let directory = scene
        .source_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("collision-reports");
    if let Err(error) = fs::create_dir_all(&directory) {
        eprintln!("Collision report directory: {error}");
        return;
    }
    let stem = format!("collision-{stamp}");
    let report_path = directory.join(format!("{stem}.txt"));
    let screenshot_path = directory.join(format!("{stem}.png"));
    let report = format!(
        "Level: {}\nCamera game_xyz: {:?}\nCamera direction game_xyz: {:?}\nFeet probe game_xyz: {:?}\nMode: {}\nWalker supported: {}\nWalker vertical speed: {:.3} m/s\n{}\n{}\n{}\nScreenshot: {}\nNote: In free-fly, feet probe is 1.7 m below the camera; walker state is inactive. BWM ray tests static world geometry; placed visual models are identified in the screenshot. Collision ray tests the combined loaded collision index. Tome object candidates precede the game's final visibility tests.\n",
        scene.source_path.display(),
        view_to_game(camera.translation),
        view_to_game(direction),
        view_to_game(probe_feet),
        if navigation.walking {
            "walk"
        } else {
            "free-fly"
        },
        navigation.walking && navigation.walker.supported,
        if navigation.walking {
            navigation.walker.vertical_speed
        } else {
            0.0
        },
        visual_text,
        collision_text,
        tome_text,
        screenshot_path.display()
    );
    let history_path = directory.join(format!("{stem}-streaming.log"));
    if let Some(diagnostics) = diagnostics.as_ref() {
        let history = format!(
            "Level: {}\nSession log: {}\n{}",
            scene.source_path.display(),
            diagnostics.log_path.display(),
            diagnostics
                .history
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
        if let Err(error) = fs::write(&history_path, history) {
            eprintln!(
                "Streaming history capture {}: {error}",
                history_path.display()
            );
        } else {
            println!("Streaming history saved: {}", history_path.display());
        }
    }
    match fs::write(&report_path, report) {
        Ok(()) => {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(screenshot_path));
            println!("Collision report saved: {}", report_path.display());
        }
        Err(error) => eprintln!("Collision report write failed: {error}"),
    }
}

// FUN_14037d8d0 dispatches the query jobs and waits before consuming their
// outputs. Reuse Bevy's persistent worker pool, not per-frame OS threads.
fn run_tome_render_jobs(
    world: &TomeWorld,
    input: &tome::OrdinaryDirectQueryInput<'_>,
) -> Vec<tome::RenderQueryJob> {
    let run = |job| {
        tome::ordinary_direct_tome_render_job(
            &world.tree,
            &world.bytes,
            &world.query_data,
            input,
            job,
            world.query_job_count,
        )
    };
    if world.query_job_count == 1 {
        return vec![run(0)];
    }
    bevy::tasks::ComputeTaskPool::get().scope(|scope| {
        for job in 0..world.query_job_count {
            let run = &run;
            scope.spawn(async move { run(job) });
        }
    })
}

// The ordinary direct-Tome query selects mapped static BWM instances and
// forwards their projected image demand. The installed worker clears only
// its own output on error, then returns; merge the successful siblings, log
// every failed job, and never retain a failed job's old or partial results.
fn stream_visible_models(
    mut commands: Commands,
    cameras: Query<(Option<&Camera>, Option<&Transform>), With<Camera3d>>,
    mut scene: ResMut<SceneRenderQueue>,
    tome_world: Option<Res<TomeWorld>>,
    placed_model_queries: Option<Res<PlacedModelQueries>>,
    pickups: Res<PickupState>,
    mut timing: Option<ResMut<VisibilityElapsedTiming>>,
    mut diagnostics: Option<ResMut<StreamingDiagnostics>>,
    mut last_tome_error: Local<Option<String>>,
) {
    let started = Instant::now();
    // Every frame replaces these outputs, including frames without a usable
    // camera. Do not expose a previous view's cells or IDs to light selection.
    scene.visible_authored_ids.clear();
    scene.visible_cell_indices.clear();
    scene.visible_cell_mask.clear();
    scene.light_query_state = None;
    let Ok((camera, transform)) = cameras.single() else {
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.frame += 1;
            diagnostics.status = "camera_unavailable".to_owned();
            diagnostics.pending = format!(
                "frame={} status=camera_unavailable authored_ids=0 visible_cell_entries=0 cell_mask_words=0 light_binding=unavailable sphere_binding=unavailable",
                diagnostics.frame
            );
        }
        return;
    };
    let mut query_ns = 0;
    let mut mapping_ns = 0;
    let mut status = if tome_world.is_some() {
        "camera_not_ready".to_owned()
    } else {
        "no_tome".to_owned()
    };
    let mut query_diagnostics = Vec::new();
    let mut job_errors = Vec::new();
    let mut query_rows = None;
    let mut query_viewport = None;
    let mut merged_query_tile = None;
    let mut requested = 0;
    let mut mapped_count = 0;
    let mut spatial_accepted = 0;
    let mut suppressed = 0;
    let mut accepted_sample = Vec::new();
    let mut shown = 0;
    let mut spawned = 0;
    let mut hidden = 0;
    let mut tome_visibility = tome_world
        .as_ref()
        .map(|_| vec![false; scene.instances.len()]);
    if let (Some(tome_world), Some(camera), Some(transform)) =
        (tome_world.as_ref(), camera, transform)
    {
        if let Some(viewport) = camera.physical_viewport_size() {
            query_viewport = Some([viewport.x, viewport.y]);
            scene.texture_viewport_pixels = viewport.x.saturating_mul(viewport.y);
            let query_started = Instant::now();
            let query = tome::original_world_visibility_query_matrix(
                ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES,
                viewport.x,
                viewport.y,
                1.0,
                game_pose_from_view_transform(transform),
                ORIGINAL_DEFAULT_NEAR,
                ORIGINAL_DEFAULT_FAR,
            )
            .and_then(|rows| {
                query_rows = Some(rows);
                let input = tome::OrdinaryDirectQueryInput {
                    camera_point: view_to_game(transform.translation),
                    render_rows: rows,
                    cropped_rows: rows,
                    initial_tile_rect: [0, 0, 8, 16],
                    camera_imprecision: [0.6; 3],
                    global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                    object_imprecision: 0.6,
                    minimum_area_metric: 25.0 / (viewport.x as f32 * viewport.y as f32),
                    winding_flip: tome::query_matrix_winding_flip(rows),
                    gate_states: Some(&tome_world.gate_states),
                    output_capacity: tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
                };
                let jobs = run_tome_render_jobs(tome_world, &input);
                let result = tome::merge_render_query_outputs(&jobs);
                job_errors =
                    jobs.iter()
                        .filter_map(|job| {
                            job.result.as_ref().err().map(|error| {
                                format!("{error} (jobId {})", job.diagnostics.job_index)
                            })
                        })
                        .collect::<Vec<_>>();
                query_diagnostics = jobs
                    .into_iter()
                    .map(|job| job.diagnostics)
                    .collect::<Vec<_>>();
                result
            });
            query_ns = query_started.elapsed().as_nanos();
            let mapping_started = Instant::now();
            match query {
                Ok(Some(output)) => {
                    merged_query_tile = output.state.as_ref().map(|state| state.viewport);
                    requested = output.indices().len();
                    accepted_sample.extend(output.indices().iter().take(16).copied());
                    let mut visible = vec![false; scene.instances.len()];
                    let mapping = &scene.tome_render_instances;
                    let mapped = mapping
                        .mark_visible_instances(output.indices(), &mut visible)
                        .and_then(|()| {
                            spatial_accepted = mapping
                                .apply_spatial_queries(output.state.as_ref(), &mut visible)?;
                            Ok(())
                        })
                        .and_then(|()| {
                            mapping
                                .submit_material_texture_demands(
                                    output.indices(),
                                    output.area_metrics(),
                                    &scene.instances,
                                    &scene.diffuse_image_demands,
                                    &scene.native_material_images,
                                )
                                .map(|_| ())
                        })
                        .and_then(|()| scene.publish_light_query_outputs(&tome_world, &output))
                        .and_then(|()| {
                            if let (Some(groups), Some(state)) =
                                (placed_model_queries.as_ref(), output.state.as_ref())
                            {
                                apply_placed_model_queries(
                                    &mut visible,
                                    &groups.0,
                                    state,
                                    view_to_game(transform.translation),
                                    [viewport.x, viewport.y],
                                )?;
                            }
                            Ok(())
                        });
                    match mapped {
                        Ok(()) => {
                            mapped_count = visible.iter().filter(|&&visible| visible).count();
                            RenderSuppressionGroup::apply_all(
                                &scene.visibility_suppression_groups,
                                &mut visible,
                            );
                            suppressed =
                                mapped_count - visible.iter().filter(|&&visible| visible).count();
                            tome_visibility = Some(visible);
                            if job_errors.is_empty() {
                                status = "ok".to_owned();
                                *last_tome_error = None;
                            } else {
                                let error = job_errors.join("; ");
                                status = if job_errors.len() == query_diagnostics.len() {
                                    // Native workers clear their lists on error;
                                    // the main query still publishes the fresh
                                    // (empty) merged result to light gathering.
                                    format!("all_jobs_failed: {}", job_errors.len())
                                } else {
                                    format!("ok_with_job_errors: {}", job_errors.len())
                                };
                                if last_tome_error.as_deref() != Some(&error) {
                                    eprintln!("Tome visibility jobs: {error}");
                                    *last_tome_error = Some(error);
                                }
                            }
                        }
                        Err(error) => {
                            status = format!("mapping_error: {error}");
                            if last_tome_error.as_deref() != Some(&error) {
                                eprintln!("Tome visibility mapping: {error}");
                                *last_tome_error = Some(error);
                            }
                        }
                    }
                }
                Ok(None) => {
                    status = "outside_tome_root".to_owned();
                    *last_tome_error = None;
                }
                Err(error) => {
                    status = format!("query_error: {error}");
                    if last_tome_error.as_deref() != Some(&error) {
                        eprintln!("Tome visibility query: {error}");
                        *last_tome_error = Some(error);
                    }
                }
            }
            mapping_ns = mapping_started.elapsed().as_nanos();
        }
    }
    let update_started = Instant::now();
    let mut active = scene.active;
    for (index, instance) in scene.instances.iter_mut().enumerate() {
        // FUN_14037e1d0 handles BWM indices; FUN_14031ad30 handles the
        // supported initial BML groups independently of serialized Tome IDs.
        let visible = tome_visibility.as_ref().is_some_and(|result| result[index]);
        let available = instance
            .pickup_id
            .as_ref()
            .is_none_or(|id| pickups.world.pickup_available(id));
        if visible && available {
            if let Some(entity) = instance.entity {
                if !instance.visible {
                    commands.entity(entity).insert(Visibility::Visible);
                    instance.visible = true;
                    active += 1;
                    shown += 1;
                }
            } else {
                let mut entity = match &instance.material {
                    MaterialHandle::Probe(material) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        instance.transform,
                        Visibility::Visible,
                    )),
                    MaterialHandle::Decal(material) => commands.spawn((
                        Mesh3d(instance.mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        instance.transform,
                        Visibility::Visible,
                    )),
                };
                // Streaming runs after transform propagation. These are
                // root entities: publish the authored world transform now,
                // rather than culling at the default identity for one frame.
                entity.insert((
                    MeshTag(instance.probe_index),
                    GlobalTransform::from(instance.transform),
                ));
                if let Some(id) = &instance.pickup_id {
                    entity.insert(PickupVisual(id.clone()));
                }
                instance.entity = Some(entity.id());
                instance.visible = true;
                active += 1;
                spawned += 1;
            }
        } else if instance.visible {
            if let Some(entity) = instance.entity {
                commands.entity(entity).insert(Visibility::Hidden);
                instance.visible = false;
                active -= 1;
                hidden += 1;
            }
        }
    }
    scene.active = active;
    if let Some(diagnostics) = diagnostics.as_deref_mut() {
        diagnostics.frame += 1;
        diagnostics.status = status;
        let pose = transform.map(game_pose_from_view_transform);
        let authored_id_sample =
            &scene.visible_authored_ids[..scene.visible_authored_ids.len().min(16)];
        let visible_cell_entries = scene.visible_cell_indices.len();
        let visible_cell_unique: u32 = scene
            .visible_cell_mask
            .iter()
            .map(|word| word.count_ones())
            .sum();
        let visible_cell_mask_words = scene.visible_cell_mask.len();
        let visible_cell_sample =
            &scene.visible_cell_indices[..scene.visible_cell_indices.len().min(16)];
        let distance_rejected_job_candidates: usize = query_diagnostics
            .iter()
            .map(|job| job.progress.distance_rejected_objects)
            .sum();
        diagnostics.pending = format!(
            "frame={} status={:?} jobs={} distance_rejected_job_candidates={distance_rejected_job_candidates} authored_ids={} authored_id_consumer=static_light_id_gate visible_cell_entries={visible_cell_entries} visible_cell_unique={visible_cell_unique} cell_mask_words={visible_cell_mask_words} light_binding=authored_ids_and_cells light_consumer=see_light_stage viewport={query_viewport:?} merged_query_tile={merged_query_tile:?} depth_grid=unsupported requested={requested} mapped={mapped_count} spatial_tested={} spatial_accepted_bwm={spatial_accepted} suppressed={suppressed} active={active}/{} spawned={spawned} shown={shown} hidden={hidden} query_ms={:.3} mapping_ms={:.3} elapsed_ms={:.3}\npose={pose:?}\nrender_rows={query_rows:?} winding_flip={:?}\naccepted_sample={accepted_sample:?}\nauthored_id_sample={authored_id_sample:?}\nvisible_cell_sample={visible_cell_sample:?}\n{query_diagnostics:?}",
            diagnostics.frame,
            diagnostics.status,
            query_diagnostics.len(),
            scene.visible_authored_ids.len(),
            scene.tome_render_instances.spatial_query_instances.len(),
            scene.instances.len(),
            query_ns as f64 / 1_000_000.0,
            mapping_ns as f64 / 1_000_000.0,
            started.elapsed().as_secs_f64() * 1000.0,
            query_rows.map(tome::query_matrix_winding_flip),
        );
    }
    if let Some(timing) = timing.as_deref_mut() {
        let elapsed_ns = started.elapsed().as_nanos();
        timing.samples += 1;
        timing.total_ns += elapsed_ns;
        timing.max_ns = timing.max_ns.max(elapsed_ns);
        timing.query_ns += query_ns;
        timing.mapping_ns += mapping_ns;
        timing.update_ns += update_started.elapsed().as_nanos();
    }
}

fn local_light_kind_index(kind: entities::LightType) -> Option<usize> {
    match kind {
        entities::LightType::Point => Some(0),
        entities::LightType::HalfPoint => Some(1),
        entities::LightType::Spot => Some(2),
        entities::LightType::Area => Some(3),
        entities::LightType::Parallel => None,
    }
}

// FUN_1400f0aa0 expands the light position by its cutoff to build the AABB
// projected by FUN_14010aea0. The non-Umbra path also gates this area test by
// its light-volume radius and clip-origin tests. The supported map Umbra
// path instead uses the cached-AABB query followed by the visible-cell sphere.
fn projected_local_light_area_pixels(
    clip_from_world: Mat4,
    center: Vec3,
    range: f32,
    viewport: UVec2,
) -> Option<f32> {
    if viewport.x == 0 || viewport.y == 0 || !range.is_finite() || range <= 0.0 {
        return None;
    }

    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for corner in 0..8 {
        let offset = Vec3::new(
            if corner & 1 == 0 { -range } else { range },
            if corner & 2 == 0 { -range } else { range },
            if corner & 4 == 0 { -range } else { range },
        );
        let clip = clip_from_world * (center + offset).extend(1.0);
        if !clip.x.is_finite()
            || !clip.y.is_finite()
            || !clip.w.is_finite()
            || clip.w < f32::MIN_POSITIVE
        {
            return None;
        }
        let ndc = Vec2::new(clip.x / clip.w, clip.y / clip.w);
        min = min.min(ndc);
        max = max.max(ndc);
    }

    let ndc_area = (max.x - min.x) * (max.y - min.y);
    let pixel_area = ndc_area * viewport.x as f32 * viewport.y as f32 * 0.25;
    pixel_area.is_finite().then_some(pixel_area.max(0.0))
}

// FUN_140232bc0 retains the transformed canonical volume at +0x170.
// This is not the position +/- range box used by the non-Umbra area gate.
fn original_light_projection_bounds(clip_to_world: Mat4) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for index in 0..8 {
        let clip = Vec3::new(
            (index & 1) as f32,
            ((index >> 1) & 1) as f32,
            ((index >> 2) & 1) as f32,
        );
        let world = clip_to_world.project_point3(clip);
        if !world.is_finite() {
            return None;
        }
        min = min.min(world);
        max = max.max(world);
    }
    Some((min, max))
}

fn original_light_proximity_from_cached_bounds(
    max_visible_range: f32,
    bounds: Option<(Vec3, Vec3)>,
) -> Option<(Vec3, f32)> {
    let bounds = if max_visible_range > 0.0 {
        let (min, max) = bounds?;
        [min.to_array(), max.to_array()]
    } else {
        [[0.0; 3]; 2]
    };
    let (center, radius) = render_light::proximity_from_bounds(max_visible_range, bounds);
    let center = Vec3::from_array(center);
    (center.is_finite() && radius.is_finite() && radius >= 0.0).then_some((center, radius))
}

#[cfg(test)]
fn original_light_proximity_bounds(
    max_visible_range: f32,
    clip_to_world: Option<Mat4>,
) -> Option<(Vec3, f32)> {
    original_light_proximity_from_cached_bounds(
        max_visible_range,
        clip_to_world.and_then(original_light_projection_bounds),
    )
}

fn original_light_proximity_pass(view_origin: Vec3, center: Vec3, radius: f32) -> bool {
    // FUN_14031bcc0 compares the maximum absolute component, not Euclidean distance.
    if !view_origin.is_finite() || !center.is_finite() || !radius.is_finite() || radius < 0.0 {
        return true;
    }
    (view_origin - center).abs().max_element() <= radius
}

// FUN_1408812b0 hashes the assigned declaration's name into actor +0x4b0;
// FUN_140881640 transfers it to +0x58 for static, nonparallel idLights.
// Blueprint/runtime-generated identities are not supplied by this map parser.
fn original_local_light_id_pass(
    kind: entities::LightType,
    culling: entities::LightCulling,
    definition_name_hash: u32,
    visible_ids: Option<&[u32]>,
) -> bool {
    kind == entities::LightType::Parallel
        || culling == entities::LightCulling::DynamicPosition
        || visible_ids
            .is_none_or(|ids| tome::sorted_authored_id_index(ids, definition_name_hash).is_some())
}

fn original_light_rgb_nonzero(color: [f32; 3]) -> bool {
    color
        .into_iter()
        .fold(0, |bits, value| bits | value.to_bits())
        & 0x7fff_ffff
        != 0
}

// FUN_140232bc0's flag update; the owning-world update lifecycle is separate.
fn original_render_light_update_flags(
    primary: u8,
    previous: u8,
    updates: u32,
    color: [f32; 3],
) -> u8 {
    let active = updates != 0;
    let eligible = active && primary & 1 != 0 && original_light_rgb_nonzero(color);
    (previous & 0xe7) | (u8::from(active) << 4) | (u8::from(eligible) << 3)
}

fn cull_original_small_local_lights(
    enabled: Res<OriginalLocalLightCullEnabled>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut lights: Query<(
        Entity,
        &OriginalLocalLightCull,
        &GlobalTransform,
        &mut Visibility,
    )>,
    scene: Option<Res<SceneRenderQueue>>,
    tome_world: Option<Res<TomeWorld>>,
    mut batch: Local<LocalLightSphereBatch>,
    mut stats: ResMut<OriginalLocalLightCullStats>,
) {
    stats.total = 0;
    stats.zero_preset_rgb = 0;
    stats.total_by_kind = [0; 4];
    stats.proximity_culled = 0;
    stats.proximity_unverified = 0;
    stats.projection_bounds_unverified = 0;
    stats.source_class_counts = [0; 2];
    stats.static_name_hash_matches = 0;
    stats.static_id_culled = 0;
    stats.static_id_binding_active = false;
    stats.clip_culled = 0;
    stats.area_culled = 0;
    stats.sphere_candidates = 0;
    stats.sphere_culled = 0;
    stats.sphere_unverified = 0;
    stats.sphere_query_ns = 0;
    stats.sphere_error = None;
    batch.spheres.clear();
    batch.point_cells.clear();
    batch.entities.clear();
    let camera = cameras.single().ok().and_then(|(camera, transform)| {
        let viewport = camera.physical_viewport_size()?;
        (enabled.0 && viewport.x > 0 && viewport.y > 0).then(|| {
            (
                camera.clip_from_view() * transform.to_matrix().inverse(),
                viewport,
                transform.translation(),
            )
        })
    });
    // A mask publishes fresh ordinary query outputs, including failed-worker
    // cleared lists. Unsupported camera/mapping frames publish no mask.
    let visible_ids = scene.as_ref().and_then(|scene| {
        (camera.is_some() && !scene.visible_cell_mask.is_empty())
            .then_some(scene.visible_authored_ids.as_slice())
    });
    stats.static_id_binding_active = visible_ids.is_some();
    let world_query = visible_ids
        .and_then(|_| scene.as_ref()?.light_query_state.as_ref())
        .map(tome::NoDepthAabbQuery::from_state)
        .transpose();
    stats.world_query_bound = matches!(&world_query, Ok(Some(_)));
    stats.world_query_unverified = 0;
    stats.world_query_error = world_query.as_ref().err().cloned();
    let minimum_area = camera.and_then(|(_, viewport, _)| {
        tome::minimum_light_projected_area_metric(
            i32::try_from(viewport.x).ok()?,
            i32::try_from(viewport.y).ok()?,
        )
        .ok()
    });
    for (entity, light, light_transform, mut visibility) in &mut lights {
        let Some(kind_index) = local_light_kind_index(light.kind) else {
            *visibility = Visibility::Visible;
            continue;
        };
        stats.total += 1;
        stats.total_by_kind[kind_index] += 1;
        // Preset RGB does not establish the runtime record after actor +0x408 updates.
        stats.zero_preset_rgb += usize::from(!original_light_rgb_nonzero(light.linear_color));
        stats.projection_bounds_unverified += usize::from(light.projection_bounds.is_none());
        let dynamic = light.culling == entities::LightCulling::DynamicPosition;
        stats.source_class_counts[usize::from(dynamic)] += 1;
        // The native source is the assigned compiled declaration, not the
        // optional runtime display name set by FUN_140878510.
        if !dynamic
            && scene.as_ref().is_some_and(|scene| {
                tome::sorted_authored_id_index(
                    &scene.visible_authored_ids,
                    light.definition_name_hash,
                )
                .is_some()
            })
        {
            stats.static_name_hash_matches += 1;
        }
        let mut culled = false;
        if !original_local_light_id_pass(
            light.kind,
            light.culling,
            light.definition_name_hash,
            visible_ids,
        ) {
            stats.static_id_culled += 1;
            culled = true;
        } else if visible_ids.is_some() {
            // World-query candidates use cached AABB area, then sphere cells.
            // Proximity/clip-volume checks still follow in BOTH native branches.
            match (&world_query, light.projection_bounds, minimum_area) {
                (Ok(Some(query)), Some((low, high)), Some(minimum_area)) => {
                    let low = Vec3::from_array(view_to_game(low));
                    let high = Vec3::from_array(view_to_game(high));
                    let bounds = [low.min(high).to_array(), low.max(high).to_array()];
                    match query.visibility_and_area(bounds) {
                        Ok((false, _)) => {
                            stats.clip_culled += 1;
                            culled = true;
                        }
                        Ok((true, area)) if area < minimum_area => {
                            stats.area_culled += 1;
                            culled = true;
                        }
                        Ok(_) => {}
                        Err(error) => {
                            stats.world_query_unverified += 1;
                            stats.world_query_error.get_or_insert(error);
                        }
                    }
                }
                _ => stats.world_query_unverified += 1,
            }
        } else if let Some((clip, viewport, view_origin)) = camera {
            let proximity_pass = match light.proximity_bounds {
                Some((center, radius)) => {
                    original_light_proximity_pass(view_origin, center, radius)
                }
                None => {
                    stats.proximity_unverified += 1;
                    true
                }
            };
            if !proximity_pass {
                stats.proximity_culled += 1;
                culled = true;
            } else if light
                .clip_to_world
                .and_then(|clip_to_world| {
                    original_light_volume_intersects_camera(clip, clip_to_world)
                })
                .is_some_and(|intersects| !intersects)
            {
                stats.clip_culled += 1;
                culled = true;
            } else if projected_local_light_area_pixels(
                clip,
                light_transform.translation(),
                light.range,
                viewport,
            )
            .is_some_and(|area| area < ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS)
            {
                stats.area_culled += 1;
                culled = true;
            }
        }
        if !culled && visible_ids.is_some() {
            if let Some(sphere) = light.initial_sphere {
                if sphere.iter().all(|v| v.is_finite()) {
                    batch.spheres.push(sphere);
                    batch.entities.push(entity);
                } else {
                    stats.sphere_unverified += 1;
                }
            } else {
                stats.sphere_unverified += 1;
            }
        }
        *visibility = if culled {
            Visibility::Hidden
        } else {
            Visibility::Visible
        };
    }
    stats.sphere_candidates = batch.spheres.len();
    if !batch.spheres.is_empty() {
        let started = Instant::now();
        let result = (|| {
            let world = tome_world
                .as_ref()
                .ok_or("light sphere query has no Tome world")?;
            let graph = world.sphere_graph.as_ref().map_err(String::as_str)?;
            let scene = scene
                .as_ref()
                .ok_or("light sphere query has no world result")?;
            let LocalLightSphereBatch {
                worker,
                spheres,
                point_cells,
                ..
            } = &mut *batch;
            for sphere in spheres.iter() {
                point_cells.push(
                    world
                        .query_data
                        .point_cell_id(&world.bytes, &world.tree, [sphere[0], sphere[1], sphere[2]])
                        .map_err(|error| {
                            stats.sphere_error.get_or_insert(error);
                            "light sphere point lookup failed"
                        })?,
                );
            }
            graph
                .query_spheres(
                    &world.bytes,
                    worker,
                    spheres,
                    point_cells,
                    &scene.visible_cell_mask,
                    Some(&world.gate_states),
                    None,
                    spheres.len(),
                )
                .map_err(|error| {
                    stats.sphere_error.get_or_insert(error);
                    "light sphere batch failed"
                })
        })();
        stats.sphere_query_ns = started.elapsed().as_nanos();
        match result {
            Ok(result) if result.error_code == 0 => {
                for &entity in &batch.entities {
                    if let Ok((_, _, _, mut visibility)) = lights.get_mut(entity) {
                        *visibility = Visibility::Hidden;
                    }
                }
                for index in result.indices {
                    if let Ok((_, _, _, mut visibility)) =
                        lights.get_mut(batch.entities[index as usize])
                    {
                        *visibility = Visibility::Visible;
                    }
                }
                stats.sphere_culled = batch.entities.len() - result.count;
            }
            Ok(result) => {
                // The original gather treats errors other than 0/5 as fatal.
                // Keep this unsupported allocator/budget path explicitly visible.
                stats.sphere_unverified += batch.entities.len();
                stats.sphere_error = Some(format!(
                    "native sphere error {}: allocator/budget handling unsupported",
                    result.error_code
                ));
            }
            Err(error) => {
                stats.sphere_unverified += batch.entities.len();
                stats.sphere_error.get_or_insert(error.to_owned());
            }
        }
    }
    // FUN_14031bcc0: sphere survivors still pass the +0x188 proximity
    // quad and +0x130 projected-volume intersection before submission.
    if visible_ids.is_some() {
        if let Some((clip, _, view_origin)) = camera {
            for (_, light, _, mut visibility) in &mut lights {
                if *visibility == Visibility::Hidden || local_light_kind_index(light.kind).is_none()
                {
                    continue;
                }
                let proximity_pass = match light.proximity_bounds {
                    Some((center, radius)) => {
                        original_light_proximity_pass(view_origin, center, radius)
                    }
                    None => {
                        stats.proximity_unverified += 1;
                        true
                    }
                };
                if !proximity_pass {
                    stats.proximity_culled += 1;
                    *visibility = Visibility::Hidden;
                } else if light
                    .clip_to_world
                    .and_then(|matrix| original_light_volume_intersects_camera(clip, matrix))
                    .is_some_and(|hit| !hit)
                {
                    stats.clip_culled += 1;
                    *visibility = Visibility::Hidden;
                }
            }
        }
    }
}

// Run after Bevy's visibility pass to distinguish Tome decisions from Bevy
// culling and texture upload state. ViewVisibility includes any camera/light
// view; this count is not a GPU draw count.
fn finish_streaming_diagnostics(
    mut diagnostics: ResMut<StreamingDiagnostics>,
    render_diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    mut light_culling: ResMut<OriginalLocalLightCullStats>,
    scene: Res<SceneRenderQueue>,
    meshes: Query<&ViewVisibility, With<MeshTag>>,
    local_lights: Query<(&ViewVisibility, &OriginalLocalLightCull)>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let mut render_metrics = render_diagnostics
        .as_deref()
        .into_iter()
        .flat_map(|store| store.iter())
        .filter(|diagnostic| {
            let path = diagnostic.path().as_str();
            path.starts_with("render/") || matches!(path, "fps" | "frame_time")
        })
        .filter_map(|diagnostic| {
            diagnostic
                .value()
                .map(|value| format!("{}={value:.3}{}", diagnostic.path(), diagnostic.suffix))
        })
        .collect::<Vec<_>>();
    render_metrics.sort_unstable();
    let render_metrics = if render_metrics.is_empty() {
        String::new()
    } else {
        format!("\nrender_diagnostics {}", render_metrics.join(" "))
    };
    light_culling.visible_by_kind = [0; 4];
    for (visibility, light) in &local_lights {
        if visibility.get() {
            if let Some(kind_index) = local_light_kind_index(light.kind) {
                light_culling.visible_by_kind[kind_index] += 1;
            }
        }
    }
    let identity_sample = format!(
        "light_source_class[static,dynamic]={:?} static_name_hash_matches={} static_id_culled={} zero_preset_rgb={} rgb_eligibility=unsupported identity_binding={} projection_bounds_unverified={} world_query_bound={} world_query_unverified={} world_query_error={:?} projection_binding=no_depth sphere_path=initial_single_tome sphere_candidates={} sphere_culled={} sphere_unverified={} sphere_query_ms={:.3} sphere_error={:?}\n",
        light_culling.source_class_counts,
        light_culling.static_name_hash_matches,
        light_culling.static_id_culled,
        light_culling.zero_preset_rgb,
        if light_culling.static_id_binding_active {
            "compiled_map_declaration"
        } else {
            "unavailable"
        },
        light_culling.projection_bounds_unverified,
        light_culling.world_query_bound,
        light_culling.world_query_unverified,
        light_culling.world_query_error,
        light_culling.sphere_candidates,
        light_culling.sphere_culled,
        light_culling.sphere_unverified,
        light_culling.sphere_query_ns as f64 / 1_000_000.0,
        light_culling.sphere_error,
    );
    let sample = format!(
        "{}{identity_sample}\nbevy_any_view_visible={} entities={} local_lights_culled[proximity,clip,area]={}/{}/{}/{} proximity_unverified={} light_total_by_kind[point,half_point,spot,area]={:?} light_visible_by_kind[point,half_point,spot,area]={:?} cutoff_px={:.0} diffuse_resident={}/{} diffuse_pending={} diffuse_failed={} mip_upgrades={}{}\n",
        diagnostics.pending,
        meshes.iter().filter(|visible| visible.get()).count(),
        meshes.iter().count(),
        light_culling.proximity_culled,
        light_culling.clip_culled,
        light_culling.area_culled,
        light_culling.total,
        light_culling.proximity_unverified,
        light_culling.total_by_kind,
        light_culling.visible_by_kind,
        ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
        scene
            .diffuse_image_handles
            .iter()
            .filter(|handle| handle.is_some())
            .count(),
        scene.diffuse_image_handles.len(),
        scene
            .diffuse_image_pending
            .iter()
            .filter(|pending| pending.is_some())
            .count(),
        scene
            .diffuse_image_attempted
            .iter()
            .enumerate()
            .filter(|&(slot, attempted)| *attempted
                && scene.diffuse_image_handles[slot].is_none()
                && scene.diffuse_image_pending[slot].is_none())
            .count(),
        scene.diffuse_mip_upgrades,
        render_metrics,
    );
    diagnostics.record(sample, scene.active, keys.just_pressed(KeyCode::F8));
}

fn benchmark_frames(
    // Performance must include stalls; gameplay Time clamps deltas to 250 ms.
    time: Res<Time<bevy::time::Real>>,
    target: Option<Res<BenchmarkSeconds>>,
    models: Res<SceneRenderQueue>,
    camera: Query<&Transform, With<Camera3d>>,
    visibility_timing: Option<Res<VisibilityElapsedTiming>>,
    light_culling: Res<OriginalLocalLightCullStats>,
    mut elapsed: Local<f32>,
    mut measured: Local<(u64, f64)>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(target) = target else {
        return;
    };
    *elapsed += time.delta_secs();
    if *elapsed > 2.0 {
        measured.0 += 1;
        measured.1 += time.delta_secs_f64();
    }
    if *elapsed >= target.0 {
        let (samples, average_ms, max_ms) = visibility_timing
            .as_ref()
            .map(|timing| {
                (
                    timing.samples,
                    timing.total_ns as f64 / timing.samples.max(1) as f64 / 1_000_000.0,
                    timing.max_ns as f64 / 1_000_000.0,
                )
            })
            .unwrap_or_default();
        println!(
            "BENCHMARK frames={} measured_seconds={:.2} fps={:.1} visible={}/{} local_lights_culled[proximity,clip,area]={}/{}/{}/{} proximity_unverified={} light_total_by_kind[point,half_point,spot,area]={:?} light_visible_by_kind[point,half_point,spot,area]={:?} cutoff_px={:.0} diffuse_resident={}/{} mip_upgrades={} camera_game_xyz={:?} visibility_elapsed_samples={} visibility_elapsed_avg_ms={:.3} visibility_elapsed_max_ms={:.3} query_avg_ms={:.3} mapping_avg_ms={:.3} update_avg_ms={:.3} static_id_culled={} identity_binding_active={} sphere_path=initial_single_tome sphere_candidates={} sphere_culled={} sphere_unverified={} sphere_query_ms={:.3} rgb_eligibility=unsupported zero_preset_rgb={}",
            measured.0,
            measured.1,
            measured.0 as f64 / measured.1.max(1e-6),
            models.active,
            models.instances.len(),
            light_culling.proximity_culled,
            light_culling.clip_culled,
            light_culling.area_culled,
            light_culling.total,
            light_culling.proximity_unverified,
            light_culling.total_by_kind,
            light_culling.visible_by_kind,
            ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
            models
                .diffuse_image_handles
                .iter()
                .filter(|handle| handle.is_some())
                .count(),
            models.diffuse_image_handles.len(),
            models.diffuse_mip_upgrades,
            camera
                .single()
                .ok()
                .map(|transform| view_to_game(transform.translation)),
            samples,
            average_ms,
            max_ms,
            visibility_timing
                .as_ref()
                .map_or(0.0, |timing| timing.query_ns as f64
                    / timing.samples.max(1) as f64
                    / 1_000_000.0),
            visibility_timing
                .as_ref()
                .map_or(0.0, |timing| timing.mapping_ns as f64
                    / timing.samples.max(1) as f64
                    / 1_000_000.0),
            visibility_timing
                .as_ref()
                .map_or(0.0, |timing| timing.update_ns as f64
                    / timing.samples.max(1) as f64
                    / 1_000_000.0),
            light_culling.static_id_culled,
            light_culling.static_id_binding_active,
            light_culling.sphere_candidates,
            light_culling.sphere_culled,
            light_culling.sphere_unverified,
            light_culling.sphere_query_ns as f64 / 1_000_000.0,
            light_culling.zero_preset_rgb,
        );
        exit.write(AppExit::Success);
    }
}

fn show_frame_rate(
    time: Res<Time<bevy::time::Real>>,
    scene: Res<SceneRenderQueue>,
    mut window: Single<&mut Window>,
    mut elapsed: Local<f32>,
    mut frames: Local<u32>,
) {
    *elapsed += time.delta_secs();
    *frames += 1;
    if *elapsed >= 1.0 {
        window.title = format!(
            "Dishonored 2 Rust | {:.0} FPS | {} loaded",
            *frames as f32 / *elapsed,
            scene.active
        );
        *elapsed = 0.0;
        *frames = 0;
    }
}

fn default_world_path(executable: &Path) -> Option<PathBuf> {
    for world in ["eotw_p_lowchaos.bwm", "menu_p.bwm"] {
        for folder in executable.ancestors().skip(1) {
            let candidate = folder.join("artifacts").join(world);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn world_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(argument) = env::args_os().nth(1) {
        let path = PathBuf::from(argument);
        if path.to_string_lossy().starts_with('[') {
            return Err(
                "that looks like a Markdown link; pass a file path, or run without an argument"
                    .into(),
            );
        }
        return Ok(path);
    }
    let executable = env::current_exe()?;
    default_world_path(&executable).ok_or_else(|| {
        "cannot find artifacts/eotw_p_lowchaos.bwm or artifacts/menu_p.bwm beside the project; pass an extracted .bwm path".into()
    })
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let profile = env::var("DH2_PERF_PROFILE").as_deref() == Ok("1");
    let original_light_area_cull = env::var("DH2_ORIGINAL_LIGHT_AREA_CULL").as_deref() != Ok("0");
    println!(
        "Original local-light culling: {} (initial map Umbra candidates + non-Umbra fallback; fallback minimum {:.0} px; DH2_ORIGINAL_LIGHT_AREA_CULL=0 disables)",
        if original_light_area_cull {
            "enabled"
        } else {
            "disabled"
        },
        ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS,
    );
    let startup = std::time::Instant::now();
    let path = world_path()?;
    let bytes =
        fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let world =
        world_mesh::parse(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if profile {
        println!(
            "PERF world mesh parse: {:.3}s",
            startup.elapsed().as_secs_f64()
        );
    }
    let (player_start, placed_models, pickups, entity_document) =
        match fs::read(path.with_extension("entities")) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                let document =
                    entities::parse(&text).map_err(|error| format!("entity parse: {error}"))?;
                (
                    entities::player_start(&document),
                    entities::render_placements(&document),
                    entities::pickups(&document),
                    Some(document),
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                (None, Vec::new(), Vec::new(), None)
            }
            Err(error) => return Err(format!("cannot read level entities: {error}").into()),
        };
    if env::var("DH2_TRANSFORM_AUDIT").as_deref() == Ok("1") {
        let samples = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let mut worst_world = (0.0f32, 0usize);
        let mut mismatches_world = 0usize;
        for (index, matrix) in world.matrices.iter().enumerate() {
            let converted = game_affine_to_view(matrix).to_matrix();
            for sample in samples {
                let difference = converted
                    .transform_point3(game_to_view(sample))
                    .distance(transformed(matrix, sample));
                if difference > worst_world.0 {
                    worst_world = (difference, index);
                }
                if difference > 0.01 {
                    mismatches_world += 1;
                }
            }
        }
        let mut worst_placed = (0.0f32, String::new());
        let mut mismatches_placed = 0usize;
        for placement in &placed_models {
            let converted = placed_transform(placement).to_matrix();
            for sample in samples {
                let difference = converted
                    .transform_point3(game_to_view(sample))
                    .distance(placed_vertex(placement, sample));
                if difference > worst_placed.0 {
                    worst_placed = (difference, placement.id.clone());
                }
                if difference > 0.01 {
                    mismatches_placed += 1;
                }
            }
        }
        println!(
            "TRANSFORM AUDIT: world worst={worst_world:?}m, samples exceeding 1cm={mismatches_world}; placed worst={worst_placed:?}m, samples exceeding 1cm={mismatches_placed}"
        );
        return Ok(());
    }
    println!(
        "Viewing {} meshes and {} objects from {}",
        world.meshes.len(),
        world.objects.len(),
        path.display()
    );
    if let Some(start) = player_start {
        println!("Player start: {:?}", start.position);
    }
    let game_base = env::var_os("DH2_GAME_BASE").map(PathBuf::from).or_else(|| {
        let known = PathBuf::from(r"D:\Programs\Steam\steamapps\common\Dishonored2\base");
        known.is_dir().then_some(known)
    });
    let catalog = game_base
        .as_ref()
        .map(|base| archive::Catalog::open(base))
        .transpose()?;
    if let (Some(base), Some(catalog)) = (game_base.as_ref(), catalog.as_ref()) {
        println!(
            "Opened {} indexed assets from {}",
            catalog.len(),
            base.display()
        );
    }
    let placed_lights = entity_document
        .as_ref()
        .map(|document| {
            entities::placed_lights_with_inheritance(document, |resource| {
                let Some(catalog) = catalog.as_ref() else {
                    return Ok(None);
                };
                let bytes = catalog.read(resource).map_err(|error| error.to_string())?;
                Ok(bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
            })
        })
        .transpose()?
        .unwrap_or_default();
    let environment_lighting = match (
        entity_document
            .as_ref()
            .and_then(entities::world_environment)
            .as_deref(),
        catalog.as_ref(),
    ) {
        (Some(environment), Some(catalog)) => {
            let resource = format!("generated/decls/env/{environment}.decl");
            catalog
                .read(&resource)?
                .map(|bytes| {
                    let source = String::from_utf8_lossy(&bytes);
                    entities::environment_lighting_with_inheritance(&source, |parent| {
                        let parent_resource = format!("generated/decls/env/{parent}.decl");
                        catalog
                            .read(&parent_resource)
                            .map(|bytes| {
                                bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                            })
                            .map_err(|error| error.to_string())
                    })
                    .map_err(std::io::Error::other)
                })
                .transpose()?
        }
        _ => None,
    };
    let probe_start = Instant::now();
    let environment_probes = match (catalog.as_ref(), path.file_stem().and_then(|s| s.to_str())) {
        (Some(catalog), Some(stem)) => {
            probe_lighting::load_for_world(catalog, stem).map_err(std::io::Error::other)?
        }
        _ => None,
    };
    if let Some(probes) = &environment_probes {
        println!(
            "Loaded authored probe lighting from {}: {} probes, {} secondary network entries, {} rejected PRB resources ({:.2}s)",
            probes.network_resource,
            probes.probes.len(),
            probes.secondary_resources.len(),
            probes.rejected_resources.len(),
            probe_start.elapsed().as_secs_f32(),
        );
    } else {
        println!("No unambiguous installed probe network matches the world stem.");
    }
    if let Some(environment) = environment_lighting {
        println!(
            "World environment lighting: ambient high={}, low={}, sun color={}, auto-exposure EV={:?}, luminance min/max={:?}/{:?}, adapt up/down={:?}/{:?}",
            environment.ambient_high.is_some() || environment.ambient_high_scalar.is_some(),
            environment.ambient_low.is_some() || environment.ambient_low_scalar.is_some(),
            environment.sun_color.is_some(),
            environment.auto_exposure_ev,
            environment.auto_exposure_min_luminance,
            environment.auto_exposure_max_luminance,
            environment.auto_exposure_adapt_speed_up,
            environment.auto_exposure_adapt_speed_down,
        );
    }
    println!("Authored light entities resolved: {}", placed_lights.len());
    let mut app = App::new();
    let tome_path = path.with_extension("tome");
    if tome_path.is_file() {
        let tome_started = Instant::now();
        let tome_bytes = fs::read(&tome_path)?;
        let tome_tree = tome::SpatialTree::parse(&tome_bytes)?;
        let gate_states = tome::initial_gate_states(&tome_bytes)?;
        let query_data = tome::DirectTomeQueryData::parse(&tome_bytes, &tome_tree)?;
        let query_job_count = env::var("DH2_UMBRA_QUERY_JOBS")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT))?;
        let threading_mode = env::var("DH2_SCHEDULER_THREADING_MODE")
            .map(|value| value.parse::<u32>())
            .unwrap_or(Ok(tome::ORIGINAL_DEFAULT_SCHEDULER_THREADING_MODE))?;
        let query_job_count =
            tome::original_world_query_job_count(query_job_count, threading_mode)?;
        println!(
            "Loaded original Tome from {}: {} spatial nodes, {} initially enabled gates; cell-slot rows {} × {} bits",
            tome_path.display(),
            tome_tree.node_count(),
            u32::from_le_bytes(tome_bytes[0x68..0x6c].try_into().unwrap()),
            query_data.slot_rows.spatial_record_count(),
            query_data.slot_rows.bit_count(),
        );
        println!(
            "Tome immutable query views: {} records decoded in {:.3}s; original query jobs={query_job_count}, scheduler mode={threading_mode}",
            query_data.decoded_record_count(),
            tome_started.elapsed().as_secs_f64(),
        );
        app.insert_resource(TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&tome_bytes),
            bytes: tome_bytes,
            tree: tome_tree,
            gate_states,
            query_data,
            query_job_count,
        });
    } else {
        println!(
            "No extracted Tome at {}; F8 Tome diagnostics unavailable",
            tome_path.display()
        );
    }
    let physics_path = path.with_extension("bphysworld");
    if physics_path.is_file() {
        let physics_start = std::time::Instant::now();
        let physics_bytes = fs::read(&physics_path)?;
        let physics = physworld::parse(&physics_bytes)?;
        let geometry = physics.collision_shapes(&physics_bytes)?;
        if profile {
            println!(
                "PERF authored physics parse and decode: {:.3}s",
                physics_start.elapsed().as_secs_f64()
            );
        }
        println!(
            "Authored collision: {} convex shapes, {} compressed meshes, {} triangles; {} compressed meshes remain undecoded",
            geometry.convex.len(),
            geometry.compressed.len(),
            geometry.triangles.len(),
            geometry.compressed_decode_failures.len()
        );
        let model_start = std::time::Instant::now();
        let (model_collision, collider_owners) = if let Some(catalog) = catalog.as_ref() {
            let bodies = entity_document.as_ref()
                .map(entities::rigid_body_settings).unwrap_or_default();
            let (triangles, owners, matched, failed) =
                static_model_collision(catalog, &placed_models, &bodies);
            println!(
                "Placed static-model collision: {matched} matched authored root HKX bodies, {} triangles, {failed} failures/unsupported recipes",
                triangles.len()
            );
            (triangles, owners)
        } else {
            (Vec::new(), Vec::new())
        };
        if profile {
            println!(
                "PERF placed model collision decode: {:.3}s",
                model_start.elapsed().as_secs_f64()
            );
        }
        let index_start = std::time::Instant::now();
        let world = navigation::CollisionWorld::with_owned(
            geometry
                .triangles
                .iter()
                .map(|triangle| (triangle.vertices.map(game_to_view), u32::MAX))
                .chain(model_collision),
            collider_owners,
        );
        if profile {
            println!(
                "PERF triangle collision index: {:.3}s",
                index_start.elapsed().as_secs_f64()
            );
        }
        println!(
            "Navigation collision index: {} nondegenerate triangles",
            world.triangle_count()
        );
        if let Some(start) = player_start {
            println!(
                "Navigation index: {} large triangles, {} candidates within 2m of spawn",
                world.large_triangle_count(),
                world.nearby_triangle_count(game_to_view(start.position), 2.0)
            );
        }
        if let Ok(ray) = env::var("DH2_NAV_RAY") {
            let values = ray
                .split(',')
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()?;
            if values.len() != 6 {
                return Err("DH2_NAV_RAY needs x,y,z,dx,dy,dz".into());
            }
            let start = game_to_view([values[0], values[1], values[2]]);
            let direction = game_to_view([values[3], values[4], values[5]]).normalize();
            let hit = world.first_hit_details(start, start + direction * 40.0);
            println!(
                "NAV RAY hit={:?}",
                hit.map(|(distance, owner, vertices)| (
                    distance,
                    world.owner_name(owner),
                    vertices.map(view_to_game)
                ))
            );
            if let Some(owner) = hit.and_then(|(_, owner, _)| world.owner_name(owner)) {
                if let Some(catalog) = &catalog {
                    for placement in expanded_model_placements(&placed_models, catalog)
                        .iter()
                        .filter(|placement| placement.id == owner)
                    {
                        println!("NAV RAY placement={placement:?}");
                    }
                }
                if let Some(document) = &entity_document {
                    for body in entities::rigid_body_settings(document)
                        .iter()
                        .filter(|body| body.entity_id == owner)
                    {
                        println!("NAV RAY body={body:?}");
                    }
                }
            }
            return Ok(());
        }
        if let Ok(point) = env::var("DH2_NAV_POINT") {
            let coordinates = point
                .split(',')
                .map(str::parse::<f32>)
                .collect::<Result<Vec<_>, _>>()?;
            if coordinates.len() != 3 {
                return Err("DH2_NAV_POINT needs x,y,z".into());
            }
            let point = game_to_view([coordinates[0], coordinates[1], coordinates[2]]);
            println!(
                "NAV POINT {:?}: floor={:?} floor-20cm={:?} floor-40cm={:?} nearby={}",
                point,
                world.floor_at(point, 0.05, 5.0),
                world.floor_at(point - Vec3::Y * 0.2, 0.05, 5.0),
                world.floor_at(point - Vec3::Y * 0.4, 0.05, 5.0),
                world.nearby_triangle_count(point, 2.0)
            );
            let mut walker = navigation::Walker::new(point, &world);
            let probe_start = std::time::Instant::now();
            for _ in 0..180 {
                walker.step(&world, Vec3::ZERO, 0.0, 1.0 / 60.0, false);
            }
            if profile {
                println!(
                    "PERF 180 walker steps: {:.3}s",
                    probe_start.elapsed().as_secs_f64()
                );
            }
            println!(
                "NAV POINT after 3s: feet={:?}, drop={:.3}m, supported={}",
                walker.feet,
                point.y - walker.feet.y,
                walker.supported
            );
            return Ok(());
        }
        if env::var("DH2_NAV_PROBE").as_deref() == Ok("1") {
            let start = player_start.ok_or("navigation probe needs a player start")?;
            let original = game_to_view(start.position);
            let floor = world.floor_at(original, 0.5, 2.0);
            let mut walker = navigation::Walker::new(original, &world);
            for _ in 0..180 {
                walker.step(&world, Vec3::ZERO, 0.0, 1.0 / 60.0, false);
            }
            println!(
                "NAV PROBE spawn feet={original:?} floor={floor:?} after 3s={:?} supported={}",
                walker.feet, walker.supported
            );
            for (label, direction) in [
                (
                    "forward",
                    game_to_view([start.forward_xy[0], start.forward_xy[1], 0.0]),
                ),
                (
                    "back",
                    -game_to_view([start.forward_xy[0], start.forward_xy[1], 0.0]),
                ),
                (
                    "right",
                    Vec3::new(start.forward_xy[1], 0.0, start.forward_xy[0]),
                ),
            ] {
                let mut walker = navigation::Walker::new(original, &world);
                for _ in 0..120 {
                    walker.step(&world, direction, 6.0, 1.0 / 60.0, false);
                }
                println!(
                    "NAV PROBE {label} after 2s: feet={:?}, displacement={:.2}m, supported={}",
                    walker.feet,
                    walker.feet.distance(original),
                    walker.supported
                );
            }
            let mut jumper = navigation::Walker::new(original, &world);
            let initial_height = jumper.feet.y;
            let mut peak = initial_height;
            for frame in 0..180 {
                jumper.step(&world, Vec3::ZERO, 0.0, 1.0 / 60.0, frame == 0);
                peak = peak.max(jumper.feet.y);
            }
            println!(
                "NAV PROBE jump peak +{:.2}m, after 3s feet={:?}, supported={}",
                peak - initial_height,
                jumper.feet,
                jumper.supported
            );
            return Ok(());
        }
        app.insert_resource(NavigationWorld(world));
    } else {
        println!(
            "No authored physics resource at {}; free-fly only",
            physics_path.display()
        );
    }
    if let Some(path) = env::var_os("DH2_SNAPSHOT") {
        app.insert_resource(SnapshotPath(PathBuf::from(path)));
    }
    if let Ok(seconds) = env::var("DH2_BENCHMARK_SECONDS") {
        app.insert_resource(BenchmarkSeconds(seconds.parse::<f32>()?.max(3.0)));
    }
    app.insert_resource(ClearColor(Color::srgb(0.055, 0.045, 0.06)))
        // Bevy allocates six depth layers for every shadow-casting point
        // light. Dense missions contain hundreds of authored lights, so keep
        // the shadow maps small enough to leave room for the mission assets.
        // Authored shadow flags remain intact.
        .insert_resource(PointLightShadowMap { size: 128 })
        .insert_resource(DirectionalLightShadowMap { size: 512 })
        .insert_resource(SceneData {
            source_path: path.clone(),
            world,
            player_start,
            placed_models,
            placed_lights,
            environment_lighting,
            environment_probes,
            pickups,
            catalog,
        })
        .add_plugins(DefaultPlugins);
    if env::var("DH2_RENDER_PROFILE").as_deref() == Ok("1") {
        println!(
            "Bevy render profiling enabled (GPU timestamps and pipeline stats require Vulkan or DX12)"
        );
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::render::diagnostic::RenderDiagnosticsPlugin,
        ));
    }
    app.add_plugins(AutoExposurePlugin)
        .add_plugins(MaterialPlugin::<ProbeMaterial>::default())
        .add_plugins(MaterialPlugin::<DecalMaterial>::default())
        .insert_resource(VisibilityElapsedTiming::default())
        .insert_resource(OriginalLocalLightCullEnabled(original_light_area_cull))
        .insert_resource(OriginalLocalLightCullStats::default())
        .insert_resource(StreamingDiagnostics::new(&path))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                camera_controls,
                update_coordinate_overlay,
                interact_pickup,
                snapshot_once,
                benchmark_frames,
                show_frame_rate,
            )
                .chain(),
        )
        .add_systems(
            PostUpdate,
            stream_visible_models
                .after(VisibilitySystems::UpdateFrusta)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(VisibilitySystems::CheckVisibility),
        )
        .add_systems(
            PostUpdate,
            cull_original_small_local_lights
                .after(stream_visible_models)
                .after(VisibilitySystems::UpdateFrusta)
                .before(VisibilitySystems::VisibilityPropagate)
                .before(VisibilitySystems::CheckVisibility),
        )
        .add_systems(
            PostUpdate,
            (finish_streaming_diagnostics, collision_report)
                .chain()
                .after(VisibilitySystems::CheckVisibility),
        )
        .add_systems(Update, stream_visible_textures);
    bevy::asset::load_internal_asset!(
        app,
        PROBE_SHADER_HANDLE,
        "../../assets/shaders/dh2_probe.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(app, SURFACE_SHADER_HANDLE, "../../assets/shaders/dh2_surface.wgsl", Shader::from_wgsl);
    bevy::asset::load_internal_asset!(app, DECAL_SHADER_HANDLE, "../../assets/shaders/dh2_decal.wgsl", Shader::from_wgsl);
    let prepared_images = PreparedImageMailbox::default();
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.insert_resource(prepared_images.clone());
        render_app.add_systems(
            Render,
            report_prepared_diffuse_images.after(RenderSystems::PrepareAssets),
        );
        app.insert_resource(prepared_images);
    }
    app.run();
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Game client error: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod game_control_tests {
    use super::*;

    #[test]
    #[ignore = "requires a graphics adapter; validates both authored material pipelines offscreen"]
    fn authored_surface_gpu_pipelines_compile() {
        use bevy::render::render_resource::{CachedPipelineState, PipelineDescriptor};
        let mut app = App::new();
        app.add_plugins(DefaultPlugins
            .set(bevy::window::WindowPlugin { primary_window: None, exit_condition: bevy::window::ExitCondition::DontExit, ..default() })
            .set(bevy::render::RenderPlugin { synchronous_pipeline_compilation: true, ..default() })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>())
            .add_plugins(MaterialPlugin::<ProbeMaterial>::default())
            .add_plugins(MaterialPlugin::<DecalMaterial>::default());
        bevy::asset::load_internal_asset!(app, PROBE_SHADER_HANDLE, "../../assets/shaders/dh2_probe.wgsl", Shader::from_wgsl);
        bevy::asset::load_internal_asset!(app, SURFACE_SHADER_HANDLE, "../../assets/shaders/dh2_surface.wgsl", Shader::from_wgsl);
        bevy::asset::load_internal_asset!(app, DECAL_SHADER_HANDLE, "../../assets/shaders/dh2_decal.wgsl", Shader::from_wgsl);
        app.finish(); app.cleanup();
        let target = app.world_mut().resource_mut::<Assets<Image>>()
            .add(Image::new_target_texture(64, 64, TextureFormat::Rgba8UnormSrgb, None));
        let mut mesh = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let n = mesh.count_vertices();
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.3, 0.7, 1.0, 1.0]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.25, 0.5]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0, 0.0, 0.0, 1.0]; n]);
        mesh.insert_attribute(DH2_BITANGENT, vec![[0.0, 1.0, 0.0]; n]);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let sh = app.world_mut().resource_mut::<Assets<ShaderBuffer>>().add(ShaderBuffer::from(vec![[Vec4::ZERO; 9]]));
        let mut surface = surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(&[]));
        surface.misc = UVec4::new(1, 1, 1, 2);
        surface.flags = UVec4::new(2, 1, 0, 1);
        let probe = app.world_mut().resource_mut::<Assets<ProbeMaterial>>().add(ProbeMaterial {
            base: StandardMaterial::default(),
            extension: ProbeExtension { sh, surface: surface.clone(),
                diffuse0: None,
                diffuse1: None,
                diffuse2: None,
                bump0: None,
                bump1: None,
                bump2: None,
                gloss0: None,
                gloss1: None,
                gloss2: None,
                metallic0: None,
                metallic1: None,
                metallic2: None,
                mask0: None,
                mask1: None,
                mask2: None,
            },
        });
        let decal = app.world_mut().resource_mut::<Assets<DecalMaterial>>().add(DecalMaterial {
            base: StandardMaterial { alpha_mode: AlphaMode::Blend, ..default() },
            extension: DecalExtension { bias: 0, surface,
                diffuse0: None,
                diffuse1: None,
                diffuse2: None,
                bump0: None,
                bump1: None,
                bump2: None,
                gloss0: None,
                gloss1: None,
                gloss2: None,
                metallic0: None,
                metallic1: None,
                metallic2: None,
                mask0: None,
                mask1: None,
                mask2: None,
            },
        });
        app.world_mut().spawn((Mesh3d(mesh.clone()), MeshMaterial3d(probe), Transform::from_xyz(-0.6, 0.0, 0.0)));
        app.world_mut().spawn((Mesh3d(mesh), MeshMaterial3d(decal), Transform::from_xyz(0.6, 0.0, 0.0)));
        app.world_mut().spawn((Camera3d::default(), bevy::camera::RenderTarget::Image(target.into()),
            Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y)));
        let mut validated = 0;
        for _ in 0..100 {
            app.update();
            let cache = app.sub_app(bevy::render::RenderApp).world().resource::<bevy::render::render_resource::PipelineCache>();
            validated = 0;
            for p in cache.pipelines() {
                if let CachedPipelineState::Err(e) = &p.state { panic!("GPU shader failure: {e}"); }
                if let (CachedPipelineState::Ok(_), PipelineDescriptor::RenderPipelineDescriptor(d)) = (&p.state, &p.descriptor) {
                    if d.fragment.as_ref().is_some_and(|f| f.shader == PROBE_SHADER_HANDLE || f.shader == DECAL_SHADER_HANDLE) { validated += 1; }
                }
            }
            if validated >= 2 { break; }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(validated >= 2, "both authored shaders must actually compile, got {validated}");
    }

    #[test]
    fn authored_mesh_retains_native_uv_color_and_independent_sheared_basis() {
        use bevy::mesh::VertexAttributeValues;
        let packed = [0x00, 0x38, 0x00, 0xbc, 127, 0, 0, 42, 0, 127, 0, 129, 0, 128, 255, 64];
        let linear = [1.0, 2.0, 3.0, 1000.0, 0.0, 1.0, 4.0, 2000.0, 0.0, 0.0, 1.0, 3000.0];
        let mesh = authored_mesh(&[Vec3::ZERO], &[[0.1, 0.2]], &[], &[packed], |v| transformed_direction(&linear, v));
        let f3 = |attribute| match mesh.attribute(attribute).unwrap() { VertexAttributeValues::Float32x3(v) => v[0], _ => panic!("wrong vertex type") };
        assert_eq!(f3(Mesh::ATTRIBUTE_NORMAL), [1.0, 0.0, 0.0]);
        assert_eq!(f3(DH2_BITANGENT), [-3.0, -1.0, 4.0]);
        let VertexAttributeValues::Float32x4(t) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap() else { panic!() };
        assert_eq!(t[0], [2.0, 0.0, -1.0, -1.0]);
        let VertexAttributeValues::Float32x4(color) = mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap() else { panic!() };
        assert_eq!(color[0], [0.0, 128.0 / 255.0, 1.0, 64.0 / 255.0]);
        let VertexAttributeValues::Float32x2(uv1) = mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap() else { panic!() };
        assert_eq!(uv1[0], [0.5, -1.0]);
        // MD6 declarations omit color, not an inferred byte channel.
        let md6_packed: [u8; 12] = packed[..12].try_into().unwrap();
        let md6 = authored_mesh(&[Vec3::ZERO], &[[0.0; 2]], &[], &[md6_packed], |v| v);
        assert!(md6.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
    }

    #[test]
    fn no_argument_launch_prefers_eotw_then_falls_back_to_menu() {
        let temp = env::temp_dir().join(format!(
            "dh2-default-world-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let artifacts = temp.join("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        let executable = temp.join("bin").join("dishonored2-rust.exe");
        let eotw = artifacts.join("eotw_p_lowchaos.bwm");
        let menu = artifacts.join("menu_p.bwm");
        fs::write(&eotw, []).unwrap();
        fs::write(&menu, []).unwrap();

        assert_eq!(default_world_path(&executable), Some(eotw.clone()));
        fs::remove_file(&eotw).unwrap();
        assert_eq!(default_world_path(&executable), Some(menu));
        fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    fn displayed_frame_rate_uses_real_time_even_when_gameplay_time_is_paused() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let mut real = Time::<bevy::time::Real>::default();
        real.advance_by(std::time::Duration::from_millis(1500));
        world.insert_resource(real);
        world.insert_resource(Time::<()>::default());
        world.insert_resource(SceneRenderQueue::default());
        let window = world.spawn(Window::default()).id();
        world.run_system_once(show_frame_rate).unwrap();
        assert_eq!(
            world.get::<Window>(window).unwrap().title,
            "Dishonored 2 Rust | 1 FPS | 0 loaded"
        );
    }

    #[test]
    fn original_local_light_area_threshold_is_32_physical_pixels() {
        let viewport = UVec2::new(800, 600);
        let threshold_range =
            (ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS / (viewport.x * viewport.y) as f32).sqrt();
        let area = projected_local_light_area_pixels(
            Mat4::IDENTITY,
            Vec3::ZERO,
            threshold_range,
            viewport,
        )
        .unwrap();
        assert!((area - ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS).abs() < 0.001);

        let below =
            projected_local_light_area_pixels(Mat4::IDENTITY, Vec3::ZERO, 0.005, viewport).unwrap();
        assert!(below < ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS);

        let crossing_camera_plane =
            Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::new(0.0, 0.0, 1.0, 1.0), Vec4::ZERO);
        assert!(
            projected_local_light_area_pixels(crossing_camera_plane, Vec3::ZERO, 0.1, viewport,)
                .is_none()
        );

        let near_camera_plane =
            Mat4::from_cols(Vec4::X, Vec4::Y, Vec4::Z, Vec4::new(0.0, 0.0, 0.0, 1.0e-8));
        let area =
            projected_local_light_area_pixels(near_camera_plane, Vec3::ZERO, 1.0e-8, viewport)
                .unwrap();
        assert!((area - (viewport.x * viewport.y) as f32).abs() < 1.0);
    }

    #[test]
    fn original_light_proximity_bounds_match_max_visible_range_branches() {
        assert_eq!(
            original_light_proximity_bounds(0.0, None),
            Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS))
        );
        assert_eq!(
            original_light_proximity_bounds(-1.0, None),
            Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS))
        );
        let clip_to_world = Mat4::from_cols(
            Vec4::new(2.0, 0.0, 0.0, 0.0),
            Vec4::new(0.0, 4.0, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 6.0, 0.0),
            Vec4::new(4.0, 5.0, 6.0, 1.0),
        );
        assert_eq!(
            original_light_proximity_bounds(2.0, Some(clip_to_world)),
            Some((Vec3::new(5.0, 7.0, 9.0), 5.0))
        );
        assert_eq!(original_light_proximity_bounds(2.0, None), None);
    }

    #[test]
    fn original_light_projection_bounds_are_retained_before_proximity() {
        let matrix = Mat4::from_scale_rotation_translation(
            Vec3::new(-2.0, 4.0, 6.0),
            Quat::IDENTITY,
            Vec3::new(4.0, 5.0, 6.0),
        );
        let bounds = original_light_projection_bounds(matrix);
        assert_eq!(
            bounds,
            Some((Vec3::new(2.0, 5.0, 6.0), Vec3::new(4.0, 9.0, 12.0)))
        );
        assert_eq!(
            original_light_proximity_from_cached_bounds(2.0, bounds),
            Some((Vec3::new(3.0, 7.0, 9.0), 5.0))
        );
        assert_eq!(
            original_light_projection_bounds(Mat4::from_cols_array(&[0.0; 16])),
            None
        );
        // The initialized fallback is independent of projection availability.
        assert_eq!(
            original_light_proximity_from_cached_bounds(0.0, None),
            Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS))
        );
    }

    #[test]
    fn original_light_clip_to_world_matches_point_and_spot_projection() {
        let view_aligned_game_rotation = [[1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]];
        let point = original_local_light_clip_to_world(
            Vec3::new(1.0, 2.0, 3.0),
            view_aligned_game_rotation,
            entities::LightType::Point,
            2.0,
            None,
        )
        .unwrap()
        .transform_point3(Vec3::ZERO);
        assert!((point - Vec3::new(-1.0, 0.0, 1.0)).length() < 0.0001);

        let cosine = 35.0_f32.to_radians().cos();
        let spot = original_local_light_clip_to_world(
            Vec3::ZERO,
            view_aligned_game_rotation,
            entities::LightType::Spot,
            2.0,
            Some(cosine),
        )
        .unwrap()
        .transform_point3(Vec3::ZERO);
        let expected = -2.0 * 35.0_f32.to_radians().tan();
        assert!((spot - Vec3::new(-1.0, expected, expected)).length() < 0.0001);
    }

    #[test]
    fn original_light_volume_clip_and_proximity_gates_match_bounds() {
        assert_eq!(
            original_light_volume_intersects_camera(Mat4::IDENTITY, Mat4::IDENTITY),
            Some(true)
        );
        assert_eq!(
            original_light_volume_intersects_camera(
                Mat4::IDENTITY,
                Mat4::from_translation(Vec3::new(0.5, 0.0, 0.0)),
            ),
            Some(true)
        );
        assert_eq!(
            original_light_volume_intersects_camera(
                Mat4::IDENTITY,
                Mat4::from_translation(Vec3::new(1.1, 0.0, 0.0)),
            ),
            Some(false)
        );

        assert!(original_light_proximity_pass(
            Vec3::new(32_000.0, 32_000.0, 0.0),
            Vec3::ZERO,
            ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS,
        ));
        assert!(!original_light_proximity_pass(
            Vec3::new(32_768.1, 0.0, 0.0),
            Vec3::ZERO,
            ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS,
        ));
    }

    fn add_culling_test_camera(world: &mut World) {
        let mut camera = Camera::default();
        camera.viewport = Some(default());
        camera.viewport.as_mut().unwrap().physical_size = UVec2::splat(128);
        camera.computed.clip_from_view = Mat4::IDENTITY;
        world.spawn((camera, Camera3d::default(), GlobalTransform::default()));
    }

    fn add_culling_test_light(
        world: &mut World,
        kind: entities::LightType,
        range: f32,
        proximity_bounds: Option<(Vec3, f32)>,
        clip_to_world: Mat4,
    ) -> Entity {
        world
            .spawn((
                OriginalLocalLightCull {
                    range,
                    kind,
                    culling: entities::LightCulling::StaticPosition,
                    definition_name_hash: 0,
                    linear_color: [1.0; 3],
                    projection_bounds: original_light_projection_bounds(clip_to_world),
                    proximity_bounds,
                    initial_sphere: None,
                    clip_to_world: Some(clip_to_world),
                },
                GlobalTransform::default(),
                Visibility::Inherited,
            ))
            .id()
    }

    #[test]
    fn parsed_light_preset_inputs_reach_native_conversion() {
        let document = entities::parse(
            r#"Version 6
entity { entityDef lamp { class = "idLight"; edit = {
    spawnPosition = { x = 1; } m_preset = "LIGHT_PRESET_CANDLE";
} } }"#,
        )
        .unwrap();
        let mut light = entities::placed_lights_with_inheritance(&document, |_| Ok(None))
            .unwrap()
            .remove(0);
        let converted = original_light_preset(&light, [1.0; 3]).unwrap();
        assert_eq!(converted.range, 10.0f32.sqrt());
        assert_eq!(converted.falloff, converted.range * 10.0 / 15.0);
        assert_eq!(converted.color, [5.0 / render_light::normalization(); 3]);
        assert!(original_light_rgb_nonzero(converted.color));
        assert!(!original_light_rgb_nonzero(
            original_light_preset(&light, [0.0; 3]).unwrap().color
        ));
        light.preset = Some("LIGHT_PRESET_100W".into());
        assert_eq!(
            original_light_preset(&light, [1.0; 3])
                .unwrap()
                .normalized_intensity,
            5.0
        );
        assert_eq!(original_light_preset(&light, [1.0; 3]).unwrap().range, 15.0);
        light.preset = Some("LIGHT_PRESET_MOON".into());
        light.distance_cutoff = 1000.0;
        assert_eq!(
            original_light_preset(&light, [1.0; 3]).unwrap().range,
            1000.0
        );
        light.preset = Some("LIGHT_PRESET_REFLECTOR".into());
        assert!(original_light_preset(&light, [1.0; 3]).is_some());
        light.reflector_preset = Some("REFLECTOR_SUN_OFF_50PCT_WALL".into());
        assert!(original_light_preset(&light, [1.0; 3]).is_some());
        light.reflector_preset = Some("UNKNOWN".into());
        assert!(original_light_preset(&light, [1.0; 3]).is_none());
        light.preset = Some("UNKNOWN".into());
        assert!(original_light_preset(&light, [1.0; 3]).is_none());
    }

    #[test]
    fn area_preset_energy_reaches_actual_bevy_extraction_without_double_normalization() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::pbr::{ExtractedRectLight, extract_lights};
        use bevy::render::{MainWorld, sync_world::RenderEntity};

        let mut main_world = MainWorld::default();
        main_world.init_resource::<PointLightShadowMap>();
        main_world.init_resource::<DirectionalLightShadowMap>();
        let mut render_world = World::new();
        let mut expected = Vec::new();
        for conservative in [false, true] {
            for preset in [6, 11] {
                for width in [1.0, 2.0] {
                    let size = [width, 3.0];
                    let output = render_light::convert(render_light::PresetInput {
                        kind: 3,
                        preset,
                        reflector: 0,
                        area_energy_conservative: conservative,
                        cutoff: 15.0,
                        falloff: 10.0,
                        cutoff_cosine: 0.5,
                        back_reflection: 0.0,
                        area_size: size,
                        color: [1.0; 3],
                    })
                    .unwrap();
                    let render_entity = render_world.spawn_empty().id();
                    main_world.spawn((
                        RectLight {
                            color: Color::WHITE,
                            intensity: light_backend_intensity(
                                output.normalized_intensity,
                                entities::LightType::Area,
                                size,
                            )
                            .unwrap(),
                            width: size[0],
                            height: size[1],
                            range: output.range,
                        },
                        GlobalTransform::default(),
                        ViewVisibility::VISIBLE,
                        RenderEntity::from(render_entity),
                    ));
                    expected.push((render_entity, output.normalized_intensity));
                }
            }
        }
        render_world.insert_resource(main_world);
        render_world.run_system_once(extract_lights).unwrap();
        for (entity, native_intensity) in expected {
            let extracted = render_world.get::<ExtractedRectLight>(entity).unwrap();
            let expected = native_intensity * 2_000.0 / std::f32::consts::PI;
            assert!((extracted.intensity / expected - 1.0).abs() < 0.000_001);
        }
    }

    #[test]
    fn backend_light_intensity_rejects_overflow_after_conversion() {
        use entities::LightType;
        for kind in [
            LightType::Point,
            LightType::HalfPoint,
            LightType::Spot,
            LightType::Parallel,
        ] {
            assert!(light_backend_intensity(f32::MAX, kind, [1.0; 2]).is_none());
            assert!(light_backend_intensity(f32::NAN, kind, [1.0; 2]).is_none());
            assert_eq!(light_backend_intensity(5.0, kind, [1.0; 2]), Some(10_000.0));
        }
        assert!(light_backend_intensity(f32::MAX, LightType::Area, [1.0; 2]).is_none());
        assert!(light_backend_intensity(1.0, LightType::Area, [f32::MAX; 2]).is_none());
        assert!(light_backend_intensity(1.0, LightType::Area, [f32::NAN, 1.0]).is_none());
        let output = render_light::convert(render_light::PresetInput {
            kind: 3,
            preset: 6,
            reflector: 0,
            area_energy_conservative: true,
            cutoff: 15.0,
            falloff: 10.0,
            cutoff_cosine: 0.5,
            back_reflection: 0.0,
            area_size: [1e-18; 2],
            color: [1.0; 3],
        })
        .unwrap();
        assert!(output.normalized_intensity.is_finite());
        assert!((output.normalized_intensity * 2_000.0).is_infinite());
        assert!(
            light_backend_intensity(output.normalized_intensity, LightType::Area, [0.01; 2])
                .unwrap()
                .is_finite()
        );
    }

    #[test]
    fn original_light_eligibility_matches_installed_instructions() {
        let mut cases = 0;
        for line in include_str!("../../analysis/original_light_update_cases.txt").lines() {
            let values = line
                .split_whitespace()
                .map(|word| word.parse::<u32>().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 7);
            let color = [values[3], values[4], values[5]].map(f32::from_bits);
            let flags = original_render_light_update_flags(
                values[0] as u8,
                values[1] as u8,
                values[2],
                color,
            );
            assert_eq!(flags, values[6] as u8, "{line}");
            if !original_light_rgb_nonzero(color) {
                assert_eq!(flags & 8, 0);
            }
            cases += 1;
        }
        assert_eq!(cases, 1920);
    }

    #[test]
    fn original_static_light_id_gate_preserves_native_bypasses_and_empty_views() {
        use entities::{LightCulling, LightType};
        for kind in [
            LightType::Point,
            LightType::HalfPoint,
            LightType::Spot,
            LightType::Area,
        ] {
            assert!(original_local_light_id_pass(
                kind,
                LightCulling::StaticPosition,
                42,
                Some(&[1, 42, 42, 100])
            ));
            assert!(!original_local_light_id_pass(
                kind,
                LightCulling::StaticPosition,
                43,
                Some(&[1, 42, 42, 100])
            ));
            assert!(!original_local_light_id_pass(
                kind,
                LightCulling::StaticPosition,
                42,
                Some(&[])
            ));
            assert!(original_local_light_id_pass(
                kind,
                LightCulling::DynamicPosition,
                42,
                Some(&[])
            ));
            assert!(original_local_light_id_pass(
                kind,
                LightCulling::StaticPosition,
                42,
                None
            ));
        }
        assert!(original_local_light_id_pass(
            LightType::Parallel,
            LightCulling::StaticPosition,
            42,
            Some(&[])
        ));
    }

    fn light_test_query_state() -> tome::RenderQueryState {
        let rows =
            std::array::from_fn(|row| std::array::from_fn(|column| f32::from(row == column)));
        tome::RenderQueryState {
            render_rows: rows,
            cropped_rows: rows,
            camera_point: [0.0; 3],
            camera_imprecision: [0.0; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.0,
            depth_grid: None,
        }
    }

    #[test]
    fn world_light_query_preserves_area_then_common_proximity_and_clip_checks() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(OriginalLocalLightCullEnabled(true));
        world.insert_resource(OriginalLocalLightCullStats::default());
        world.insert_resource(SceneRenderQueue {
            visible_authored_ids: vec![0],
            visible_cell_mask: vec![1],
            light_query_state: Some(light_test_query_state()),
            ..default()
        });
        add_culling_test_camera(&mut world);
        let mut lights = Vec::new();
        for bounds in [
            [[0.1, 0.1, 0.2], [0.5, 0.5, 0.6]],
            [[2.0, 0.1, 0.2], [3.0, 0.5, 0.6]],
            [[0.1, 0.1, 0.2], [0.11, 0.11, 0.6]],
        ] {
            let light = add_culling_test_light(
                &mut world,
                entities::LightType::Point,
                10.0,
                Some((Vec3::splat(1.0e6), 0.0)),
                Mat4::IDENTITY,
            );
            let low = game_to_view(bounds[0]);
            let high = game_to_view(bounds[1]);
            world
                .get_mut::<OriginalLocalLightCull>(light)
                .unwrap()
                .projection_bounds = Some((low.min(high), low.max(high)));
            lights.push(light);
        }
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        for &light in &lights {
            assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
        }
        let stats = world.resource::<OriginalLocalLightCullStats>();
        assert!(stats.world_query_bound);
        assert_eq!(stats.world_query_unverified, 0);
        assert_eq!(stats.proximity_culled, 1);
        assert_eq!(stats.clip_culled, 1);
        assert_eq!(stats.area_culled, 1);

        // The common proximity check is required in the Umbra branch too.
        // Its area uses the cached box, NOT the non-Umbra range sphere.
        let mut first = world.get_mut::<OriginalLocalLightCull>(lights[0]).unwrap();
        first.proximity_bounds = Some((Vec3::ZERO, 1.0));
        first.range = 1.0e-9;
        drop(first);
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world.get::<Visibility>(lights[0]),
            Some(&Visibility::Visible)
        );
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .proximity_culled,
            0
        );

        // Missing world-area data must not reuse a previous metric. Known
        // common proximity/clip checks remain valid without that descriptor.
        world.resource_mut::<SceneRenderQueue>().light_query_state = None;
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world.get::<Visibility>(lights[0]),
            Some(&Visibility::Visible)
        );
        for &light in &lights[1..] {
            assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
        }
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .world_query_unverified,
            3
        );
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .proximity_culled,
            2
        );

        world
            .resource_mut::<SceneRenderQueue>()
            .visible_cell_mask
            .clear();
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        for light in lights {
            assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
        }
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .proximity_culled,
            2
        );
        assert_eq!(
            world.resource::<OriginalLocalLightCullStats>().area_culled,
            1
        );
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn light_sphere_batch_rejects_hidden_cells_and_retains_unlocated_sources() {
        use bevy::ecs::system::RunSystemOnce;
        let bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
        let point = [-36.467716, 192.6837, 5.6344576];
        let outside = [1.0e6; 3];
        let cell = query_data
            .point_cell_id(&bytes, &tree, point)
            .unwrap()
            .unwrap();
        assert_eq!(cell, 391);
        assert_eq!(
            query_data.point_cell_id(&bytes, &tree, outside).unwrap(),
            None
        );
        let cell_count = query_data.cell_count();
        let mut world = World::new();
        world.insert_resource(TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&bytes),
            gate_states: tome::initial_gate_states(&bytes).unwrap(),
            bytes,
            tree,
            query_data,
            query_job_count: tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT,
        });
        world.insert_resource(OriginalLocalLightCullEnabled(true));
        world.insert_resource(OriginalLocalLightCullStats::default());
        world.insert_resource(SceneRenderQueue {
            visible_authored_ids: vec![0],
            visible_cell_mask: vec![0; cell_count.div_ceil(32)],
            light_query_state: Some(light_test_query_state()),
            ..default()
        });
        add_culling_test_camera(&mut world);
        let lights = [point, outside].map(|point| {
            let light = add_culling_test_light(
                &mut world,
                entities::LightType::Point,
                0.1,
                None,
                Mat4::IDENTITY,
            );
            world
                .get_mut::<OriginalLocalLightCull>(light)
                .unwrap()
                .initial_sphere = Some([point[0], point[1], point[2], 0.1]);
            light
        });
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world.get::<Visibility>(lights[0]),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            world.get::<Visibility>(lights[1]),
            Some(&Visibility::Visible)
        );
        let stats = world.resource::<OriginalLocalLightCullStats>();
        assert_eq!(stats.sphere_candidates, 2);
        assert_eq!(stats.sphere_culled, 1);
        assert_eq!(stats.sphere_unverified, 0);
        assert_eq!(stats.sphere_error, None);

        // A fresh mask containing the source cell retains its light.
        world.resource_mut::<SceneRenderQueue>().visible_cell_mask[cell as usize / 32] |=
            1 << (cell % 32);
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world.get::<Visibility>(lights[0]),
            Some(&Visibility::Visible)
        );
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .sphere_culled,
            0
        );
    }

    #[test]
    fn preset_rgb_is_diagnostic_not_runtime_light_eligibility() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(OriginalLocalLightCullEnabled(true));
        world.insert_resource(OriginalLocalLightCullStats::default());
        world.insert_resource(SceneRenderQueue {
            visible_authored_ids: vec![0],
            visible_cell_mask: vec![1],
            ..default()
        });
        add_culling_test_camera(&mut world);
        let mut lights = Vec::new();
        for kind in [
            entities::LightType::Point,
            entities::LightType::HalfPoint,
            entities::LightType::Spot,
            entities::LightType::Area,
        ] {
            for color in [[0.0, -0.0, 0.0], [0.0, 1.0, 0.0]] {
                let light = add_culling_test_light(
                    &mut world,
                    kind,
                    10.0,
                    Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS)),
                    Mat4::IDENTITY,
                );
                world
                    .get_mut::<OriginalLocalLightCull>(light)
                    .unwrap()
                    .linear_color = color;
                lights.push(light);
            }
        }
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        for light in &lights {
            assert_eq!(world.get::<Visibility>(*light), Some(&Visibility::Visible));
        }
        let stats = world.resource::<OriginalLocalLightCullStats>();
        assert_eq!(stats.zero_preset_rgb, 4);
        assert_eq!(stats.static_id_culled, 0);
        assert_eq!(stats.proximity_culled, 0);
        assert_eq!(stats.clip_culled, 0);
        assert_eq!(stats.area_culled, 0);

        world
            .resource_mut::<SceneRenderQueue>()
            .visible_cell_mask
            .clear();
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .zero_preset_rgb,
            4
        );
        for light in lights {
            assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
        }
    }

    #[test]
    fn local_light_culler_consumes_current_ids_and_fails_open_on_missing_outputs() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(OriginalLocalLightCullEnabled(true));
        world.insert_resource(OriginalLocalLightCullStats::default());
        world.insert_resource(SceneRenderQueue {
            instances: Vec::new(),
            active: 0,
            tome_render_instances: TomeRenderInstanceMap::default(),
            visible_authored_ids: vec![7],
            visible_cell_indices: vec![0],
            visible_cell_mask: vec![1],
            light_query_state: None,
            visibility_suppression_groups: Vec::new(),
            diffuse_image_demands: Vec::new(),
            diffuse_image_paths: Vec::new(),
            diffuse_image_materials: Vec::new(),
            native_material_images: HashMap::new(),
            diffuse_image_handles: Vec::new(),
            diffuse_image_pending: Vec::new(),
            diffuse_mip_states: Vec::new(),
            diffuse_image_attempted: Vec::new(),
            diffuse_image_last_visible: Vec::new(),
            texture_stream_frame: 0,
            texture_viewport_pixels: 0,
            diffuse_mip_upgrades: 0,
            preferred_mip: None,
        });
        add_culling_test_camera(&mut world);
        let mut handles = Vec::new();
        for (hash, dynamic) in [(7, false), (8, false), (9, true)] {
            let light = add_culling_test_light(
                &mut world,
                entities::LightType::Point,
                10.0,
                Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS)),
                Mat4::IDENTITY,
            );
            let mut component = world.get_mut::<OriginalLocalLightCull>(light).unwrap();
            component.definition_name_hash = hash;
            if dynamic {
                component.culling = entities::LightCulling::DynamicPosition;
            }
            handles.push(light);
        }
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world.get::<Visibility>(handles[0]),
            Some(&Visibility::Visible)
        );
        assert_eq!(
            world.get::<Visibility>(handles[1]),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            world.get::<Visibility>(handles[2]),
            Some(&Visibility::Visible)
        );
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_culled,
            1
        );
        assert!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_binding_active
        );
        // A valid empty visible-ID result rejects static lights, not dynamic lights.
        world
            .resource_mut::<SceneRenderQueue>()
            .visible_authored_ids
            .clear();
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world.get::<Visibility>(handles[0]),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            world.get::<Visibility>(handles[1]),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            world.get::<Visibility>(handles[2]),
            Some(&Visibility::Visible)
        );
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_culled,
            2
        );
        // An unavailable camera/matrix/mapping must not turn cleared IDs into
        // rejections. Native worker errors instead publish fresh empty lists;
        // that connected pipeline policy is tested separately below.
        world
            .resource_mut::<SceneRenderQueue>()
            .visible_cell_mask
            .clear();
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        for light in handles {
            assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
        }
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_culled,
            0
        );
        assert!(
            !world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_binding_active
        );
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn streamed_light_ids_replace_stale_data_on_worker_errors_and_camera_loss() {
        use bevy::ecs::system::RunSystemOnce;
        let bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
        let cell_count = query_data.cell_count();
        let object_count = u32::from_le_bytes(bytes[0x40..0x44].try_into().unwrap()) as usize;
        let mut world = World::new();
        world.insert_resource(TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&bytes),
            gate_states: tome::initial_gate_states(&bytes).unwrap(),
            bytes,
            tree,
            query_data,
            query_job_count: tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT,
        });
        world.insert_resource(PickupState {
            items: Vec::new(),
            world: runtime::WorldState::default(),
        });
        world.insert_resource(SceneRenderQueue {
            // This focused fixture has no BWM bindings; every selected object
            // can therefore contribute its authored user IDs to light lookup.
            tome_render_instances: TomeRenderInstanceMap::from_authored_bwm_mapping(
                &vec![Vec::new(); object_count],
                &[],
            ),
            ..default()
        });
        world.insert_resource(StreamingDiagnostics {
            frame: 0,
            pending: String::new(),
            history: VecDeque::new(),
            log: None,
            log_path: PathBuf::new(),
            status: String::new(),
            previous_status: String::new(),
            previous_active: 0,
            last_logged: Instant::now(),
        });
        world.insert_resource(OriginalLocalLightCullEnabled(true));
        world.insert_resource(OriginalLocalLightCullStats::default());
        let mut camera = Camera::default();
        camera.viewport = Some(default());
        camera.viewport.as_mut().unwrap().physical_size = UVec2::splat(128);
        camera.computed.clip_from_view = Mat4::IDENTITY;
        let point = game_to_view([-36.467716, 192.6837, 5.6344576]);
        let camera_entity = world
            .spawn((
                camera,
                Camera3d::default(),
                Transform::from_translation(point),
                GlobalTransform::from_translation(point),
            ))
            .id();
        world.run_system_once(stream_visible_models).unwrap();
        assert_eq!(world.resource::<StreamingDiagnostics>().status, "ok");
        let id = *world
            .resource::<SceneRenderQueue>()
            .visible_authored_ids
            .first()
            .unwrap();
        let light = add_culling_test_light(
            &mut world,
            entities::LightType::Point,
            10.0,
            Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS)),
            Mat4::from_translation(point),
        );
        world
            .get_mut::<OriginalLocalLightCull>(light)
            .unwrap()
            .definition_name_hash = id;
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_culled,
            0
        );

        // Replay a mixed merge through the same publication stage used by
        // streaming. Failed workers clear their lists; successful siblings
        // still supply fresh IDs/cells to the light gate, never stale seeds.
        let mut jobs = {
            let tome_world = world.resource::<TomeWorld>();
            let pose =
                game_pose_from_view_transform(world.get::<Transform>(camera_entity).unwrap());
            let rows = tome::original_world_visibility_query_matrix(
                80.0,
                128,
                128,
                1.0,
                pose,
                ORIGINAL_DEFAULT_NEAR,
                ORIGINAL_DEFAULT_FAR,
            )
            .unwrap();
            let input = tome::OrdinaryDirectQueryInput {
                camera_point: view_to_game(point),
                render_rows: rows,
                cropped_rows: rows,
                initial_tile_rect: [0, 0, 8, 16],
                camera_imprecision: [0.6; 3],
                object_imprecision: 0.6,
                minimum_area_metric: 25.0 / (128.0 * 128.0),
                global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                winding_flip: tome::query_matrix_winding_flip(rows),
                gate_states: Some(&tome_world.gate_states),
                output_capacity: tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
            };
            run_tome_render_jobs(tome_world, &input)
        };
        jobs[0].result = Err("injected native cleared-worker result".into());
        let merged = tome::merge_render_query_outputs(&jobs).unwrap().unwrap();
        world.resource_scope(|world, mut scene: Mut<SceneRenderQueue>| {
            scene.visible_authored_ids = vec![u32::MAX];
            scene.visible_cell_indices = vec![u32::MAX];
            scene
                .publish_light_query_outputs(world.resource::<TomeWorld>(), &merged)
                .unwrap();
        });
        let scene = world.resource::<SceneRenderQueue>();
        assert!(!scene.visible_authored_ids.is_empty());
        assert_eq!(scene.visible_cell_indices, merged.cell_indices());
        assert!(!scene.visible_authored_ids.contains(&u32::MAX));
        assert_eq!(scene.visible_cell_mask.len(), cell_count.div_ceil(32));
        let id = scene.visible_authored_ids[0];
        world
            .get_mut::<OriginalLocalLightCull>(light)
            .unwrap()
            .definition_name_hash = id;
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_culled,
            0
        );
        assert!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_binding_active
        );

        // An outside-root input is native worker error 5. All workers clear
        // their outputs, so merge publishes fresh empty IDs/cells (not the
        // preceding view and not an unavailable-query fail-open substitute).
        let point = game_to_view([1.0e8, 1.0e8, 1.0e8]);
        *world.get_mut::<Transform>(camera_entity).unwrap() = Transform::from_translation(point);
        *world.get_mut::<GlobalTransform>(camera_entity).unwrap() =
            GlobalTransform::from_translation(point);
        world
            .get_mut::<OriginalLocalLightCull>(light)
            .unwrap()
            .clip_to_world = Some(Mat4::from_translation(point));
        world.run_system_once(stream_visible_models).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .status
                .starts_with("all_jobs_failed:"),
            "{}",
            world.resource::<StreamingDiagnostics>().pending
        );
        let scene = world.resource::<SceneRenderQueue>();
        assert!(scene.visible_authored_ids.is_empty());
        assert!(scene.visible_cell_indices.is_empty());
        assert_eq!(scene.visible_cell_mask.len(), cell_count.div_ceil(32));
        assert!(scene.visible_cell_mask.iter().all(|&word| word == 0));
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Hidden));
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_culled,
            1
        );
        assert!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_binding_active
        );

        // Losing the unique camera has no initialized query descriptor. The
        // cleared mask must not masquerade as a fresh empty native result.
        world.despawn(camera_entity);
        world.run_system_once(stream_visible_models).unwrap();
        assert_eq!(
            world.resource::<StreamingDiagnostics>().status,
            "camera_unavailable"
        );
        let scene = world.resource::<SceneRenderQueue>();
        assert!(scene.visible_authored_ids.is_empty());
        assert!(scene.visible_cell_indices.is_empty());
        assert!(scene.visible_cell_mask.is_empty());
        add_culling_test_camera(&mut world);
        world
            .get_mut::<OriginalLocalLightCull>(light)
            .unwrap()
            .clip_to_world = Some(Mat4::IDENTITY);
        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
        assert_eq!(
            world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_culled,
            0
        );
        assert!(
            !world
                .resource::<OriginalLocalLightCullStats>()
                .static_id_binding_active
        );
    }

    #[test]
    fn local_light_culling_system_applies_each_gate_and_counts_kinds() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.insert_resource(OriginalLocalLightCullEnabled(true));
        world.insert_resource(OriginalLocalLightCullStats::default());
        add_culling_test_camera(&mut world);
        let point = add_culling_test_light(
            &mut world,
            entities::LightType::Point,
            10.0,
            Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS)),
            Mat4::IDENTITY,
        );
        let half_point = add_culling_test_light(
            &mut world,
            entities::LightType::HalfPoint,
            10.0,
            Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS)),
            Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
        );
        let spot = add_culling_test_light(
            &mut world,
            entities::LightType::Spot,
            10.0,
            Some((Vec3::new(50_000.0, 0.0, 0.0), 1.0)),
            Mat4::IDENTITY,
        );
        let area = add_culling_test_light(
            &mut world,
            entities::LightType::Area,
            0.001,
            Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS)),
            Mat4::IDENTITY,
        );
        assert_eq!(
            original_light_volume_intersects_camera(
                Mat4::IDENTITY,
                Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
            ),
            Some(false)
        );
        let mut camera_query = world.query::<&Camera>();
        let camera = camera_query.iter(&world).next().unwrap();
        assert_eq!(camera.physical_viewport_size(), Some(UVec2::splat(128)));
        assert_eq!(camera.clip_from_view(), Mat4::IDENTITY);
        drop(camera_query);

        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();
        assert_eq!(
            world.resource::<OriginalLocalLightCullStats>().clip_culled,
            1
        );

        assert_eq!(world.get::<Visibility>(point), Some(&Visibility::Visible));
        assert_eq!(
            world.get::<Visibility>(half_point),
            Some(&Visibility::Hidden)
        );
        assert_eq!(world.get::<Visibility>(spot), Some(&Visibility::Hidden));
        assert_eq!(world.get::<Visibility>(area), Some(&Visibility::Hidden));
        let stats = world.resource::<OriginalLocalLightCullStats>();
        assert_eq!(stats.total, 4);
        assert_eq!(stats.total_by_kind, [1, 1, 1, 1]);
        assert_eq!(stats.proximity_culled, 1);
        assert_eq!(stats.clip_culled, 1);
        assert_eq!(stats.area_culled, 1);
        assert_eq!(stats.proximity_unverified, 0);
        assert_eq!(stats.source_class_counts, [4, 0]);
        assert_eq!(stats.static_name_hash_matches, 0);
    }

    #[test]
    fn unknown_local_light_proximity_bounds_fail_open_and_are_counted() {
        use bevy::ecs::system::RunSystemOnce;

        let mut world = World::new();
        world.insert_resource(OriginalLocalLightCullEnabled(true));
        world.insert_resource(OriginalLocalLightCullStats::default());
        add_culling_test_camera(&mut world);
        let light = world
            .spawn((
                OriginalLocalLightCull {
                    range: 10.0,
                    kind: entities::LightType::Point,
                    culling: entities::LightCulling::DynamicPosition,
                    definition_name_hash: 0,
                    linear_color: [1.0; 3],
                    projection_bounds: None,
                    proximity_bounds: None,
                    initial_sphere: None,
                    clip_to_world: None,
                },
                GlobalTransform::from(Transform::from_translation(Vec3::new(50_000.0, 0.0, 0.0))),
                Visibility::Inherited,
            ))
            .id();

        world
            .run_system_once(cull_original_small_local_lights)
            .unwrap();

        assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
        let stats = world.resource::<OriginalLocalLightCullStats>();
        assert_eq!(stats.proximity_culled, 0);
        assert_eq!(stats.proximity_unverified, 1);
        assert_eq!(stats.projection_bounds_unverified, 1);
        assert_eq!(stats.source_class_counts, [0, 1]);
    }

    #[test]
    fn local_light_culling_system_fails_open_without_camera_or_when_disabled() {
        use bevy::ecs::system::RunSystemOnce;

        for (enabled, add_camera) in [(true, false), (false, true)] {
            let mut world = World::new();
            world.insert_resource(OriginalLocalLightCullEnabled(enabled));
            world.insert_resource(OriginalLocalLightCullStats::default());
            if add_camera {
                add_culling_test_camera(&mut world);
            }
            let light = add_culling_test_light(
                &mut world,
                entities::LightType::Point,
                0.001,
                Some((Vec3::ZERO, ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS)),
                Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)),
            );

            world
                .run_system_once(cull_original_small_local_lights)
                .unwrap();

            assert_eq!(world.get::<Visibility>(light), Some(&Visibility::Visible));
            let stats = world.resource::<OriginalLocalLightCullStats>();
            assert_eq!(stats.total, 1);
            assert_eq!(stats.proximity_culled, 0);
            assert_eq!(stats.clip_culled, 0);
            assert_eq!(stats.area_culled, 0);
        }
    }

    #[test]
    fn authored_light_angle_thresholds_match_original_cosine_transfer() {
        let (cutoff, falloff) = authored_light_cosines(70.0, 20.0);
        assert!((cutoff - 35.0_f32.to_radians().cos()).abs() < 0.000_001);
        assert!((falloff - 10.0_f32.to_radians().cos()).abs() < 0.000_001);

        let (cutoff, falloff) = authored_light_cosines(20.0, 70.0);
        assert!((cutoff - 10.0_f32.to_radians().cos()).abs() < 0.000_001);
        let expected_falloff = cutoff as f64 + 0.000_152_304_841_3;
        assert!((falloff as f64 - expected_falloff).abs() < 0.000_000_1);
    }

    #[test]
    fn original_light_distance_uses_cutoff_range_and_falloff_start() {
        let (range, falloff_start) = original_light_distance(8.0, 15.0).unwrap();
        assert_eq!(range, 8.0);
        assert!((falloff_start - 7.92).abs() < 0.000_001);
        let (range, falloff_start) = original_light_distance(10.0, 20.0).unwrap();
        assert_eq!(range, 10.0);
        assert!((falloff_start - 9.9).abs() < 0.000_001);
        assert_eq!(original_light_distance(0.0, 0.0), Some((0.1, 0.0)));
        assert_eq!(original_light_distance(f32::NAN, 10.0), None);
    }

    #[test]
    fn deferred_diffuse_reader_loads_installed_bc7_mip() {
        let base = PathBuf::from(r"D:\Programs\Steam\steamapps\common\Dishonored2\base");
        if !base.is_dir() {
            return;
        }
        let catalog = archive::Catalog::open(&base).unwrap();
        let path = catalog
            .paths_containing("dirt_mud_01_dark_d2.bimage7_mip2")
            .into_iter()
            .find(|path| path.ends_with("dirt_mud_01_dark_d2.bimage7_mip2"))
            .expect("installed BC7 mip resource");
        let stem = path.strip_suffix("_mip2").unwrap();
        let inline = catalog.read(stem).unwrap().unwrap();
        let base = texture::compressed_inline_bc7_mip(&inline, 0).unwrap();
        let inline_chain = texture::compressed_inline_bc7_chain(&inline).unwrap();
        assert_eq!((base.width, base.height), (256, 256));
        let image = read_streamed_diffuse_image(&catalog, stem, 2)
            .unwrap()
            .expect("authored mip resource");
        assert_eq!(image.level.resource_mip, Some(2));
        assert_eq!(image.image.texture_descriptor.size.width, 512);
        assert_eq!(image.image.texture_descriptor.size.height, 512);
        assert_eq!(
            image.image.texture_descriptor.mip_level_count,
            1 + inline_chain.mip_count
        );
        assert!(image.image.data.is_none());
        assert_eq!(
            image
                .uploads
                .iter()
                .map(|mip| mip.payload.len())
                .sum::<usize>(),
            262_144 + inline_chain.payload.len()
        );
        let finer = read_streamed_diffuse_at_target(&catalog, stem, TextureReadTarget::Exact(2, texture::CompressedFormat::Bc7Srgb))
            .unwrap()
            .expect("single authored finer mip");
        assert_eq!(finer.uploads.len(), 1);
        assert_eq!(finer.uploads[0].payload.len(), 262_144);
        assert_eq!(finer.image.texture_descriptor.mip_level_count, 1);
    }

    #[test]
    fn visible_diffuse_is_uploaded_then_reduced_after_recorded_interval() {
        let base = PathBuf::from(r"D:\Programs\Steam\steamapps\common\Dishonored2\base");
        if !base.is_dir() {
            return;
        }
        let catalog = archive::Catalog::open(&base).unwrap();
        let path = catalog
            .paths_containing("dirt_mud_01_dark_d2.bimage7_mip2")
            .into_iter()
            .find(|path| path.ends_with("dirt_mud_01_dark_d2.bimage7_mip2"))
            .unwrap();
        let stem = path.strip_suffix("_mip2").unwrap().to_string();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(Assets::<Image>::default());
        let mut materials = Assets::<ProbeMaterial>::default();
        let material = materials.add(ProbeMaterial {
            base: StandardMaterial::default(),
            extension: ProbeExtension {
                sh: Handle::default(),
                surface: surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(&[])),
                diffuse0: None,
                diffuse1: None,
                diffuse2: None,
                bump0: None,
                bump1: None,
                bump2: None,
                gloss0: None,
                gloss1: None,
                gloss2: None,
                metallic0: None,
                metallic1: None,
                metallic2: None,
                mask0: None,
                mask1: None,
                mask2: None,
            },
        });
        app.insert_resource(materials);
        app.insert_resource(Assets::<DecalMaterial>::default());
        app.insert_resource(TextureReadWorker::new(catalog, 1));
        app.insert_resource(SceneRenderQueue {
            instances: vec![RenderInstance {
                mesh: Handle::default(),
                material: MaterialHandle::Probe(material.clone()),
                diffuse_image_slot: Some(0),
                probe_index: 0,
                transform: Transform::IDENTITY,
                bounds: render_bounds(&[Vec3::ZERO, Vec3::ONE]),
                pickup_id: None,
                entity: None,
                visible: true,
            }],
            active: 1,
            tome_render_instances: TomeRenderInstanceMap::default(),
            visible_authored_ids: Vec::new(),
            visible_cell_indices: Vec::new(),
            visible_cell_mask: Vec::new(),
            visibility_suppression_groups: Vec::new(),
            diffuse_image_demands: vec![texture::ProjectedAreaDemand::default()],
            diffuse_image_paths: vec![stem],
            light_query_state: None,
            diffuse_image_materials: vec![vec![(MaterialHandle::Probe(material.clone()), surface_gpu::ImageRole(0))]],
            native_material_images: HashMap::new(),
            diffuse_image_handles: vec![None],
            diffuse_image_pending: vec![None],
            diffuse_mip_states: vec![DiffuseMipState::default()],
            diffuse_image_attempted: vec![false],
            diffuse_image_last_visible: vec![0],
            texture_stream_frame: 0,
            texture_viewport_pixels: 0,
            diffuse_mip_upgrades: 0,
            preferred_mip: None,
        });
        app.add_systems(Update, stream_visible_textures);
        for _ in 0..1000 {
            app.update();
            if app
                .world()
                .resource::<SceneRenderQueue>()
                .diffuse_image_handles[0]
                .is_some()
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let image = app
            .world()
            .resource::<SceneRenderQueue>()
            .diffuse_image_handles[0]
            .clone()
            .expect("visible image staged");
        assert_eq!(
            app.world()
                .resource::<SceneRenderQueue>()
                .diffuse_mip_states[0]
                .resident
                .unwrap()
                .resource_mip,
            None
        );
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&image)
                .unwrap()
                .texture_descriptor
                .size
                .width,
            256
        );
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&image)
                .is_some()
        );
        assert!(
            app.world()
                .resource::<Assets<ProbeMaterial>>()
                .get(&material)
                .unwrap()
                .base
                .base_color_texture
                .is_some()
        );

        {
            let mut queue = app.world_mut().resource_mut::<SceneRenderQueue>();
            assert_eq!(
                queue.diffuse_mip_states[0]
                    .resident
                    .unwrap()
                    .next_separate_suffix,
                Some(2)
            );
            queue.texture_viewport_pixels = 1920 * 1080;
            queue.diffuse_image_demands[0].submit(1.0);
        }
        for _ in 0..1000 {
            app.update();
            if app
                .world()
                .resource::<SceneRenderQueue>()
                .diffuse_mip_states[0]
                .resident
                .is_some_and(|level| level.resource_mip == Some(2))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let finer = app
            .world()
            .resource::<SceneRenderQueue>()
            .diffuse_image_handles[0]
            .clone()
            .expect("finer authored mip staged");
        assert_eq!(
            app.world()
                .resource::<SceneRenderQueue>()
                .diffuse_mip_upgrades,
            1
        );
        assert_ne!(finer.id(), image.id());
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&finer)
                .unwrap()
                .texture_descriptor
                .size
                .width,
            512
        );
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&image)
                .is_none()
        );

        app.world_mut().resource_mut::<SceneRenderQueue>().instances[0].visible = false;
        for _ in 0..=TEXTURE_REDUCE_AFTER_FRAMES {
            app.update();
        }
        let reduced = app
            .world()
            .resource::<SceneRenderQueue>()
            .diffuse_image_handles[0]
            .clone()
            .expect("minimum authored mip remains resident");
        assert_eq!(
            app.world()
                .resource::<SceneRenderQueue>()
                .diffuse_mip_states[0]
                .resident
                .unwrap()
                .resource_mip,
            None
        );
        assert_eq!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&reduced)
                .unwrap()
                .texture_descriptor
                .size
                .width,
            256
        );
        assert!(
            app.world()
                .resource::<Assets<Image>>()
                .get(&finer)
                .is_none()
        );
        assert!(
            app.world()
                .resource::<Assets<ProbeMaterial>>()
                .get(&material)
                .unwrap()
                .base
                .base_color_texture
                .is_some()
        );

        let late_image = Image::new(
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![0; 16],
            TextureFormat::Bc7RgbaUnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        {
            let mut worker = app.world_mut().resource_mut::<TextureReadWorker>();
            worker.inflight[0] = true;
            worker.inflight_count = 1;
            worker.ready.push_back((
                0,
                TextureReadTarget::Initial(Some(2)),
                Ok(Some(StreamedDiffuse {
                    image: late_image,
                    uploads: Vec::new(),
                    level: DiffuseMipLevel {
                        resource_mip: Some(2),
                        next_separate_suffix: Some(1),
                        width: 4,
                        height: 4,
                        format: texture::CompressedFormat::Bc7Srgb,
                    },
                })),
            ));
        }
        app.world_mut()
            .resource_mut::<SceneRenderQueue>()
            .diffuse_image_attempted[0] = true;
        app.update();
        let queue = app.world().resource::<SceneRenderQueue>();
        assert!(queue.diffuse_image_handles[0].is_some());
        assert!(queue.diffuse_image_attempted[0]);
        assert_eq!(
            app.world().resource::<TextureReadWorker>().inflight_count,
            0
        );
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn roof_point_diagnostic_reports_authored_tome_records() {
        let bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        let roof_bounds = tome::object_bounds(&bytes, 28684).unwrap();
        assert_eq!(
            roof_bounds,
            [
                [-53.679977, 132.2928, 42.5],
                [-32.959953, 145.30208, 47.52765],
            ]
        );
        let gate_states = tome::initial_gate_states(&bytes).unwrap();
        let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
        let tome_world = TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&bytes),
            bytes,
            tree,
            gate_states,
            query_data,
            query_job_count: tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT,
        };
        let empty_world = world_mesh::WorldMesh {
            meshes: Vec::new(),
            matrices: Vec::new(),
            visibility: Vec::new(),
            auxiliary_instance_groups: std::array::from_fn(|_| Vec::new()),
            visibility_groups: Vec::new(),
            tome_object_instances: Vec::new(),
            spatial_query_instances: Vec::new(),
            objects: Vec::new(),
        };
        let render_queue = SceneRenderQueue {
            light_query_state: None,
            instances: Vec::new(),
            active: 0,
            tome_render_instances: TomeRenderInstanceMap::default(),
            visible_authored_ids: Vec::new(),
            visible_cell_indices: Vec::new(),
            visible_cell_mask: Vec::new(),
            visibility_suppression_groups: Vec::new(),
            diffuse_image_demands: Vec::new(),
            diffuse_image_paths: Vec::new(),
            diffuse_image_materials: Vec::new(),
            native_material_images: HashMap::new(),
            diffuse_image_handles: Vec::new(),
            diffuse_image_pending: Vec::new(),
            diffuse_mip_states: Vec::new(),
            diffuse_image_attempted: Vec::new(),
            diffuse_image_last_visible: Vec::new(),
            texture_stream_frame: 0,
            texture_viewport_pixels: 0,
            diffuse_mip_upgrades: 0,
            preferred_mip: Some(3),
        };
        let point_leaves = tome_world
            .tree
            .record_leaves_containing_point([-43.0, 138.0, 44.0]);
        let point_text = tome_point_text(
            &tome_world,
            &empty_world,
            &render_queue,
            "eye",
            [-43.0, 138.0, 44.0],
        );
        assert!(point_text.contains(&format!(
            "point-containment prefilter touches {} authored spatial records",
            point_leaves.len()
        )));
        for leaf in point_leaves {
            assert!(point_text.contains(&format!("point-containing spatial record {leaf}")));
        }
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn parallel_tome_dispatch_matches_serial_jobs() {
        let bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
        let gate_states = tome::initial_gate_states(&bytes).unwrap();
        let mut world = TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&bytes),
            bytes,
            tree,
            gate_states,
            query_data,
            query_job_count: 1,
        };
        // User capture frame 1164: dense street, ~5,000 mapped render instances.
        let pose = [
            [-0.72593063, 0.01375436, -0.6876305, -36.467716],
            [-0.68776804, -0.014517557, 0.7257855, 192.6837],
            [4.656613e-10, 0.9998, 0.019998547, 5.6344576],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let rows =
            tome::original_world_visibility_query_matrix(80.0, 1920, 1017, 1.0, pose, 0.01, 7000.0)
                .unwrap();
        for count in [1, 2, 3, 4, 12, 16] {
            world.query_job_count = count;
            let input = tome::OrdinaryDirectQueryInput {
                camera_point: [pose[0][3], pose[1][3], pose[2][3]],
                render_rows: rows,
                cropped_rows: rows,
                initial_tile_rect: [0, 0, 8, 16],
                camera_imprecision: [0.6; 3],
                object_imprecision: 0.6,
                minimum_area_metric: 25.0 / (1920.0 * 1017.0),
                global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                winding_flip: tome::query_matrix_winding_flip(rows),
                gate_states: Some(&world.gate_states),
                output_capacity: tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
            };
            let serial: Vec<_> = (0..count)
                .map(|job| {
                    tome::ordinary_direct_tome_render_job(
                        &world.tree,
                        &world.bytes,
                        &world.query_data,
                        &input,
                        job,
                        count,
                    )
                })
                .collect();
            let started = Instant::now();
            let parallel = run_tome_render_jobs(&world, &input);
            let mut elapsed = started.elapsed();
            let samples = if count == tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT {
                20
            } else {
                1
            };
            for _ in 1..samples {
                let started = Instant::now();
                let sample = run_tome_render_jobs(&world, &input);
                elapsed += started.elapsed();
                for (sample, reference) in sample.iter().zip(&parallel) {
                    let sample = sample.result.as_ref().unwrap().as_ref().unwrap();
                    let reference = reference.result.as_ref().unwrap().as_ref().unwrap();
                    assert_eq!(sample.indices(), reference.indices());
                    assert_eq!(sample.area_metrics(), reference.area_metrics());
                }
            }
            println!(
                "headless dispatch jobs={count} samples={samples} mean_wall_ms={:.3}",
                elapsed.as_secs_f64() * 1000.0 / samples as f64
            );
            assert_eq!(parallel.len(), count as usize);
            for (parallel, serial) in parallel.iter().zip(&serial) {
                assert_eq!(parallel.diagnostics.job_index, serial.diagnostics.job_index);
                assert_eq!(parallel.diagnostics.viewport, serial.diagnostics.viewport);
                let parallel = parallel.result.as_ref().unwrap().as_ref().unwrap();
                let serial = serial.result.as_ref().unwrap().as_ref().unwrap();
                assert_eq!(parallel.indices(), serial.indices());
                assert_eq!(parallel.area_metrics(), serial.area_metrics());
            }
            let merged = tome::merge_render_query_outputs(&parallel)
                .unwrap()
                .unwrap();
            let serial = tome::merge_render_query_outputs(&serial).unwrap().unwrap();
            assert_eq!(merged.indices(), serial.indices());
            assert_eq!(merged.area_metrics(), serial.area_metrics());
            assert_eq!(merged.state, serial.state);
            assert_eq!(merged.state.as_ref().unwrap().viewport, [0, 0, 64, 64]);
            assert_eq!(merged.state.as_ref().unwrap().cropped_rows, rows);
            assert!(!merged.indices().is_empty());
            // Exercise the downstream byte-style visibility map with the
            // duplicate indices preserved across jobs.
            let max_index = *merged.indices().iter().max().unwrap() as usize;
            let mapping: Vec<_> = (0..=max_index).map(|i| vec![i]).collect();
            let map = TomeRenderInstanceMap::from_authored_bwm_mapping(&mapping, &mapping);
            let mut visible = vec![false; mapping.len()];
            map.mark_visible_instances(merged.indices(), &mut visible)
                .unwrap();
            assert!(visible.iter().any(|visible| *visible));
        }
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome and entities fixtures"]
    fn eotw_player_start_direct_tome_query_accepts_authored_objects() {
        let tome_bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        let entity_bytes = fs::read("artifacts/eotw_p_lowchaos.entities")
            .expect("missing local EOTW entities fixture");
        let document = entities::parse(&String::from_utf8_lossy(&entity_bytes)).unwrap();
        let start = entities::player_start(&document).unwrap();
        let camera = view_of(&FocusTarget {
            center: game_to_view(start.position),
            distance: 10.0,
            label: "player start".into(),
            spawn: Some(start),
        });
        let point = view_to_game(camera.translation);
        let rows = tome::original_world_visibility_query_matrix(
            ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES,
            1920,
            1080,
            1.0,
            game_pose_from_view_transform(&camera),
            ORIGINAL_DEFAULT_NEAR,
            ORIGINAL_DEFAULT_FAR,
        )
        .unwrap();
        let gates = tome::initial_gate_states(&tome_bytes).unwrap();
        let input = tome::OrdinaryDirectQueryInput {
            camera_point: point,
            render_rows: rows,
            cropped_rows: rows,
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (1920.0 * 1080.0),
            winding_flip: tome::query_matrix_winding_flip(rows),
            gate_states: Some(&gates),
            output_capacity: 32768,
        };
        let tree = tome::SpatialTree::parse(&tome_bytes).unwrap();
        let output = tome::ordinary_direct_tome_visible_objects(&tree, &tome_bytes, &input)
            .unwrap()
            .expect("player start is inside the authored Tome");
        let mut counts = vec![output.count()];
        for degrees in [90.0_f32, 180.0, 270.0] {
            let mut rotated = camera.clone();
            rotated.rotation = Quat::from_rotation_y(degrees.to_radians()) * camera.rotation;
            let rows = tome::original_world_visibility_query_matrix(
                ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES,
                1920,
                1080,
                1.0,
                game_pose_from_view_transform(&rotated),
                ORIGINAL_DEFAULT_NEAR,
                ORIGINAL_DEFAULT_FAR,
            )
            .unwrap();
            let input = tome::OrdinaryDirectQueryInput {
                render_rows: rows,
                cropped_rows: rows,
                winding_flip: tome::query_matrix_winding_flip(rows),
                ..input
            };
            let result = tome::ordinary_direct_tome_visible_objects(&tree, &tome_bytes, &input);
            counts.push(result.unwrap().unwrap().count());
        }
        // Rewrite query regression counts after native outward pixel coverage.
        // The installed pixel/distance predicates have separate native fixtures.
        assert_eq!(counts, [1596, 1068, 235, 131]);
        assert_eq!(&output.indices()[..4], &[17564, 22092, 22095, 22098]);
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn reported_street_position_tome_query_regression() {
        let bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        let gates = tome::initial_gate_states(&bytes).unwrap();
        let point = [-38.43, 175.76, 4.91];
        let eye = game_to_view(point);
        for direction in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
            let camera = Transform::from_translation(eye).looking_at(eye + direction, Vec3::Y);
            let rows = tome::original_world_visibility_query_matrix(
                ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES,
                1362,
                722,
                1.0,
                game_pose_from_view_transform(&camera),
                ORIGINAL_DEFAULT_NEAR,
                ORIGINAL_DEFAULT_FAR,
            )
            .unwrap();
            let output = tome::ordinary_direct_tome_visible_objects(
                &tree,
                &bytes,
                &tome::OrdinaryDirectQueryInput {
                    camera_point: point,
                    render_rows: rows,
                    cropped_rows: rows,
                    initial_tile_rect: [0, 0, 8, 16],
                    camera_imprecision: [0.6; 3],
                    global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                    object_imprecision: 0.6,
                    minimum_area_metric: 25.0 / (1362.0 * 722.0),
                    winding_flip: tome::query_matrix_winding_flip(rows),
                    gate_states: Some(&gates),
                    output_capacity: 32768,
                },
            )
            .unwrap()
            .expect("reported street position is inside the Tome");
            assert!(
                output.count() > 0,
                "direction {direction:?} returned no world objects"
            );
            if direction == Vec3::X {
                // Installed FUN_140a82710 captures accept 22108 and 17564,
                // but reject the out-of-range alternate tier 22109 here.
                for index in [22108, 17564] {
                    assert!(
                        output.indices().contains(&index),
                        "nearby world object {index} missing"
                    );
                }
                assert!(!output.indices().contains(&22109));
            }
        }
    }

    #[test]
    #[ignore = "requires local original-game EOTW BWM and Tome fixtures"]
    fn authored_distance_tiers_dispatch_far_bwm_surfaces() {
        let world_bytes =
            fs::read("artifacts/eotw_p_lowchaos.bwm").expect("missing local EOTW BWM fixture");
        let tome_bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        let world = world_mesh::parse(&world_bytes).unwrap();
        let mut bwm_instances = vec![Vec::new(); world.visibility.len()];
        let mut lods = Vec::new();
        let mut triangles = Vec::new();
        for object in &world.objects {
            for instances in &mut bwm_instances[object.matrix_start..object.matrix_end] {
                instances.push(lods.len());
                lods.push(object.lod);
                triangles.push(world.meshes[object.mesh_index].indices.len() / 3);
            }
        }
        let mapping = TomeRenderInstanceMap::from_world_mapping(&world, &bwm_instances);
        // Optional capture from the previous executable and installed predicate;
        // these counts describe mapped geometry, not GPU time or draw calls.
        if let Ok(cases) = fs::read_to_string("artifacts/object-distance-render-work-inputs.txt") {
            let mut visible = vec![false; lods.len()];
            for line in cases.lines() {
                let (label, indices) = line.split_once('=').unwrap();
                let indices: Vec<u32> = indices
                    .split(',')
                    .map(|value| value.parse().unwrap())
                    .collect();
                visible.fill(false);
                mapping
                    .mark_visible_instances(&indices, &mut visible)
                    .unwrap();
                let mut surface_count = 0;
                let mut triangle_count = 0;
                for (surface, &visible) in visible.iter().enumerate() {
                    if visible && !(label.starts_with("before") && lods[surface] > 0) {
                        surface_count += 1;
                        triangle_count += triangles[surface];
                    }
                }
                eprintln!(
                    "DISTANCE_RENDER_WORK {label} mapped_surfaces={surface_count} mapped_triangles={triangle_count}"
                );
            }
        }
        let mut far_groups = 0;
        let mut mixed_groups = 0;
        for index in 0..world.tome_object_instances.len() {
            let surfaces = mapping.get(index);
            let has_far = surfaces.iter().any(|&surface| lods[surface] > 0);
            if has_far {
                assert!(
                    tome::object_distance_bounds(&tome_bytes, index as u32)
                        .unwrap()
                        .is_some()
                );
                far_groups += 1;
                mixed_groups += usize::from(surfaces.iter().any(|&surface| lods[surface] == 0));
            }
        }
        eprintln!("AUTHORED_BWM_DISTANCE_GROUPS far={far_groups} mixed={mixed_groups}");
        assert!(far_groups > 0);
        assert_eq!(mixed_groups, 0);
        let roof_bounds = tome::object_distance_bounds(&tome_bytes, 28684)
            .unwrap()
            .unwrap();
        for (point, index) in [
            ([-43.0, 138.0, 48.0], 28684),
            ([-43.0, 138.0, roof_bounds[1][2] + 20.0], 28685),
            ([-43.0, 138.0, 100.0], 28686),
        ] {
            let bounds = tome::object_distance_bounds(&tome_bytes, index)
                .unwrap()
                .unwrap();
            assert_eq!(
                tome::object_distance_rejection_mask(bounds, point, [1.0; 4]),
                0
            );
            let expected = mapping.get(index as usize);
            assert!(!expected.is_empty());
            let mut visible = vec![false; lods.len()];
            mapping
                .mark_visible_instances(&[index], &mut visible)
                .unwrap();
            let actual: Vec<usize> = visible
                .iter()
                .enumerate()
                .filter_map(|(surface, &visible)| visible.then_some(surface))
                .collect();
            let mut expected = expected.to_vec();
            expected.sort_unstable();
            expected.dedup();
            assert_eq!(actual, expected);
            eprintln!(
                "AUTHORED_ROOF_DISTANCE_GROUP index={index} lods={:?}",
                expected
                    .iter()
                    .map(|&surface| lods[surface])
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn bwm_bounds_use_authored_game_coordinates() {
        let record = world_mesh::WorldVisibility {
            bounds: [[-3.0, 4.0, -2.0], [5.0, 9.0, 7.0]],
            umbra_id: 42,
            trailing_field: 0xffff,
        };
        let bounds = authored_world_bounds(&record);
        assert_eq!(Vec3::from(bounds.min()), Vec3::new(-3.0, -2.0, -9.0));
        assert_eq!(Vec3::from(bounds.max()), Vec3::new(5.0, 7.0, -4.0));
    }

    #[test]
    fn unmapped_object_has_no_invented_frustum_visibility() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(PickupState {
            items: Vec::new(),
            world: runtime::WorldState::from_pickups(&[]).unwrap(),
        });
        app.insert_resource(SceneRenderQueue {
            instances: vec![RenderInstance {
                mesh: Handle::default(),
                material: MaterialHandle::Probe(Handle::default()),
                diffuse_image_slot: None,
                probe_index: 0,
                transform: Transform::IDENTITY,
                bounds: render_bounds(&[Vec3::new(-2.0, 0.0, -200.0), Vec3::new(2.0, 3.0, -200.0)]),
                pickup_id: None,
                entity: None,
                visible: false,
            }],
            active: 0,
            tome_render_instances: TomeRenderInstanceMap::default(),
            visible_authored_ids: Vec::new(),
            visible_cell_indices: Vec::new(),
            visible_cell_mask: Vec::new(),
            visibility_suppression_groups: Vec::new(),
            diffuse_image_demands: Vec::new(),
            diffuse_image_paths: Vec::new(),
            diffuse_image_materials: Vec::new(),
            native_material_images: HashMap::new(),
            diffuse_image_handles: Vec::new(),
            diffuse_image_pending: Vec::new(),
            diffuse_mip_states: Vec::new(),
            diffuse_image_attempted: Vec::new(),
            diffuse_image_last_visible: Vec::new(),
            texture_stream_frame: 0,
            texture_viewport_pixels: 0,
            diffuse_mip_upgrades: 0,
            preferred_mip: Some(3),
            light_query_state: None,
        });
        app.add_systems(Update, stream_visible_models);
        for camera_count in [0, 2, 1] {
            let cameras: Vec<_> = (0..camera_count)
                .map(|_| app.world_mut().spawn(Camera3d::default()).id())
                .collect();
            {
                let mut scene = app.world_mut().resource_mut::<SceneRenderQueue>();
                scene.visible_authored_ids = vec![123];
                scene.visible_cell_indices = vec![0];
                scene.visible_cell_mask = vec![1];
                scene.light_query_state = Some(light_test_query_state());
            }
            app.update();
            let scene = app.world().resource::<SceneRenderQueue>();
            assert_eq!(scene.active, 0);
            assert!(scene.visible_authored_ids.is_empty());
            assert!(scene.visible_cell_indices.is_empty());
            assert!(scene.visible_cell_mask.is_empty());
            assert!(scene.light_query_state.is_none());
            for camera in cameras {
                app.world_mut().despawn(camera);
            }
        }
    }

    #[test]
    fn tome_object_mapping_resolves_authored_bwm_indices_to_render_instances() {
        let authored = vec![vec![2, 0, 2], vec![1]];
        let rendered = vec![vec![4, 5], vec![6], vec![7, 8, 9]];
        let map = TomeRenderInstanceMap::from_authored_bwm_mapping(&authored, &rendered);
        assert_eq!(map.get(0), &[7, 8, 9, 4, 5, 7, 8, 9]);
        assert_eq!(map.get(1), &[6]);
        assert!(map.get(2).is_empty());
    }

    #[test]
    fn parent_visibility_suppresses_authored_child_instances() {
        let groups = [RenderSuppressionGroup {
            parent: vec![1],
            children: vec![2, 3],
        }];
        let mut visible = [true, false, true, true];
        RenderSuppressionGroup::apply_all(&groups, &mut visible);
        assert_eq!(visible, [true, false, true, true]);
        visible[1] = true;
        RenderSuppressionGroup::apply_all(&groups, &mut visible);
        assert_eq!(visible, [true, true, false, false]);
    }

    #[test]
    fn successful_tome_results_mark_each_authored_bwm_render_instance() {
        let map = TomeRenderInstanceMap::from_authored_bwm_mapping(
            &[vec![2, 0, 2], vec![], vec![1]],
            &[vec![4], vec![5, 6], vec![7, 8]],
        );
        let mut visible = vec![false; 9];
        map.mark_visible_instances(&[0, 1, 2], &mut visible)
            .unwrap();
        assert_eq!(
            visible,
            [false, false, false, false, true, true, true, true, true]
        );
        assert!(map.mark_visible_instances(&[99], &mut visible).is_err());
        assert!(map.mark_visible_instances(&[0], &mut visible[..8]).is_err());
    }

    #[test]
    #[ignore = "requires installed original image indices/resources"]
    fn installed_image_source_aliases_load_lod_and_constant_maps() {
        let catalog = archive::Catalog::open(std::path::Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base")).unwrap();
        let cases = [
            ("models/environment/vs_map/nature/roots_01/roots_lod2_green_d$nomips", "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_green_d$nomips.tga.bimage7"),
            ("models/environment/vs_map/nature/roots_01/roots_lod2_n.tga$nomips", "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_n.tga$nomips.bimage7"),
            ("models/environment/vs_map/nature/roots_01/roots_lod2_g.tga$nomips", "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_g.tga$nomips.bimage7"),
            ("ipr_constantcolor(0.0745098, 0.0980392, 0.12549, 1)", "generated/image/ipr_constantcolor__0.0745098__0.0980392__0.12549__1_.bimage7"),
            ("ipr_constantcolor(0.501961, 0.501961, 0, 1)", "generated/image/ipr_constantcolor__0.501961__0.501961__0__1_.bimage7"),
            ("ipr_constantcolor(0, 0, 0, 1)", "generated/image/ipr_constantcolor__0__0__0__1_.bimage7"),
            ("ipr_constantcolor(0.6, 0.6, 0.6, 1)", "generated/image/ipr_constantcolor__0.6__0.6__0.6__1_.bimage7"),
        ];
        for (source, expected) in cases {
            let stem = material::compiled_image_stem(&catalog, source).unwrap();
            assert_eq!(stem, expected, "{source}");
            let bytes = catalog.read(&stem).unwrap().unwrap();
            let chain = texture::compressed_inline_chain(&bytes).unwrap();
            assert!(!chain.payload.is_empty(), "{source}");
        }
    }

    #[test]
    fn accepted_tome_area_metrics_reach_mapped_diffuse_images() {
        let map = TomeRenderInstanceMap::from_authored_bwm_mapping(
            &[vec![0, 1], vec![1]],
            &[vec![0], vec![1]],
        );
        let instances = [0, 1].map(|diffuse_image_slot| RenderInstance {
            mesh: Handle::default(),
            material: MaterialHandle::Probe(Handle::default()),
            diffuse_image_slot: Some(diffuse_image_slot),
            probe_index: 0,
            transform: Transform::IDENTITY,
            bounds: render_bounds(&[Vec3::ZERO, Vec3::ONE]),
            pickup_id: None,
            entity: None,
            visible: false,
        });
        let demands = [
            texture::ProjectedAreaDemand::default(),
            texture::ProjectedAreaDemand::default(),
        ];
        assert_eq!(
            map.submit_texture_demands(&[0, 1], &[Some(0.25), Some(0.5)], &instances, &demands,)
                .unwrap(),
            3
        );
        assert_eq!(demands[0].take(), 0.25);
        assert_eq!(demands[1].take(), 0.5);
        assert!(
            map.submit_texture_demands(&[2], &[Some(1.0)], &instances, &demands)
                .is_err()
        );
    }

    #[test]
    fn post_merge_spatial_list_overwrites_visibility_and_clears_area_metrics() {
        let bounds = [
            [[20.0; 3], [21.0; 3]],
            [[0.0, 0.1, 0.2], [0.3, 0.4, 0.5]],
            [[8.0, 0.0, 0.0], [9.0, 1.0, 1.0]],
        ];
        let world = world_mesh::WorldMesh {
            meshes: Vec::new(),
            matrices: vec![[0.0; 12]; 3],
            visibility: bounds
                .map(|bounds| world_mesh::WorldVisibility {
                    bounds,
                    umbra_id: u32::MAX,
                    trailing_field: 0,
                })
                .into(),
            auxiliary_instance_groups: Default::default(),
            visibility_groups: Vec::new(),
            tome_object_instances: vec![vec![0], vec![1], vec![2]],
            spatial_query_instances: vec![2, 1],
            objects: Vec::new(),
        };
        // Authored BWM indices are deliberately not render indices, and the
        // separate spatial-list order is also different from both tables.
        let map = TomeRenderInstanceMap::from_world_mapping(&world, &[vec![2], vec![0], vec![1]]);
        assert_eq!(map.spatial_query_instances[0].bounds, bounds[2]);
        assert_eq!(map.spatial_query_instances[0].render_indices, [1]);
        assert_eq!(map.spatial_query_instances[1].bounds, bounds[1]);
        assert_eq!(map.spatial_query_instances[1].render_indices, [0]);
        assert_eq!(map.zero_metric_instances, HashSet::from([0, 1]));
        let rows = std::array::from_fn(|row| {
            std::array::from_fn(|column| if row == column { 1.0 } else { 0.0 })
        });
        let state = tome::RenderQueryState {
            render_rows: rows,
            cropped_rows: rows,
            camera_point: [0.0; 3],
            camera_imprecision: [0.6; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.0,
            depth_grid: None,
        };
        let mut visible = [false, true, true];
        assert_eq!(
            map.apply_spatial_queries(Some(&state), &mut visible)
                .unwrap(),
            1
        );
        assert_eq!(visible, [true, false, true]);
        assert_eq!(map.apply_spatial_queries(None, &mut visible).unwrap(), 2);
        assert_eq!(visible, [true; 3]);
        assert!(
            map.apply_spatial_queries(Some(&state), &mut visible[..1])
                .is_err()
        );
        let instances = [0, 1, 2].map(|slot| RenderInstance {
            mesh: Handle::default(),
            material: MaterialHandle::Probe(Handle::default()),
            diffuse_image_slot: Some(slot),
            probe_index: 0,
            transform: Transform::IDENTITY,
            bounds: render_bounds(&[Vec3::ZERO, Vec3::ONE]),
            pickup_id: None,
            entity: None,
            visible: false,
        });
        let demands = std::array::from_fn::<_, 3, _>(|_| texture::ProjectedAreaDemand::default());
        assert_eq!(
            map.submit_texture_demands(&[0, 1, 2], &[Some(0.5); 3], &instances, &demands)
                .unwrap(),
            3
        );
        assert_eq!(demands.map(|demand| demand.take()), [0.0, 0.0, 0.5]);
    }

    #[test]
    fn authored_id_dispatch_uses_original_group_emptiness_and_unsigned_sort() {
        let map = TomeRenderInstanceMap::from_authored_bwm_mapping(
            &[vec![0], vec![], vec![1], vec![]],
            &[vec![0], vec![]],
        );
        // Group 2 has authored BWM instances but no instantiated surfaces:
        // this must not be mistaken for the empty-group authored-ID path.
        let mut bytes = vec![0u8; 0x90];
        bytes[0x40..0x44].copy_from_slice(&4u32.to_le_bytes());
        bytes[0x50..0x54].copy_from_slice(&0x80u32.to_le_bytes());
        for (index, id) in [11u32, 0x80000001, 22, 1].into_iter().enumerate() {
            bytes[0x80 + index * 4..0x84 + index * 4].copy_from_slice(&id.to_le_bytes());
        }
        assert_eq!(
            map.visible_authored_ids(&bytes, &[3, 2, 1, 0, 3]).unwrap(),
            [1, 1, 0x80000001],
        );
        assert!(map.visible_authored_ids(&bytes, &[4]).is_err());
    }

    #[test]
    fn sightline_identifies_bwm_surface_and_instance() {
        let world = world_mesh::WorldMesh {
            meshes: vec![world_mesh::Mesh {
                positions: vec![[-1.0, 2.0, -1.0], [1.0, 2.0, -1.0], [0.0, 2.0, 1.0]],
                uvs: vec![],
                packed_channels: vec![],
                indices: vec![0, 1, 2],
            }],
            matrices: vec![[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]],
            visibility: vec![world_mesh::WorldVisibility {
                bounds: [[-1.0, 2.0, -1.0], [1.0, 2.0, 1.0]],
                umbra_id: 0,
                trailing_field: 0,
            }],
            tome_object_instances: vec![vec![0]],
            auxiliary_instance_groups: std::array::from_fn(|_| Vec::new()),
            visibility_groups: vec![],
            spatial_query_instances: vec![],
            objects: vec![world_mesh::WorldObject {
                mesh_index: 0,
                material_path: "sample_roof".into(),
                matrix_start: 0,
                matrix_end: 1,
                lod: 0,
            }],
        };
        let (distance, mesh, matrix, material) =
            sightline_world_surface(&world, Vec3::ZERO, Vec3::NEG_Z, 10.0).unwrap();
        assert!((distance - 2.0).abs() < 0.001);
        assert_eq!((mesh, matrix, material.as_str()), (0, 0, "sample_roof"));
    }

    #[test]
    #[ignore = "requires local original-game EOTW Tome fixture"]
    fn reported_street_query_accepts_architecture_railing_and_sky() {
        let bytes =
            fs::read("artifacts/eotw_p_lowchaos.tome").expect("missing local EOTW Tome fixture");
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        let gates = tome::initial_gate_states(&bytes).unwrap();
        let point = [-38.43, 175.76, 4.91];
        let camera = Transform::from_translation(game_to_view(point))
            .looking_at(game_to_view([-40.9, 169.7, 3.8]), Vec3::Y);
        let rows = tome::original_world_visibility_query_matrix(
            ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES,
            1362,
            722,
            1.0,
            game_pose_from_view_transform(&camera),
            ORIGINAL_DEFAULT_NEAR,
            ORIGINAL_DEFAULT_FAR,
        )
        .unwrap();
        assert!(tome::query_matrix_winding_flip(rows));
        let output = tome::ordinary_direct_tome_visible_objects(
            &tree,
            &bytes,
            &tome::OrdinaryDirectQueryInput {
                camera_point: point,
                render_rows: rows,
                cropped_rows: rows,
                initial_tile_rect: [0, 0, 8, 16],
                camera_imprecision: [0.6; 3],
                global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                object_imprecision: 0.6,
                minimum_area_metric: 25.0 / (1362.0 * 722.0),
                winding_flip: tome::query_matrix_winding_flip(rows),
                gate_states: Some(&gates),
                output_capacity: 32768,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(output.count(), 255);
        for index in [10, 17113, 17184] {
            assert!(
                output.indices().contains(&index),
                "missing authored Tome group {index}"
            );
        }
        // The installed distance predicate rejects this alternate tier here.
        assert!(!output.indices().contains(&17581));
    }

    #[test]
    fn shared_mesh_transform_matches_baked_world_vertex() {
        let matrix = [0.0, -1.0, 0.0, 3.0, 1.0, 0.0, 0.0, 4.0, 0.0, 0.0, 1.0, 5.0];
        let vertex = [2.0, 1.0, 3.0];
        let actual = game_affine_to_view(&matrix)
            .to_matrix()
            .transform_point3(game_to_view(vertex));
        let expected = transformed(&matrix, vertex);
        assert!(
            actual.distance(expected) < 1e-5,
            "{actual:?} vs {expected:?}"
        );
    }

    #[test]
    fn placed_model_queries_select_separate_instances_without_bwm_ids() {
        let rows = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.01, 1.0],
            [0.0, 0.0, -0.101, 0.0],
        ];
        let state = tome::RenderQueryState {
            render_rows: rows,
            cropped_rows: rows,
            camera_point: [0.0; 3],
            camera_imprecision: [0.6; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.0,
            depth_grid: None,
        };
        // An accepted matrix1/init1/imprecision0.6 box from native_light_aabb_cases.txt.
        let bounds = [[-5.0, -4.0, -3.5], [1.0, 2.0, 2.5]];
        let group = PlacedModelQueryGroup {
            indices: 2..4,
            input: render_model::ModelQueryInput {
                frame_flags: 5,
                shape_flags: 0,
                bounds,
                region: render_model::range_region(bounds, 40.0),
            },
        };
        let mut visible = vec![true, false, false, false];
        assert_eq!(
            apply_placed_model_queries(&mut visible, &[group], &state, [0.0; 3], [1280, 720])
                .unwrap(),
            1
        );
        assert_eq!(visible, [true, false, true, true]);
        let outside = PlacedModelQueryGroup {
            indices: 2..4,
            input: render_model::ModelQueryInput {
                frame_flags: 5,
                shape_flags: 0,
                bounds,
                region: [100.0, 0.0, 0.0, 1.0],
            },
        };
        let mut visible = vec![true, false, false, false];
        assert_eq!(
            apply_placed_model_queries(&mut visible, &[outside], &state, [0.0; 3], [1280, 720])
                .unwrap(),
            0
        );
        assert_eq!(visible, [true, false, false, false]);
        let invalid = PlacedModelQueryGroup {
            indices: 2..5,
            input: render_model::ModelQueryInput {
                frame_flags: 5,
                shape_flags: 0,
                bounds,
                region: [0.0, 0.0, 0.0, 32768.0],
            },
        };
        assert!(
            apply_placed_model_queries(&mut visible, &[invalid], &state, [0.0; 3], [1280, 720])
                .is_err()
        );
        assert_eq!(visible, [true, false, false, false]);
    }

    #[test]
    fn initial_model_query_preserves_native_transform_and_constructor_range() {
        let mut placement = entities::PlacedModel {
            id: "range".into(),
            resource_path: String::new(),
            position: [3.0, 4.0, 5.0],
            scale: [-2.0, 3.0, 1.0],
            rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            custom_materials: Vec::new(),
            max_visible_range: None,
        };
        let bounds = [[-1.0; 3], [1.0; 3]];
        let default_input = initial_placed_model_query(&placement, bounds).unwrap();
        assert_eq!(default_input.bounds, [[0.0, 2.0, 4.0], [6.0, 6.0, 6.0]]);
        assert_eq!(default_input.region, [0.0, 0.0, 0.0, 32768.0]);
        assert!(render_model::model_eligible(default_input.frame_flags, u8::MAX));
        placement.max_visible_range = Some(40.0);
        let input = initial_placed_model_query(&placement, bounds).unwrap();
        assert_eq!(input.bounds, [[0.0, 2.0, 4.0], [6.0, 6.0, 6.0]]);
        assert_eq!(input.region, [3.0, 4.0, 5.0, 43.0]);
        assert!(render_model::model_eligible(input.frame_flags, u8::MAX));
    }

    #[test]
    fn placed_model_orientation_uses_authored_axes_not_rows() {
        let placement = entities::PlacedModel {
            id: "rotated".into(),
            resource_path: String::new(),
            position: [3.0, 4.0, 5.0],
            scale: [2.0, 3.0, 1.0],
            // Local X points along the authored first axis (+Y), while
            // local Y points along the second axis (-X).
            rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            custom_materials: Vec::new(),
            max_visible_range: None,
        };
        let local_x = [1.0, 0.0, 0.0];
        let expected = game_to_view([3.0, 6.0, 5.0]);
        assert!(placed_vertex(&placement, local_x).distance(expected) < 1e-6);
        assert!(
            placed_transform(&placement)
                .to_matrix()
                .transform_point3(game_to_view(local_x))
                .distance(expected)
                < 1e-5
        );
    }

    #[test]
    fn sheared_world_instance_keeps_exact_vertices() {
        let matrix = [1.0, 0.3, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let source = [[1.0, 1.0, 0.0], [2.0, 0.0, 0.0]];
        let exact: Vec<_> = source
            .iter()
            .copied()
            .map(|point| transformed(&matrix, point))
            .collect();
        assert!(needs_exact_world_mesh(
            &game_affine_to_view(&matrix),
            &source,
            &exact
        ));
        assert_eq!(exact[0], game_to_view([1.3, 1.0, 0.0]));
    }

    #[test]
    fn placed_havok_uses_baked_scale_and_instance_rotation() {
        let placement = entities::PlacedModel {
            id: "fence".into(),
            resource_path: "generated/model/models/fence.bmodel".into(),
            position: [2.0, 3.0, 4.0],
            scale: [1.2, 1.0, -1.0],
            rotation: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            custom_materials: Vec::new(),
            max_visible_range: None,
        };
        assert_eq!(
            model_havok_path(&placement, "hkt", false).as_deref(),
            Some("generated/models/fence.lwo.hkt._120_100_-100.10.hkx")
        );
        assert_eq!(
            placed_collision_vertex(&placement, [1.2, 0.0, 0.0]),
            game_to_view([2.0, 4.2, 4.0])
        );
    }

    #[test]
    fn blueprint_havok_recipe_keeps_root_source_and_declared_kind() {
        let placement = entities::PlacedModel {
            id: "root".into(),
            resource_path: "generated/decls/blueprint/models/fence.blueprint.decl".into(),
            position: [0.0; 3], scale: [1.5, -0.015, 1.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            custom_materials: Vec::new(), max_visible_range: None,
        };
        assert_eq!(model_havok_path(&placement, "hkt", false).as_deref(),
            Some("generated/models/fence.blueprint.hkt._150_-1_100.10.hkx"));
        assert_eq!(model_havok_path(&placement, "box", false).as_deref(),
            Some("generated/models/fence.blueprint.box._150_-1_100.10.hkx"));
        assert_eq!(model_havok_path(&placement, "cvd", false).as_deref(),
            Some("generated/models/fence.blueprint.cvd._150_-1_100.10.hkx"));
        assert_eq!(model_havok_path(&placement, "hkt", true).as_deref(),
            Some("generated/models/fence_hd.blueprint.hkt._150_-1_100.10.hkx"));
        assert_eq!(model_havok_path(&placement, "unknown", false), None);
    }

    #[test]
    #[ignore = "requires installed catalog and artifacts/eotw_p_lowchaos.entities"]
    fn blueprint_root_collision_uses_component_not_render_children() {
        let document = entities::parse(&fs::read_to_string("artifacts/eotw_p_lowchaos.entities").unwrap()).unwrap();
        let bodies = entities::rigid_body_settings(&document);
        let parent = entities::render_placements(&document).into_iter()
            .find(|placement| placement.id == "env_bp_fence_metal_200cm_02").unwrap();
        let catalog = archive::Catalog::open(&PathBuf::from(
            r"D:\Programs\Steam\steamapps\common\Dishonored2\base")).unwrap();
        assert_eq!(model_havok_path(&parent, "hkt", false).as_deref(),
            Some("generated/models/environment/props/karnaca/fence_metal_set_01/bp_fence_metal_200cm_02.blueprint.hkt._100_100_100.10.hkx"));
        let placements = expanded_model_placements(&[parent], &catalog);
        assert_eq!(placements.len(), 10);
        let (triangles, owners, matched, failed) = static_model_collision(&catalog, &placements, &bodies);
        assert_eq!((matched, failed, triangles.len()), (1, 0, 312));
        assert_eq!(owners, ["env_bp_fence_metal_200cm_02"]);
        assert!(triangles.iter().all(|(vertices, owner)| *owner == 0
            && vertices.iter().all(|point| point.is_finite())));
        assert_eq!(static_model_collision(&catalog, &placements, &[]).0.len(), 0);
        let mut unbound_defaults = bodies.clone();
        for body in &mut unbound_defaults { body.physics_behavior = None; }
        assert_eq!(static_model_collision(&catalog, &placements, &unbound_defaults).0.len(), 0);
        let world = navigation::CollisionWorld::with_owned(triangles, owners);
        let eye = game_to_view([-33.679825, 83.83279, 18.319]);
        let direction = game_to_view([0.5484812, 0.29665253, -0.78177094]).normalize();
        let (distance, owner, _) = world.first_hit_details(eye, eye + direction * 40.0).unwrap();
        assert_eq!(world.owner_name(owner), Some("env_bp_fence_metal_200cm_02"));
        assert!((distance - 0.28008184).abs() < 0.00001);
    }

    #[test]
    fn pickup_selection_rejects_wall_occlusion() {
        let definitions = [entities::Pickup {
            entity_id: "test_item".into(),
            position: [0.0, 0.0, 0.0],
            item_path: "test/inventoryitem".into(),
            quantity: 1,
        }];
        let state = PickupState {
            items: vec![PickupInstance {
                entity_id: "test_item".into(),
                position: Vec3::new(0.0, 1.0, -2.0),
            }],
            world: runtime::WorldState::from_pickups(&definitions).unwrap(),
        };
        let camera = Transform::from_xyz(0.0, 1.0, 0.0);
        let clear = navigation::CollisionWorld::new(std::iter::empty());
        assert_eq!(pickup_in_view(&camera, &state, Some(&clear)), Some(0));
        let wall = navigation::CollisionWorld::new([
            [
                Vec3::new(-1.0, 0.0, -1.0),
                Vec3::new(1.0, 0.0, -1.0),
                Vec3::new(-1.0, 2.0, -1.0),
            ],
            [
                Vec3::new(1.0, 0.0, -1.0),
                Vec3::new(1.0, 2.0, -1.0),
                Vec3::new(-1.0, 2.0, -1.0),
            ],
        ]);
        assert_eq!(pickup_in_view(&camera, &state, Some(&wall)), None);
    }

    #[test]
    fn mouse_capture_look_and_mode_toggle_reach_camera_system() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ButtonInput::<KeyCode>::default());
        app.insert_resource(ButtonInput::<MouseButton>::default());
        app.insert_resource(AccumulatedMouseMotion {
            delta: Vec2::new(100.0, 0.0),
        });
        app.insert_resource(FocusTargets {
            targets: vec![FocusTarget {
                center: Vec3::ZERO,
                distance: 10.0,
                label: "test".into(),
                spawn: None,
            }],
            active: 0,
        });
        app.insert_resource(NavigationState {
            walking: false,
            walker: navigation::Walker {
                feet: Vec3::ZERO,
                vertical_speed: 0.0,
                supported: false,
            },
        });
        app.insert_resource(NavigationWorld(navigation::CollisionWorld::new(
            std::iter::empty(),
        )));
        let cursor = app.world_mut().spawn(CursorOptions::default()).id();
        let camera = app
            .world_mut()
            .spawn((Camera3d::default(), Transform::default()))
            .id();
        app.add_systems(Update, camera_controls);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyG);
        app.update();
        let camera = app.world().entity(camera).get::<Transform>().unwrap();
        assert!(
            camera.forward().x > 0.1,
            "mouse delta did not rotate camera: {camera:?}"
        );
        assert!(app.world().resource::<NavigationState>().walking);
        let cursor = app.world().entity(cursor).get::<CursorOptions>().unwrap();
        assert_eq!(cursor.grab_mode, CursorGrabMode::Locked);
        assert!(!cursor.visible);
        {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.release(KeyCode::KeyG);
            keys.clear();
            keys.press(KeyCode::KeyG);
        }
        app.update();
        assert!(!app.world().resource::<NavigationState>().walking);
    }
}
