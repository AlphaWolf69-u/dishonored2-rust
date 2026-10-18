//! Dishonored 2 Rust game client using assets from a local installation.
#[path = "../archive.rs"]
mod archive;
#[path = "../bmd6.rs"]
mod bmd6;
#[path = "../edge_skeleton.rs"]
mod edge_skeleton;
#[path = "../bmodel.rs"]
mod bmodel;
#[path = "../bprobe.rs"]
mod bprobe;
#[path = "../entities.rs"]
mod entities;
#[path = "../frame_profile.rs"]
mod frame_profile;
#[path = "../game_clock.rs"]
mod game_clock;
#[path = "../game_clock_runtime.rs"]
mod game_clock_runtime;
#[path = "../material.rs"]
mod material;
#[path = "../material_surface.rs"]
mod material_surface;
#[path = "../native_coarse_depth.rs"]
mod native_coarse_depth;
#[path = "../native_camera.rs"]
mod native_camera;
mod native_shadow_live;
mod native_local_shadow_live;
#[path="../native_local_shadow_cache.rs"]
mod native_local_shadow_cache;
#[path="../native_shadow_depth_copy.rs"]
mod native_shadow_depth_copy;
#[path = "../native_screen_shadows.rs"]
mod native_screen_shadows;
#[path = "../native_screen_shadow_scene.rs"]
mod native_screen_shadow_scene;
#[path = "../native_shadow_gpu.rs"]
mod native_shadow_gpu;
#[path = "../native_shadow_sampling.rs"]
mod native_shadow_sampling;
#[path = "../native_shadow_scene.rs"]
mod native_shadow_scene;
#[path = "../navigation.rs"]
mod navigation;
#[path = "../original_settings.rs"]
mod original_settings;
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
#[path = "../render_program.rs"]
mod render_program;
#[path = "../render_resource.rs"]
mod render_resource;
#[path = "../render_shadow.rs"]
mod render_shadow;
#[path = "../render_state.rs"]
mod render_state;
#[path = "../runtime.rs"]
mod runtime;
#[path = "../shader_cache.rs"]
mod shader_cache;
#[path = "../shadow_receiver.rs"]
mod shadow_receiver;
#[path = "../surface_gpu.rs"]
mod surface_gpu;
#[path = "../texture.rs"]
mod texture;
#[path = "../tiled_lighting.rs"]
mod tiled_lighting;
#[path = "../tiled_lighting_gpu.rs"]
mod tiled_lighting_gpu;
#[path = "../tome.rs"]
mod tome;
#[path = "../world_mesh.rs"]
mod world_mesh;
#[path = "../native_local_world_query.rs"]
mod native_local_world_query;
#[path="../native_task_queue.rs"]
mod native_task_queue;

use bevy::app::AppExit;
use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::camera::{PerspectiveProjection, Projection};
use bevy::core_pipeline::prepass::DepthPrepass;
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
    SpecializedMeshPipelineError, TexelCopyBufferLayout, TextureDimension, TextureFormat,
    VertexFormat,
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
    #[allow(dead_code)] // Preserved for the not-yet-ported fog, water, sky, and postfx passes.
    environment_render_parameters: Option<entities::EnvironmentRenderParameters>,
    environment_probes: Option<probe_lighting::ProbeLightingSet>,
    #[allow(dead_code)]
    // Actual startup inputs for the native CPU frame publisher, not captured defaults.
    shadow_quality_preset: Option<u32>,
    #[allow(dead_code)]
    shadow_quality: Option<entities::ShadowQualitySettings>,
    #[allow(dead_code)] // Actual startup target specification; not a fabricated Ready frame.
    shadow_target_dimensions: Option<native_shadow_scene::NativeParallelShadowTargetDimensions>,
    #[allow(dead_code)] // Original r_depthBiasClamp CVar used by native hardware shadow bias.
    shadow_depth_bias_clamp: Option<f32>,
    shadow_sort_ccw: Option<bool>,
    shadow_cascade_controls: Option<original_settings::ShadowCascadeControls>,
    #[allow(dead_code)] // Parsed runtime CVar; used by the pending live cascade publisher.
    shadow_cascade_fars: Option<[f32; 4]>,
    #[allow(dead_code)] // Authored whole-view receiver bounds, not visible-mesh bounds.
    far_shadow_bounds: Option<[[f32; 3]; 2]>,
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
struct NativeShadowCasterWorld(tome::shadow_caster::ShadowCasterTome);

#[derive(Resource)]
struct NativeViewSettings(Result<original_settings::ViewSettings,String>);

#[derive(Resource)]
struct NavigationState {
    walking: bool,
    walker: navigation::Walker,
}

#[derive(Component)]
struct CoordinateOverlay;

#[derive(Component)]
struct FrameRateOverlay;

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
struct BenchmarkSeconds(f32, f32);

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
    world_query_depth_grid_bound: bool,
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
    trace_texture_streaming: bool,
    texture_events_pending: VecDeque<String>,
    texture_event_history: VecDeque<String>,
    texture_events_dropped: usize,
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
        let trace_texture_streaming = env::var("DH2_TRACE_TEXTURE_STREAMING").as_deref() == Ok("1");
        if trace_texture_streaming {
            println!(
                "Detailed texture-stream events enabled: request/read/upload transitions go to the streaming log and F8 history"
            );
        }
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
            trace_texture_streaming,
            texture_events_pending: VecDeque::new(),
            texture_event_history: VecDeque::with_capacity(256),
            texture_events_dropped: 0,
        }
    }

    fn texture_event(&mut self, message: impl Into<String>) {
        if !self.trace_texture_streaming {
            return;
        }
        let event = format!("frame={} {}", self.frame, message.into());
        if self.texture_event_history.len() == 256 {
            self.texture_event_history.pop_front();
        }
        self.texture_event_history.push_back(event.clone());
        if self.log.is_some() {
            if self.texture_events_pending.len() == 2048 {
                self.texture_events_pending.pop_front();
                self.texture_events_dropped += 1;
            }
            self.texture_events_pending.push_back(event);
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
            let texture_events = self
                .texture_events_pending
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let texture_events_dropped = self.texture_events_dropped;
            let write_result = self.log.as_mut().map(|log| {
                writeln!(log, "{sample}")?;
                if texture_events_dropped > 0 {
                    writeln!(
                        log,
                        "TEXTURE_STREAM_EVENTS_DROPPED {texture_events_dropped}"
                    )?;
                }
                for event in &texture_events {
                    writeln!(log, "TEXTURE_STREAM {event}")?;
                }
                log.flush()
            });
            match write_result {
                Some(Err(error)) => {
                    eprintln!(
                        "Streaming log {}: {error}; continuing in-memory capture",
                        self.log_path.display()
                    );
                    self.log = None;
                    self.texture_events_pending.clear();
                    self.texture_events_dropped = 0;
                }
                Some(Ok(())) => {
                    self.texture_events_pending.clear();
                    self.texture_events_dropped = 0;
                }
                None => {}
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

#[cfg(test)]
mod texture_streaming_diagnostics_tests {
    use super::StreamingDiagnostics;
    use std::{
        collections::VecDeque,
        fs,
        path::PathBuf,
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn opt_in_texture_events_reach_session_log_and_recent_history() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "dh2-streaming-diagnostics-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let log_path = directory.join("streaming.log");
        let mut diagnostics = StreamingDiagnostics {
            frame: 7,
            pending: String::new(),
            history: VecDeque::new(),
            log: Some(fs::File::create(&log_path).unwrap()),
            log_path: PathBuf::from(&log_path),
            status: String::new(),
            previous_status: String::new(),
            previous_active: 0,
            last_logged: Instant::now(),
            trace_texture_streaming: true,
            texture_events_pending: VecDeque::new(),
            texture_event_history: VecDeque::new(),
            texture_events_dropped: 0,
        };
        diagnostics.texture_event("read_requested slot=4 path=\"texture.dds\"".to_owned());
        diagnostics.record("frame=7 summary".to_owned(), 1, false);
        let log = fs::read_to_string(&log_path).unwrap();
        assert!(log.contains("TEXTURE_STREAM frame=7 read_requested slot=4"));
        assert_eq!(diagnostics.texture_event_history.len(), 1);
        assert!(diagnostics.texture_events_pending.is_empty());
        drop(diagnostics);
        fs::remove_dir_all(directory).unwrap();
    }
}

/// Original asset association survives Bevy mesh conversion, including shear
/// baked into world-space vertices. This is provenance, not a guessed native
/// shadow program bank. Native shadow programs consume the retained bind
/// inputs and the same owner rest palette used by visible geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderModelSurface {
    World {
        object: usize,
        matrix: usize,
    },
    Bmodel {
        placement: usize,
        surface: usize,
    },
    Bmd6 {
        placement: usize,
        surface: usize,
        cpu_vertex_declaration: u32,
        gpu_vertex_declaration: u32,
        skin_influences: u8,
        matrix_offset: u32,
    },
}

/// The exact expanded placements used to enqueue models, including rigid-body
/// additions. Model source indices refer here, NOT SceneData.placed_models.
/// These are source associations/initial poses, not current animated poses.
#[derive(Resource)]
struct RenderSourcePlacements(Vec<entities::PlacedModel>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RenderInstanceSource {
    surface: RenderModelSurface,
    world_baked: bool,
}

struct RenderInstance {
    source: Option<RenderInstanceSource>,
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
const PROBE_PREPASS_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("f78172ec-62f2-4bb4-9e45-c6e99fdf6c36");
const SURFACE_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("b7018b4d-b81d-44c0-9dd0-0b64cc879316");
const DECAL_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("9165b294-a09d-46cb-af56-27edbe334a1b");
const INDIRECT_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("8257a42a-79cb-431e-862a-d5c8380347b5");
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
    native_vertex_lighting: bool,
}

/// Resolved once beside the real material/mesh association; the native CPU
/// shadow owner consumes this rather than reparsing declarations every frame.
#[derive(Clone, Debug)]
struct NativeShadowMaterialInputs {
    source: material::ShadowSourceInputs,
    /// Actual slot-10 row-zero source identification. This is not the complete
    /// native pointer bank, draw-state override, or a ready shadow frame.
    rigid_base_source:
        Result<Option<std::sync::Arc<shader_cache::CachedRigidShadowSource>>, String>,
    /// Native material +0x48 bit0 tests bank0 slot10 (+0xf0), regardless
    /// of whether that program's rigid ABI has been ported. An unsupported
    /// shader-class error must not be mistaken for an absent native pointer.
    base_slot10_present: Result<bool, String>,
    /// Actual bank2 slot10 (+0x250), not a rigid-source substitute.
    world_source: Result<Option<std::sync::Arc<shader_cache::CachedWorldShadowSource>>, String>,
    world_parameters: Result<Option<NativeShadowMaterialParameters>, String>,
    main_program: Option<String>,
    alpha_test: bool,
    world_uv_set: Option<u32>,
    diffuse_image_slot: Option<usize>,
    parameters: Result<Option<NativeShadowMaterialParameters>, String>,
    cull_mode: Option<bevy::render::render_resource::Face>,
}

impl NativeShadowMaterialInputs {
    /// Only the two original material flags consumed by FUN_1402996f0.
    /// Slot presence comes from +0xf0; needsWind uses the MAIN fallback
    /// (1402d565a..1402d567e), not the specialized shadow shader's defaults.
    fn world_gather_flags48(&self) -> Result<u8, String> {
        Ok(u8::from(self.base_slot10_present.clone()?) | (u8::from(self.source.needs_wind) << 6))
    }
}

/// Literal material overrides and registered defaults remain separate from the
/// draw owner's optional fallback. In particular, neither a forward material's
/// UV values nor guessed wind/clock values become native shadow constants.
#[derive(Clone, Debug)]
struct NativeShadowMaterialParameters {
    overrides: render_program::SerializedParameterBlock,
    defaults: HashMap<Vec<u8>, [u32; 4]>,
}

impl NativeShadowMaterialParameters {
    fn vector(
        &self,
        name: &[u8],
        fallback: Option<&render_program::SerializedParameterBlock>,
    ) -> Result<[f32; 4], String> {
        let default = *self.defaults.get(name).ok_or_else(|| {
            format!(
                "unresolved native shadow Uniform {}",
                String::from_utf8_lossy(name)
            )
        })?;
        self.overrides
            .vector_words(name, 0x30, default, fallback)
            .map(|words| words.map(f32::from_bits))
    }

    fn vegetation(
        &self,
        fallback: Option<&render_program::SerializedParameterBlock>,
    ) -> Result<native_shadow_scene::NativeVegetationShadowParameters, String> {
        Ok(native_shadow_scene::NativeVegetationShadowParameters {
            main_variation: self.vector(b"vegetanim/mainbendingvar", fallback)?,
            main_frequency: self.vector(b"vegetanim/mainbendingfreq", fallback)?,
            main_amplitude: self.vector(b"vegetanim/mainbendingamplitude", fallback)?,
            minimum_wind: self.vector(b"vegetanim/leafminwindstrength", fallback)?,
            leaf_frequency: self.vector(b"vegetanim/leaffreq", fallback)?,
            leaf_amplitude: self.vector(b"vegetanim/leafamplitude", fallback)?,
            branch_amplitude: self.vector(b"vegetanim/branchamplitude", fallback)?,
        })
    }

    /// Resolve the real material vectors and original model-selected gameplay
    /// clock into VS14200's exact 208-byte record. The draw owner still supplies
    /// fallback, native model flags/offset, MVP and depth multiplier explicitly.
    fn vegetation_instance_cb(
        &self,
        fallback: Option<&render_program::SerializedParameterBlock>,
        clocks: &game_clock::GameplayViewClocks,
        model_offset: f32,
        model_flags_1c31: u8,
        mvp_rows: &[[f32; 4]; 4],
        model_depth_multiplier: f32,
    ) -> Result<[u8; native_shadow_scene::NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES], String> {
        let local_time = clocks
            .model_local_time(model_offset, model_flags_1c31)
            .ok_or_else(|| {
                format!(
                    "unavailable native vegetation clock group {}",
                    render_model::model_shader_time_group(model_flags_1c31)
                )
            })?;
        Ok(native_shadow_scene::native_vegetation_shadow_instance_cb(
            self.vector(b"layer0_scalebias", fallback)?,
            &self.vegetation(fallback)?,
            mvp_rows,
            local_time,
            model_depth_multiplier,
        ))
    }
}

#[derive(Resource, Default)]
struct NativeShadowMaterials(HashMap<MaterialHandle, Result<NativeShadowMaterialInputs, String>>);

impl NativeShadowMaterials {
    fn skin_influences(&self,material:&MaterialHandle)->u8 {
        self.0.get(material).and_then(|inputs|inputs.as_ref().ok())
            .and_then(|inputs|inputs.rigid_base_source.as_ref().ok()).and_then(Option::as_ref)
            .map_or(0,|program|program.skin_influences)
    }
    fn uses_vegetation(&self, material: &MaterialHandle) -> bool {
        let Some(Ok(inputs)) = self.0.get(material) else {
            return false;
        };
        let Ok(Some(source)) = &inputs.rigid_base_source else {
            return false;
        };
        source.class == shader_cache::CachedRigidShadowClass::Vegetation
    }
}

/// Original vertex inputs retained only for meshes whose resolved native source
/// consumes them. Errors remain available to the shadow owner, not synthetic
/// normals/colors or a silently substituted rigid program.
#[derive(Resource, Default)]
struct NativeVegetationMeshInputs(HashMap<bevy::asset::AssetId<Mesh>, Result<usize, String>>);

/// Original BWM draw-object ownership kept beside the shared forward meshes.
/// The native world-shadow owner addresses object IDs and matrix ranges, while
/// the forward scene may use per-matrix baked meshes for exact affine shear.
#[derive(Resource, Default)]
struct NativeWorldShadowMeshSources {
    objects: Vec<NativeWorldShadowMeshSource>,
    shared_mesh_count: usize,
}

#[derive(Clone)]
struct NativeWorldShadowMeshSource {
    object_id: usize,
    mesh_index: usize,
    matrix_start: usize,
    matrix_end: usize,
    lod: usize,
    flipped: bool,
    mesh: Handle<Mesh>,
    raw_vertex_count: Result<usize, String>,
    /// The actual material resolved for this BWM object's forward instances.
    /// Native world records likewise own one material pointer at +0x18.
    /// None means no binding was resolved; it must not imply no shadows.
    material: Option<MaterialHandle>,
    material_error: Option<String>,
    /// Native slot admission survives an intentionally absent color binding.
    /// In particular noShadow/screen-gate/invisible materials must be tested
    /// by the native pointer-bank rule before requiring a GPU draw material.
    gather_flags48: Option<Result<u8,String>>,
}

impl NativeWorldShadowMeshSources {
    fn prepare(world: &world_mesh::WorldMesh, meshes: &mut Assets<Mesh>) -> Self {
        let mut shared = HashMap::<(usize, bool), (Handle<Mesh>, Result<usize, String>)>::new();
        let mut objects = Vec::with_capacity(world.objects.len());
        for (object_id, object) in world.objects.iter().enumerate() {
            let source = &world.meshes[object.mesh_index];
            let key = (object.mesh_index, object.flipped);
            let (mesh, raw_vertex_count) = if let Some((mesh, count)) = shared.get(&key) {
                (mesh.clone(), count.clone())
            } else {
                let mut forward = source_mesh(
                    &source.positions,
                    &source.uvs,
                    &source.indices,
                    &source.packed_channels,
                    object.flipped,
                );
                let raw_vertex_count = retain_native_world_shadow_channels(
                    &mut forward,
                    &source.positions,
                    &source.packed_channels,
                );
                // Per-frame native program admission validates the original
                // vertex ABI after render extraction as well as at startup.
                forward.asset_usage |= RenderAssetUsages::MAIN_WORLD;
                let mesh = meshes.add(forward);
                shared.insert(key, (mesh.clone(), raw_vertex_count.clone()));
                (mesh, raw_vertex_count)
            };
            objects.push(NativeWorldShadowMeshSource {
                object_id,
                mesh_index: object.mesh_index,
                matrix_start: object.matrix_start,
                matrix_end: object.matrix_end,
                lod: object.lod,
                flipped: object.flipped,
                mesh,
                raw_vertex_count,
                material: None,
                material_error: None,
                gather_flags48: None,
            });
        }
        Self {
            objects,
            shared_mesh_count: shared.len(),
        }
    }

    fn object(&self, object_id: usize) -> Option<&NativeWorldShadowMeshSource> {
        self.objects
            .get(object_id)
            .filter(|source| source.object_id == object_id)
    }

    fn bind_material(&mut self, object_id: usize, material: &MaterialHandle) -> Result<(), String> {
        let source = self
            .objects
            .get_mut(object_id)
            .filter(|source| source.object_id == object_id)
            .ok_or("native BWM material binding has no object owner")?;
        if let Some(existing) = &source.material {
            if existing != material {
                return Err(format!(
                    "native BWM object {object_id} resolved conflicting material owners"
                ));
            }
        } else {
            source.material = Some(material.clone());
            source.material_error = None;
        }
        Ok(())
    }

    fn record_material_error(&mut self, object_id: usize, error: String) -> Result<(), String> {
        let source = self
            .objects
            .get_mut(object_id)
            .filter(|source| source.object_id == object_id)
            .ok_or("native BWM material error has no object owner")?;
        if source.material.is_none() {
            source.material_error = Some(error);
        }
        Ok(())
    }
}

impl NativeVegetationMeshInputs {
    fn prepare<const N: usize>(
        &mut self,
        required: bool,
        handle: &Handle<Mesh>,
        meshes: &mut Assets<Mesh>,
        positions: &[[f32; 3]],
        packed: &[[u8; N]],
    ) {
        if !required || self.0.contains_key(&handle.id()) {
            return;
        }
        let result = meshes
            .get_mut(handle)
            .ok_or_else(|| "native vegetation source Mesh is unavailable".to_string())
            .and_then(|mut mesh| retain_native_vegetation_channels(&mut mesh, positions, packed))
            .map(|()| positions.len());
        if let Err(error) = &result {
            eprintln!(
                "Native vegetation Mesh {:?} unavailable: {error}",
                handle.id()
            );
        }
        self.0.insert(handle.id(), result);
    }
}

fn retain_native_model_shadow_position(meshes:&mut Assets<Mesh>,handle:&Handle<Mesh>,positions:&[[f32;3]]) {
    let mut mesh=meshes.get_mut(handle).expect("new native model Mesh retains its startup owner");
    if !mesh.contains_attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION) {
        mesh.insert_attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION,positions.to_vec());
    }
}

/// Installed synchronous program-cache owner. Data streams stay lazy and
/// read-only; this is source data, NOT a fabricated executable shadow bank.
/// Keeping it separate from SceneData avoids invalidating visibility resources
/// when an on-demand shader read advances a file cursor.
#[derive(Resource)]
struct NativeProgramCacheSource(Result<shader_cache::NativeProgramLibrary, String>);

fn native_rest_skinning(
    catalog:&archive::Catalog,resource:&str,model:&bmd6::ModelInfo,buffers:&mut Assets<ShaderBuffer>,
) ->Result<Arc<native_shadow_live::NativeRestSkinning>,String> {
    let binding=bmd6::resolve_skeleton_binding(resource,|path|catalog.read(path).ok().flatten()
        .map(|bytes|entities::native_declaration_text(&bytes).into_owned()))?;
    let name=binding.as_ref().map(|binding|binding.name.as_str()).filter(|name|!name.is_empty())
        .or_else(||(!model.skeleton_name.is_empty()).then_some(model.skeleton_name.as_str()))
        .ok_or("native MD6 init has no actual skeleton binding")?;
    let stem=name.strip_suffix(".md6skl").ok_or("unsupported native skeleton resource suffix")?;
    let path=format!("generated/skeleton/{stem}.edgeskl");
    let bytes=catalog.read(&path).map_err(|e|e.to_string())?.ok_or_else(||format!("native skeleton {path} is absent"))?;
    let skeleton=edge_skeleton::parse(&bytes)?;
    let root=edge_skeleton::JointRestPose {rotation:[0.0,0.0,0.0,1.0],
        translation:binding.as_ref().and_then(|binding|binding.offset).unwrap_or([0.0;3]),scale:[1.0;3]};
    let current=edge_skeleton::native_pose_matrices(&skeleton.rest,&skeleton.hierarchy,&root)?;
    let matrices=edge_skeleton::native_skin_palette(&current,&skeleton.inverse_bind[..current.len()])?;
    if matrices.is_empty() || matrices.iter().flatten().flatten().any(|value|!value.is_finite()) {
        return Err("native rest skin palette is empty or non-finite".into());
    }
    let gpu=matrices.iter().map(|matrix|matrix.map(Vec4::from_array)).collect::<Vec<_>>();
    Ok(Arc::new(native_shadow_live::NativeRestSkinning {palette:buffers.add(ShaderBuffer::from(gpu)),matrices}))
}

fn retain_native_skin_inputs(mesh:&mut Mesh,surface:&bmd6::SurfaceInfo) {
    use bevy::mesh::VertexAttributeValues;
    use native_shadow_scene::*;
    for (attribute,values,start) in [(NATIVE_SKIN_INDICES_0,&surface.skin_indices,0),
        (NATIVE_SKIN_INDICES_1,&surface.skin_indices,4)] {
        mesh.insert_attribute(attribute,VertexAttributeValues::Uint8x4(values.iter().map(|v|v[start..start+4].try_into().unwrap()).collect()));
    }
    for (attribute,values,start) in [(NATIVE_SKIN_WEIGHTS_0,&surface.skin_weights,0),
        (NATIVE_SKIN_WEIGHTS_1,&surface.skin_weights,4)] {
        mesh.insert_attribute(attribute,VertexAttributeValues::Unorm8x4(values.iter().map(|v|v[start..start+4].try_into().unwrap()).collect()));
    }
}

/// The rewrite currently owns a static rest pose. Use that actual palette in
/// both color geometry and shadow geometry; raw bind inputs stay separate.
fn apply_native_rest_pose(mesh:&mut Mesh,surface:&bmd6::SurfaceInfo,palette:&[[[f32;4];3]],
    influences:u8,placement:&entities::PlacedModel,world_baked:bool)->Result<(),String> {
    if surface.skin_indices.len()!=surface.positions.len() || surface.skin_weights.len()!=surface.positions.len() {
        return Err("native skin vertex streams have different lengths".into());
    }
    let mut positions=Vec::new();let mut normals=Vec::new();let mut tangents=Vec::new();let mut bitangents=Vec::new();
    let direction=|v|if world_baked {placed_direction(placement,v)}else{game_to_view(v.to_array())};
    for i in 0..surface.positions.len() {
        let matrix=edge_skeleton::native_vertex_skin_matrix(palette,surface.matrix_offset,surface.skin_indices[i],surface.skin_weights[i],influences)?;
        let transform=|v:Vec3,w:f32|Vec3::from_array(matrix.map(|r|Vec4::from_array(r).dot(v.extend(w))));
        let p=transform(Vec3::from_array(surface.positions[i]),1.0);
        positions.push(if world_baked {placed_vertex(placement,p.to_array()).to_array()}else{game_to_view(p.to_array()).to_array()});
        let packed=surface.packed_channels[i];let snorm=|b:u8|((b as i8 as f32)/127.0).max(-1.0);
        let n=transform(Vec3::new(snorm(packed[4]),snorm(packed[5]),snorm(packed[6])),0.0);
        let t=transform(Vec3::new(snorm(packed[8]),snorm(packed[9]),snorm(packed[10])),0.0);
        let sign=snorm(packed[11]);
        normals.push(direction(n).to_array());tangents.push(direction(t).extend(sign).to_array());
        bitangents.push(direction(n.cross(t)*sign).to_array());
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION,positions);mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL,normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT,tangents);mesh.insert_attribute(DH2_BITANGENT,bitangents);
    Ok(())
}

#[derive(Clone)]
struct CachedMaterial {
    handle: MaterialHandle,
    diffuse_image_slot: Option<usize>,
    images: Vec<(usize, surface_gpu::ImageRole)>,
    native_vertex_lighting: bool,
}

/// Persistent native directional target and level-owned preparation. This is
/// separate from NativeShadowSceneFrame: allocating the atlas must never turn
/// unresolved per-frame caster/receiver inputs into a fabricated Ready frame.
///
/// Whole-view caster submission is NOT a Tome/color-visible-ID query:
/// FUN_14032de00 (analysis/decompiled/chunk_000120.c:2646) builds world draw
/// masks in 4096-record chunks using object LOD low bits and group parents.
/// WorldMesh::for_each_whole_view_shadow_batch preserves its native overlap
/// arithmetic. The first masks below belong to initial publication, not a
/// cross-frame cache; later world submissions must run the same rule again.
/// Material/program packet submission is still separate from mask selection.
/// FUN_1402996f0 (analysis/decompiled/chunk_000107.c:3544) gathers selected
/// record IDs using material +0x48 masks0x01/0x40, binds structured world buffers,
/// then resolves shadow program +0x250. Its packet uses the first record's
/// serialized final u16: the loader maps FILE +28 to RUNTIME +0x1e, leaving
/// separate +0x1c for the nonserialized wind-center index. World shader
/// remapping and the bank2 source are resolved separately from rigid models.
#[derive(Resource)]
struct NativeParallelShadowOwner {
    target_image: Handle<Image>,
    dimensions: native_shadow_scene::NativeParallelShadowTargetDimensions,
    whole_view: render_shadow::DirectionalCascade,
    whole_view_initial_masks: Vec<Vec<u8>>,
    hardware_bias: [f32; 3],
    shader_bias: [f32; 3],
    sort_shadows_ccw: bool,
    /// Source definition byte+4 copied into view+0x39d4. This selects the
    /// actual world `$localTime`; it is not a render-model clock selector.
    source_flag_04: u8,
    cascade_controls: original_settings::ShadowCascadeControls,
    /// Parsed r_shadowCascadeFars CVar; None retains its executable default.
    cascade_fars: [f32; 4],
}

impl NativeParallelShadowOwner {
    fn from_scene(scene: &SceneData, images: &mut Assets<Image>) -> Result<Self, String> {
        let dimensions = scene
            .shadow_target_dimensions
            .ok_or("native shadow target settings are unresolved")?;
        let environment = scene
            .environment_lighting
            .ok_or("native shadow environment is unresolved")?;
        let parameters = scene
            .environment_render_parameters
            .as_ref()
            .ok_or("native shadow environment render parameters are unresolved")?;
        let authored = scene
            .far_shadow_bounds
            .ok_or("native world m_farShadowsArea default/override is unresolved")?;
        let bounds = if render_shadow::whole_view_shadow_uses_world_bounds(authored) {
            scene
                .world
                .static_query_bounds()
                .ok_or("native static-world shadow query has no root")?
        } else {
            authored
        };
        let toward_sun = render_shadow::environment_sun_direction(
            environment
                .sun_theta_degrees
                .ok_or("native sun theta is unresolved")?,
            environment
                .sun_phi_degrees
                .ok_or("native sun phi is unresolved")?,
        );
        let ray = toward_sun.map(|value| f32::from_bits(value.to_bits() ^ 0x8000_0000));
        let near = render_shadow::directional_shadow_near(environment.parallel_light_extend)?;
        let whole_view = render_shadow::whole_view_shadow_projection(
            bounds,
            ray,
            near,
            dimensions.width as i32,
        )?;
        let hardware_bias = render_shadow::parallel_hardware_bias(
            parameters,
            dimensions.depth_32_bits,
            scene
                .shadow_depth_bias_clamp
                .ok_or("native r_depthBiasClamp is unresolved")?,
        )?;
        let shader_bias =
            render_shadow::parallel_shader_bias(parameters, dimensions.depth_32_bits)?;
        let sort_shadows_ccw = scene
            .shadow_sort_ccw
            .ok_or("native r_sortShadowsCCW is unresolved")?;
        let cascade_controls = scene
            .shadow_cascade_controls
            .ok_or("native cascade update controls are unresolved")?;
        let cascade_fars = scene.shadow_cascade_fars.unwrap_or([4.0, 12.0, 30.0, 75.0]);
        let mut whole_view_initial_masks = Vec::new();
        scene.world.for_each_whole_view_shadow_batch(|mask| {
            whole_view_initial_masks.push(mask.to_vec());
        })?;
        let format = match render_shadow::parallel_shadow_depth_format(dimensions.depth_32_bits)? {
            render_shadow::ParallelShadowDepthFormat::Depth16 => {
                bevy::render::render_resource::TextureFormat::Depth16Unorm
            }
            render_shadow::ParallelShadowDepthFormat::Depth24Stencil8 => {
                bevy::render::render_resource::TextureFormat::Depth24PlusStencil8
            }
        };
        // Resolve all source-dependent inputs before mutating Assets: an error
        // must not leave a new, unused atlas behind on every initialization.
        let target_image = images.add(native_shadow_gpu::shadow_depth_image(
            bevy::render::render_resource::Extent3d {
                width: dimensions.width,
                height: dimensions.height,
                depth_or_array_layers: dimensions.layers,
            },
            format,
        )?);
        Ok(Self {
            target_image,
            dimensions,
            whole_view,
            whole_view_initial_masks,
            hardware_bias,
            shader_bias,
            sort_shadows_ccw,
            source_flag_04: game_clock::RESET_VIEW_LOCAL_TIME_FLAG,
            cascade_controls,
            cascade_fars,
        })
    }
}

#[derive(Clone)]
struct NativeWorldShadowMaskPlan {
    instance_ids: Vec<u32>,
    auxiliary: Vec<[u32; 8]>,
    instances: Vec<NativeWorldShadowPreparedInstance>,
}

#[derive(Clone)]
struct NativeWorldShadowPreparedInstance {
    mesh: Handle<Mesh>,
    constants: NativeWorldShadowPreparedConstants,
    program: native_shadow_scene::NativeShadowProgram,
    alpha_material: Option<native_shadow_scene::NativeShadowAlphaMaterial>,
    front_face: bevy::render::render_resource::FrontFace,
    cull_mode: Option<bevy::render::render_resource::Face>,
    draw_state: [u32; 9],
}

#[derive(Clone)]
enum NativeWorldShadowPreparedConstants {
    World {
        bytes: [u8; native_shadow_scene::NATIVE_WORLD_SHADOW_INSTANCE_BYTES],
        instance_count: u32,
    },
    Vegetation {
        bytes: [u8; native_shadow_scene::NATIVE_VEGETATION_SHADOW_INSTANCE_BYTES],
        instance_count: u32,
        uses_wind_buffer: bool,
    },
}

/// Resolve every initial native mask to CPU-owned draw inputs before creating
/// any ShaderBuffer assets. A missing off-camera alpha image is queued through
/// the ordinary texture streamer and retried; no caster is silently dropped.
fn prepare_native_whole_view_masks(
    owner: &NativeParallelShadowOwner,
    world: &world_mesh::WorldMesh,
    sources: &NativeWorldShadowMeshSources,
    materials: &NativeShadowMaterials,
    meshes: &Assets<Mesh>,
    images: &Assets<Image>,
    queue: &mut SceneRenderQueue,
    local_time: Option<[f32; 2]>,
    clock_error: Option<&str>,
) -> Result<Vec<NativeWorldShadowMaskPlan>, String> {
    prepare_native_world_shadow_masks(owner,world,sources,materials,meshes,images,queue,
        &owner.whole_view_initial_masks,&owner.whole_view.clip,local_time,clock_error)
}

fn prepare_native_world_shadow_masks(
    owner: &NativeParallelShadowOwner,
    world: &world_mesh::WorldMesh,
    sources: &NativeWorldShadowMeshSources,
    materials: &NativeShadowMaterials,
    meshes: &Assets<Mesh>, images: &Assets<Image>, queue: &mut SceneRenderQueue,
    masks: &[Vec<u8>], clip: &render_shadow::ShadowMatrix,
    local_time: Option<[f32;2]>, clock_error: Option<&str>,
) -> Result<Vec<NativeWorldShadowMaskPlan>,String> {
    prepare_native_world_shadow_masks_with_settings(&NativeWorldShadowDrawSettings {
        dimensions:owner.dimensions,hardware_bias:owner.hardware_bias,sort_shadows_ccw:owner.sort_shadows_ccw,
        source_flag_04:owner.source_flag_04,exclude_lod:false,depth_clip:0,
    },world,sources,materials,meshes,images,queue,masks,clip,local_time,clock_error)
}

struct NativeWorldShadowDrawSettings {
    dimensions:native_shadow_scene::NativeParallelShadowTargetDimensions,
    hardware_bias:[f32;3],sort_shadows_ccw:bool,source_flag_04:u8,
    exclude_lod:bool,depth_clip:u8,
}

fn prepare_native_world_shadow_masks_with_settings(
    owner: &NativeWorldShadowDrawSettings,
    world: &world_mesh::WorldMesh,
    sources: &NativeWorldShadowMeshSources,
    materials: &NativeShadowMaterials,
    meshes: &Assets<Mesh>, images: &Assets<Image>, queue: &mut SceneRenderQueue,
    masks: &[Vec<u8>], clip: &render_shadow::ShadowMatrix,
    local_time: Option<[f32;2]>, clock_error: Option<&str>,
) -> Result<Vec<NativeWorldShadowMaskPlan>,String> {
    if sources.objects.len() != world.objects.len() {
        return Err("native BWM mesh/material owner count differs from world records".into());
    }
    if owner.dimensions.width == 0 || owner.dimensions.height == 0 {
        return Err("native whole-view shadow target has zero dimensions".into());
    }
    if clip
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
        || owner.hardware_bias.iter().any(|value| !value.is_finite())
    {
        return Err("native whole-view projection or hardware bias is non-finite".into());
    }
    if queue.diffuse_image_handles.len() != queue.diffuse_image_paths.len() {
        return Err("native shadow diffuse image handles and paths have different domains".into());
    }

    let mut flags48 = Vec::with_capacity(world.objects.len());
    for (object_id, object) in world.objects.iter().enumerate() {
        let source = sources
            .object(object_id)
            .ok_or_else(|| format!("native BWM object {object_id} has no retained mesh owner"))?;
        if source.mesh_index != object.mesh_index
            || source.matrix_start != object.matrix_start
            || source.matrix_end != object.matrix_end
            || source.lod != object.lod
            || source.flipped != object.flipped
        {
            return Err(format!(
                "native BWM object {object_id} source association changed"
            ));
        }
        if let Some(flags) = &source.gather_flags48 {
            flags48.push(flags.clone().map_err(|error|
                format!("native BWM object {object_id} admission: {error}"))?);
            continue;
        }
        let Some(material) = source.material.as_ref() else {
            if object.matrix_start == object.matrix_end {
                // Empty record ranges cannot contribute a native packet.
                flags48.push(0);
                continue;
            }
            return Err(source.material_error.clone().unwrap_or_else(|| {
                format!("native BWM object {object_id} has no resolved material +0x18 owner")
            }));
        };
        let inputs = materials
            .0
            .get(material)
            .ok_or_else(|| {
                format!("native BWM object {object_id} material has no native input record")
            })?
            .as_ref()
            .map_err(|error| format!("native BWM object {object_id} material input: {error}"))?;
        flags48.push(
            inputs
                .world_gather_flags48()
                .map_err(|error| format!("native BWM object {object_id} +0x48 flags: {error}"))?,
        );
    }

    if owner.exclude_lod {
        //2a2490/2a3080/2a4850 reject native object+c bit0 before gathering.
        for (flags,object) in flags48.iter_mut().zip(&world.objects) {
            if object.lod&1!=0 {*flags=0;}
        }
    }

    let mut plans = Vec::with_capacity(masks.len());
    for (mask_index, mask) in masks.iter().enumerate() {
        let mut gathered = world
            .gather_native_shadow_packets(mask, &flags48)
            .map_err(|error| format!("native whole-view mask {mask_index}: {error}"))?;
        gathered.sort_for_shadow_draw(owner.sort_shadows_ccw);
        for &id in &gathered.instance_ids {
            if id > 0x00ff_ffff || id as usize >= world.matrices.len() {
                return Err(format!(
                    "native whole-view mask {mask_index} contains invalid low-24-bit matrix ID {id}"
                ));
            }
            if world.matrices[id as usize]
                .iter()
                .any(|value| !value.is_finite())
            {
                return Err(format!(
                    "native whole-view matrix {id} contains a non-finite affine value"
                ));
            }
        }
        if gathered.auxiliary.iter().flatten().any(|word| *word != 0) {
            return Err(format!(
                "native whole-view mask {mask_index} auxiliary seed differs from the verified zero source"
            ));
        }

        let mut plan = NativeWorldShadowMaskPlan {
            instance_ids: gathered.instance_ids,
            auxiliary: gathered.auxiliary,
            instances: Vec::with_capacity(gathered.packets.len()),
        };
        for packet in gathered.packets {
            let count = packet
                .end
                .checked_sub(packet.start)
                .filter(|count| *count != 0)
                .ok_or_else(|| {
                    format!(
                        "native whole-view packet for object {} has an empty or reversed range",
                        packet.object
                    )
                })?;
            if packet.end as usize > plan.instance_ids.len() {
                return Err(format!(
                    "native whole-view packet for object {} exceeds its mask ID buffer",
                    packet.object
                ));
            }
            let object = world.objects.get(packet.object).ok_or_else(|| {
                format!(
                    "native whole-view packet references absent BWM object {}",
                    packet.object
                )
            })?;
            let source = sources.object(packet.object).ok_or_else(|| {
                format!(
                    "native whole-view packet object {} has no retained mesh",
                    packet.object
                )
            })?;
            let material = source.material.as_ref().ok_or_else(|| {
                source.material_error.clone().unwrap_or_else(|| {
                    format!(
                        "native whole-view caster object {} has no material owner",
                        packet.object
                    )
                })
            })?;
            let inputs = materials
                .0
                .get(material)
                .ok_or_else(|| {
                    format!(
                        "native whole-view caster object {} has no material inputs",
                        packet.object
                    )
                })?
                .as_ref()
                .map_err(|error| {
                    format!(
                        "native whole-view caster object {} material: {error}",
                        packet.object
                    )
                })?;
            let world_source = inputs
                .world_source
                .as_ref()
                .map_err(|error| {
                    format!(
                        "native world bank-2 source for object {}: {error}",
                        packet.object
                    )
                })?
                .as_ref()
                .ok_or_else(|| {
                    format!(
                        "native world bank-2 slot +0x250 absent for caster object {}",
                        packet.object
                    )
                })?;
            let class = world_source.class;
            let raw_vertex_count = *source.raw_vertex_count.as_ref().map_err(|error| {
                format!("native world source Mesh object {}: {error}", packet.object)
            })?;
            if raw_vertex_count == 0 {
                return Err(format!(
                    "native world caster object {} has no source vertices",
                    packet.object
                ));
            }
            let source_mesh = meshes.get(&source.mesh).ok_or_else(|| {
                format!(
                    "native world source Mesh object {} is unavailable",
                    packet.object
                )
            })?;
            if source_mesh.count_vertices() != raw_vertex_count
                || source_mesh.indices().is_none()
                || source_mesh
                    .attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION)
                    .is_none()
            {
                return Err(format!(
                    "native world source Mesh object {} lacks its indexed source-position ABI",
                    packet.object
                ));
            }
            if matches!(
                class,
                shader_cache::CachedWorldShadowClass::Vegetation
                    | shader_cache::CachedWorldShadowClass::VegetationFixed
            ) && (source_mesh
                .attribute(native_shadow_scene::NATIVE_VEGETATION_NORMAL)
                .is_none()
                || source_mesh
                    .attribute(native_shadow_scene::NATIVE_VEGETATION_COLOR)
                    .is_none())
            {
                return Err(format!(
                    "native world vegetation Mesh object {} lacks authored NORMAL/COLOR channels",
                    packet.object
                ));
            }
            let (program, constants, alpha_material) = match class {
                shader_cache::CachedWorldShadowClass::Opaque => (
                    native_shadow_scene::NativeShadowProgram::WorldOpaque,
                    NativeWorldShadowPreparedConstants::World {
                        bytes: native_shadow_scene::native_world_shadow_view_cb(
                            clip,
                            packet.start,
                        ),
                        instance_count: count,
                    },
                    None,
                ),
                shader_cache::CachedWorldShadowClass::AlphaTested
                | shader_cache::CachedWorldShadowClass::VistaAlpha
                | shader_cache::CachedWorldShadowClass::Vegetation
                | shader_cache::CachedWorldShadowClass::VegetationFixed => {
                    let params = inputs
                        .world_parameters
                        .as_ref()
                        .map_err(|error| {
                            format!(
                                "native world parameters for object {}: {error}",
                                packet.object
                            )
                        })?
                        .as_ref()
                        .ok_or_else(|| {
                            format!(
                                "native world material parameters are absent for object {}",
                                packet.object
                            )
                        })?;
                    let uv_scale_bias =
                        params.vector(b"layer0_scalebias", None).map_err(|error| {
                            format!(
                                "native Layer0 UV parameters for object {}: {error}",
                                packet.object
                            )
                        })?;
                    if uv_scale_bias.iter().any(|value| !value.is_finite()) {
                        return Err(format!(
                            "native Layer0 UV parameters for object {} are non-finite",
                            packet.object
                        ));
                    }
                    let uv_set = if class == shader_cache::CachedWorldShadowClass::VistaAlpha {
                        // The installed VistaAlpha fingerprint uses raw TEXCOORD0.
                        0
                    } else {
                        inputs.world_uv_set.ok_or_else(|| format!(
                            "native world alpha material object {} has no resolved Layer0 UV selector", packet.object
                        ))?
                    };
                    if uv_set > 1 {
                        return Err(format!(
                            "native world alpha material object {} selects unsupported UV set {uv_set}",
                            packet.object
                        ));
                    }
                    let uv_attribute = if uv_set == 0 {
                        Mesh::ATTRIBUTE_UV_0
                    } else {
                        Mesh::ATTRIBUTE_UV_1
                    };
                    if source_mesh.attribute(uv_attribute).is_none() {
                        return Err(format!(
                            "native world alpha Mesh object {} lacks authored UV set {uv_set}",
                            packet.object
                        ));
                    }
                    let slot = inputs.diffuse_image_slot.ok_or_else(|| {
                        format!(
                            "native world alpha material object {} has no diffuse image slot",
                            packet.object
                        )
                    })?;
                    let path = queue.diffuse_image_paths.get(slot).cloned()
                        .ok_or_else(|| format!("native world alpha material object {} references absent diffuse slot {slot}", packet.object))?;
                    if !queue.native_shadow_diffuse_slots.contains(&slot) {
                        queue.native_shadow_diffuse_slots.push(slot);
                    }
                    let diffuse = queue.diffuse_image_handles.get(slot)
                        .and_then(Option::as_ref)
                        .ok_or_else(|| format!("native whole-view alpha image pending through texture streamer: slot {slot} ({path})"))?;
                    if images.get(diffuse).is_none() {
                        return Err(format!(
                            "native whole-view alpha image asset is unavailable: slot {slot} ({path})"
                        ));
                    }
                    let alpha_material = native_shadow_scene::NativeShadowAlphaMaterial {
                        diffuse: diffuse.clone(),
                        uv_set,
                        uv_scale_bias,
                    };
                    match class {
                        shader_cache::CachedWorldShadowClass::AlphaTested => (
                            native_shadow_scene::NativeShadowProgram::WorldAlphaTested,
                            NativeWorldShadowPreparedConstants::World {
                                bytes: native_shadow_scene::native_world_alpha_shadow_view_cb(
                                    clip,
                                    packet.start,
                                    uv_scale_bias,
                                ),
                                instance_count: count,
                            },
                            Some(alpha_material),
                        ),
                        shader_cache::CachedWorldShadowClass::VistaAlpha => (
                            native_shadow_scene::NativeShadowProgram::WorldVistaAlpha,
                            NativeWorldShadowPreparedConstants::World {
                                bytes: native_shadow_scene::native_world_alpha_shadow_view_cb(
                                    clip,
                                    packet.start,
                                    uv_scale_bias,
                                ),
                                instance_count: count,
                            },
                            Some(alpha_material),
                        ),
                        shader_cache::CachedWorldShadowClass::Vegetation
                        | shader_cache::CachedWorldShadowClass::VegetationFixed => {
                            let time = local_time.ok_or_else(|| format!(
                                "native world vegetation `$localTime` unavailable for source flag+0x04={}; {}",
                                owner.source_flag_04,
                                clock_error.unwrap_or("native gameplay-view clock is absent")
                            ))?;
                            if time.iter().any(|value| !value.is_finite()) {
                                return Err(
                                    "native world vegetation `$localTime` is non-finite".into()
                                );
                            }
                            let wind = params.vegetation(None).map_err(|error| {
                                format!(
                                    "native vegetation parameters for object {}: {error}",
                                    packet.object
                                )
                            })?;
                            let vectors = [
                                wind.main_variation,
                                wind.main_frequency,
                                wind.main_amplitude,
                                wind.minimum_wind,
                                wind.leaf_frequency,
                                wind.leaf_amplitude,
                                wind.branch_amplitude,
                            ];
                            if vectors.iter().flatten().any(|value| !value.is_finite()) {
                                return Err(format!(
                                    "native vegetation parameters for object {} are non-finite",
                                    packet.object
                                ));
                            }
                            let uses_wind_buffer =
                                class == shader_cache::CachedWorldShadowClass::Vegetation;
                            let wind_offset = if uses_wind_buffer {
                                let offset = packet.auxiliary_start.ok_or_else(|| format!(
                                    "native wind-buffer shader object {} has no gathered auxiliary offset", packet.object
                                ))?;
                                let end = usize::from(offset)
                                    .checked_add(count as usize)
                                    .ok_or("native wind auxiliary range overflow")?;
                                if end > plan.auxiliary.len() {
                                    return Err(format!(
                                        "native wind auxiliary range {offset}..{end} exceeds mask buffer",
                                    ));
                                }
                                u32::from(offset)
                            } else {
                                u32::from(packet.auxiliary_start.unwrap_or(0))
                            };
                            if uses_wind_buffer && plan.auxiliary.is_empty() {
                                return Err(format!(
                                    "native wind-buffer shader object {} has no auxiliary buffer",
                                    packet.object
                                ));
                            }
                            (
                                if uses_wind_buffer {
                                    native_shadow_scene::NativeShadowProgram::WorldVegetation
                                } else {
                                    native_shadow_scene::NativeShadowProgram::WorldVegetationFixed
                                },
                                NativeWorldShadowPreparedConstants::Vegetation {
                                    bytes:
                                        native_shadow_scene::native_world_vegetation_shadow_view_cb(
                                            clip,
                                            uv_scale_bias,
                                            &wind,
                                            time,
                                            packet.start,
                                            wind_offset,
                                        ),
                                    instance_count: count,
                                    uses_wind_buffer,
                                },
                                Some(alpha_material),
                            )
                        }
                        shader_cache::CachedWorldShadowClass::Opaque => unreachable!(),
                    }
                }
            };
            let main_render_state = inputs.source.main_render_state.ok_or_else(|| {
                format!(
                    "native main material draw state +0x2c8 is absent for caster object {}",
                    packet.object
                )
            })?;
            let state = render_state::world_shadow_draw_state(
                main_render_state,
                u8::from(object.flipped),
                owner.hardware_bias[0],
                owner.hardware_bias[1],
                owner.hardware_bias[2],
                owner.depth_clip,
            );
            let (front_face, cull_mode) =
                native_shadow_gpu::native_shadow_mesh_raster(&state, !source.flipped);
            plan.instances.push(NativeWorldShadowPreparedInstance {
                mesh: source.mesh.clone(),
                constants,
                program,
                alpha_material,
                front_face,
                cull_mode,
                draw_state: state,
            });
        }
        plans.push(plan);
    }
    Ok(plans)
}

fn build_native_whole_view_layer(
    plans: Vec<NativeWorldShadowMaskPlan>,
    world: &world_mesh::WorldMesh,
    dimensions: native_shadow_scene::NativeParallelShadowTargetDimensions,
    shader_buffers: &mut Assets<ShaderBuffer>,
) -> native_shadow_scene::NativeShadowLayer {
    let has_instances = plans.iter().any(|plan| !plan.instances.is_empty());
    let matrices =
        has_instances.then(|| shader_buffers.add(ShaderBuffer::from(world.matrices.clone())));
    build_native_world_layer(plans, matrices.as_ref(), dimensions, shader_buffers)
}

fn build_native_world_layer(
    plans: Vec<NativeWorldShadowMaskPlan>,
    matrices: Option<&Handle<ShaderBuffer>>,
    dimensions: native_shadow_scene::NativeParallelShadowTargetDimensions,
    shader_buffers: &mut Assets<ShaderBuffer>,
) -> native_shadow_scene::NativeShadowLayer {
    let mut instances = Vec::new();
    for plan in plans {
        if plan.instances.is_empty() {
            continue;
        }
        let visible_indices = shader_buffers.add(ShaderBuffer::from(plan.instance_ids));
        let winds = (!plan.auxiliary.is_empty())
            .then(|| shader_buffers.add(ShaderBuffer::from(plan.auxiliary)));
        let matrices = matrices
            .expect("nonempty native world has matrices")
            .clone();
        for prepared in plan.instances {
            let per_instance_cb = match prepared.constants {
                NativeWorldShadowPreparedConstants::World {
                    bytes,
                    instance_count,
                } => native_shadow_scene::NativeShadowInstanceConstants::World {
                    constants: bytes,
                    visible_indices: visible_indices.clone(),
                    matrices: matrices.clone(),
                    instance_count,
                },
                NativeWorldShadowPreparedConstants::Vegetation {
                    bytes,
                    instance_count,
                    uses_wind_buffer,
                } => native_shadow_scene::NativeShadowInstanceConstants::WorldVegetation {
                    constants: bytes,
                    visible_indices: visible_indices.clone(),
                    matrices: matrices.clone(),
                    winds: if uses_wind_buffer {
                        Some(
                            winds
                                .as_ref()
                                .expect("preflight validated native wind buffer")
                                .clone(),
                        )
                    } else {
                        None
                    },
                    instance_count,
                },
            };
            instances.push(native_shadow_scene::NativeShadowInstance {
                mesh: prepared.mesh,
                per_instance_cb,
                program: prepared.program,
                alpha_material: prepared.alpha_material,
                front_face: prepared.front_face,
                cull_mode: prepared.cull_mode,
                draw_state: prepared.draw_state,
                stencil_reference: 0,
                scissor: native_shadow_gpu::NativeShadowScissor {
                    x: 0,
                    y: 0,
                    width: dimensions.width,
                    height: dimensions.height,
                },
            });
        }
    }
    native_shadow_scene::NativeShadowLayer {
        ops: native_shadow_scene::NativeShadowLayerOps {
            depth: native_shadow_scene::NativeShadowLoad::Clear(1.0),
            stencil: native_shadow_scene::NativeStencilLoad::Clear(0),
        },
        cache: native_shadow_scene::NativeShadowCacheOp::None,
        instances,
    }
}

fn publish_initial_native_whole_view_depth(
    owner: Option<Res<NativeParallelShadowOwner>>,
    scene: Option<Res<SceneData>>,
    sources: Option<Res<NativeWorldShadowMeshSources>>,
    materials: Option<Res<NativeShadowMaterials>>,
    meshes: Option<Res<Assets<Mesh>>>,
    images: Option<Res<Assets<Image>>>,
    gameplay_clock: Option<Res<game_clock_runtime::NativeGameplayViewClock>>,
    mut queue: Option<ResMut<SceneRenderQueue>>,
    mut shader_buffers: Option<ResMut<Assets<ShaderBuffer>>>,
    completion:Option<Res<native_shadow_scene::NativeShadowSceneCompletion>>,
    mut frame: ResMut<native_shadow_scene::NativeShadowSceneFrame>,
    mut last_error: Local<Option<String>>,
) {
    if matches!(
        &frame.state,
        native_shadow_scene::NativeShadowSceneState::WholeViewDepth { .. }
            | native_shadow_scene::NativeShadowSceneState::Ready { .. }
    ) {
        return;
    }
    let Some(owner) = owner else { return };
    if completion.as_ref().is_some_and(|done|done.whole_view_completed(&owner.target_image,owner.dimensions)) {
        return;
    }
    let revision = match frame.revision.checked_add(1) {
        Some(revision) => revision,
        None => {
            let error = "native whole-view frame revision overflow".to_string();
            if last_error.as_ref() != Some(&error) {
                eprintln!("Native whole-view depth unavailable: {error}");
                *last_error = Some(error.clone());
            }
            frame.state = native_shadow_scene::NativeShadowSceneState::Unavailable(error);
            return;
        }
    };
    let (local_time, clock_error) = match gameplay_clock {
        Some(clock) => match clock.view_shader_times(owner.source_flag_04) {
            Ok(times) => (Some(times.local_time), None),
            Err(error) => (None, Some(error)),
        },
        None => (
            None,
            Some("native gameplay-view clock resource is unavailable".into()),
        ),
    };
    let attempt = (|| -> Result<native_shadow_scene::NativeShadowLayer, String> {
        let scene = scene
            .as_deref()
            .ok_or("native SceneData dependency is unavailable")?;
        let sources = sources
            .as_deref()
            .ok_or("native world mesh owners are unavailable")?;
        let materials = materials
            .as_deref()
            .ok_or("native shadow material inputs are unavailable")?;
        let meshes = meshes
            .as_deref()
            .ok_or("native Mesh assets are unavailable")?;
        let images = images
            .as_deref()
            .ok_or("native Image assets are unavailable")?;
        let queue = queue
            .as_deref_mut()
            .ok_or("native render queue is unavailable")?;
        let shader_buffers = shader_buffers
            .as_deref_mut()
            .ok_or("native ShaderBuffer asset table is unavailable")?;
        let plans = prepare_native_whole_view_masks(
            &owner,
            &scene.world,
            sources,
            materials,
            meshes,
            images,
            queue,
            local_time,
            clock_error.as_deref(),
        )?;
        Ok(build_native_whole_view_layer(
            plans,
            &scene.world,
            owner.dimensions,
            shader_buffers,
        ))
    })();
    match attempt {
        Ok(layer) => {
            frame.revision = revision;
            frame.state = native_shadow_scene::NativeShadowSceneState::WholeViewDepth {
                target_image: owner.target_image.clone(),
                dimensions: owner.dimensions,
                layer,
            };
            *last_error = None;
        }
        Err(error) => {
            let message = format!("native initial whole-view depth: {error}");
            if last_error.as_ref() != Some(&message) {
                eprintln!("Native whole-view depth unavailable: {error}");
                *last_error = Some(message.clone());
            }
            frame.state = native_shadow_scene::NativeShadowSceneState::Unavailable(message);
        }
    }
}

fn initialize_native_parallel_shadow_owner(
    mut commands: Commands,
    scene: Res<SceneData>,
    mut images: ResMut<Assets<Image>>,
    mut frame: ResMut<native_shadow_scene::NativeShadowSceneFrame>,
) {
    match NativeParallelShadowOwner::from_scene(&scene, &mut images) {
        Ok(owner) => {
            println!(
                "Native parallel shadow target: {}x{}, {} layers; whole-view projection and environment bias resolved",
                owner.dimensions.width, owner.dimensions.height, owner.dimensions.layers
            );
            commands.insert_resource(owner);
            frame.state = native_shadow_scene::NativeShadowSceneState::Unavailable(
                "native target owner initialized; initial native whole-view world depth publication pending".into(),
            );
        }
        Err(error) => {
            eprintln!("Native parallel shadow owner unavailable: {error}");
            commands.remove_resource::<NativeParallelShadowOwner>();
            frame.state = native_shadow_scene::NativeShadowSceneState::Unavailable(error);
        }
    }
}

fn native_shadow_target_dimensions(
    slice_size: u32,
    quality: &entities::ShadowQualitySettings,
) -> Result<native_shadow_scene::NativeParallelShadowTargetDimensions, String> {
    let target = render_shadow::ParallelShadowTargetDimensions::from_settings(
        slice_size,
        quality.parallel_resolution_divider,
        quality.parallel_slice_count,
    )?;
    Ok(native_shadow_scene::NativeParallelShadowTargetDimensions {
        width: target.side,
        height: target.side,
        layers: target.layers,
        depth_32_bits: quality.depth_32_bits,
    })
}

/// Changing the atlas association is rare; moving the camera only updates the
/// shared receiver buffer, not every material or its bind group.
fn sync_native_shadow_target<E: MaterialExtension + NativeShadowImageAccess>(
    frame: Res<native_shadow_scene::NativeShadowSceneFrame>,
    local_owner:Option<Res<native_local_shadow_live::NativeLocalShadowOwner>>,
    sampling: Option<Res<native_shadow_sampling::NativeParallelSamplingBuffer>>,
    screen:Option<Res<native_screen_shadow_scene::NativeScreenShadowFrame>>,
    mut materials: ResMut<Assets<ExtendedMaterial<StandardMaterial,E>>>,
    mut added_materials: MessageReader<bevy::asset::AssetEvent<ExtendedMaterial<StandardMaterial,E>>>,
    mut current: Local<Option<(Option<Handle<Image>>,Option<Handle<Image>>,Option<Handle<Image>>,Option<Handle<Image>>)>>,
) {
    let Some(sampling) = sampling else {
        return;
    };
    let target = match &frame.state {
        native_shadow_scene::NativeShadowSceneState::Ready { target_image, .. } => {
            Some(target_image.clone())
        }
        _ => Some(sampling.unavailable_depth.clone()),
    };
    let cloud=screen.as_ref().map(|s|s.cloud.clone());
    let screen=screen.map(|s|s.output.clone());
    let local_target=Some(local_owner.map(|o|o.atlas_image.clone()).unwrap_or_else(||sampling.unavailable_depth.clone()));
    let association=Some((target.clone(),screen.clone(),cloud.clone(),local_target.clone()));
    if *current == association {
        // New streamed materials need the current atlas even when the owner
        // hasn't changed targets. Process additions, not every material/frame.
        for event in added_materials.read() {
            if let bevy::asset::AssetEvent::Added { id } = event {
                if let Some(mut material) = materials.get_mut(*id) {
                    material.extension.sync_shadow_images(&target,&screen,&cloud,&local_target);
                }
            }
        }
        return;
    }
    added_materials.clear();
    for (_, material) in materials.iter_mut() {
        material.extension.sync_shadow_images(&target,&screen,&cloud,&local_target);
    }
    *current = association;
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(SurfaceKey)]
struct ProbeExtension {
    #[storage(100, read_only)]
    sh: Handle<ShaderBuffer>,
    #[storage(119, read_only)]
    vertex_probes: Handle<ShaderBuffer>,
    #[storage(120, read_only)]
    native_shadow: Handle<ShaderBuffer>,
    #[storage(126, read_only)]
    native_local_shadow: Handle<ShaderBuffer>,
    #[texture(127, dimension = "2d_array", sample_type = "depth")]
    #[sampler(128, sampler_type = "comparison")]
    native_local_shadow_map: Option<Handle<Image>>,
    #[texture(121, dimension = "2d_array", sample_type = "depth")]
    #[sampler(122, sampler_type = "comparison")]
    native_shadow_map: Option<Handle<Image>>,
    #[texture(123)]
    native_screen_map:Option<Handle<Image>>,
    #[texture(124)]
    #[sampler(125)]
    native_cloud_map:Option<Handle<Image>>,
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
    #[texture(134)]
    specular0: Option<Handle<Image>>,
    #[texture(135)]
    specular1: Option<Handle<Image>>,
    #[texture(136)]
    specular2: Option<Handle<Image>>,
}

// Original common vertex shader transforms N/T/B independently with the
// model's linear rows. A separate bitangent avoids replacing that rule with
// inverse-transpose normals or a post-transform cross product under shear.
const DH2_BITANGENT: MeshVertexAttribute =
    MeshVertexAttribute::new("DH2_Bitangent", 0x4448_3200, VertexFormat::Float32x3);

use surface_gpu::SurfaceKey;

impl From<&ProbeExtension> for SurfaceKey {
    fn from(extension: &ProbeExtension) -> Self {
        Self::from(&extension.surface)
    }
}

fn specialize_surface(descriptor: &mut RenderPipelineDescriptor, key: surface_gpu::SurfaceKey) {
    if let Some(fragment) = descriptor.fragment.as_mut() {
        // Optional numerical light attribution; normal rendering has no branch.
        if let Ok(component)=std::env::var("DH2_TRACE_LIGHT_COMPONENT").unwrap_or_default().parse::<u32>() {
            fragment.shader_defs.push(bevy::shader::ShaderDefVal::UInt("DH2_TRACE_LIGHT_COMPONENT".into(),component));
        }
        for (name, value) in surface_gpu::SurfaceKey::NAMES.into_iter().zip(key.0) {
            fragment
                .shader_defs
                .push(bevy::shader::ShaderDefVal::UInt(name.into(), value));
        }
    }
}

fn authored_basis_pipeline(
    descriptor: &mut RenderPipelineDescriptor,
    layout: &MeshVertexBufferLayoutRef,
) -> Result<(), SpecializedMeshPipelineError> {
    if layout.0.contains(DH2_BITANGENT) {
        let extra = layout
            .0
            .get_layout(&[DH2_BITANGENT.at_shader_location(8)])?;
        descriptor.vertex.buffers[0]
            .attributes
            .extend(extra.attributes);
        descriptor
            .vertex
            .shader_defs
            .push("DH2_AUTHORED_BASIS".into());
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_AUTHORED_BASIS".into());
        }
    }
    Ok(())
}

fn specialize_native_vertex_lighting(descriptor:&mut RenderPipelineDescriptor,layout:&MeshVertexBufferLayoutRef,key:surface_gpu::SurfaceKey) {
        let native_lighting = matches!(key.0[6], 1 | 2)
            && layout.0.contains(Mesh::ATTRIBUTE_NORMAL)
            && layout.0.contains(Mesh::ATTRIBUTE_TANGENT)
            && descriptor
                .fragment
                .as_ref()
                .is_some_and(|fragment| (fragment.shader == PROBE_SHADER_HANDLE || fragment.shader == DECAL_SHADER_HANDLE));
        if native_lighting {
            descriptor
                .vertex
                .shader_defs
                .push("DH2_VERTEX_PROBE".into());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.push("DH2_VERTEX_PROBE".into());
            }
            for (name, value) in surface_gpu::SurfaceKey::NAMES
                .into_iter()
                .zip(key.0)
            {
                descriptor
                    .vertex
                    .shader_defs
                    .push(bevy::shader::ShaderDefVal::UInt(name.into(), value));
            }
        }
}

impl MaterialExtension for ProbeExtension {
    fn fragment_shader() -> ShaderRef {
        PROBE_SHADER_HANDLE.into()
    }

    fn prepass_fragment_shader() -> ShaderRef {
        PROBE_PREPASS_SHADER_HANDLE.into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(fragment)=descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_NATIVE_LOCAL_SHADOWS".into());
        }
        specialize_native_vertex_lighting(descriptor,layout,key.bind_group_data);
        specialize_surface(descriptor, key.bind_group_data);
        authored_basis_pipeline(descriptor, layout)
    }

    fn enable_prepass() -> bool {
        true
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[bind_group_data(DecalKey)]
struct DecalExtension {
    bias: i32,
    #[storage(100,read_only)]
    sh:Handle<ShaderBuffer>,
    #[storage(119,read_only)]
    vertex_probes:Handle<ShaderBuffer>,
    #[storage(120, read_only)]
    native_shadow: Handle<ShaderBuffer>,
    #[storage(126, read_only)]
    native_local_shadow: Handle<ShaderBuffer>,
    #[texture(127, dimension = "2d_array", sample_type = "depth")]
    #[sampler(128, sampler_type = "comparison")]
    native_local_shadow_map: Option<Handle<Image>>,
    #[texture(121, dimension = "2d_array", sample_type = "depth")]
    #[sampler(122, sampler_type = "comparison")]
    native_shadow_map: Option<Handle<Image>>,
    #[texture(123)]
    native_screen_map: Option<Handle<Image>>,
    #[texture(124)]
    #[sampler(125)]
    native_cloud_map: Option<Handle<Image>>,
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
    #[texture(134)]
    specular0: Option<Handle<Image>>,
    #[texture(135)]
    specular1: Option<Handle<Image>>,
    #[texture(136)]
    specular2: Option<Handle<Image>>,
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
                    16 => &mut self.specular0,
                    17 => &mut self.specular1,
                    18 => &mut self.specular2,
                    _ => panic!("invalid authored image role"),
                }
            }
        }
    };
}
native_image_access!(ProbeExtension);
native_image_access!(DecalExtension);

trait NativeShadowImageAccess {
    fn sync_shadow_images(&mut self,target:&Option<Handle<Image>>,screen:&Option<Handle<Image>>,
        cloud:&Option<Handle<Image>>,local:&Option<Handle<Image>>);
}
macro_rules! native_shadow_image_access {
    ($ty:ty) => {
        impl NativeShadowImageAccess for $ty {
            fn sync_shadow_images(&mut self,target:&Option<Handle<Image>>,screen:&Option<Handle<Image>>,
                cloud:&Option<Handle<Image>>,local:&Option<Handle<Image>>) {
                if &self.native_shadow_map!=target {self.native_shadow_map=target.clone();}
                if &self.native_screen_map!=screen {self.native_screen_map=screen.clone();}
                if &self.native_cloud_map!=cloud {self.native_cloud_map=cloud.clone();}
                if &self.native_local_shadow_map!=local {self.native_local_shadow_map=local.clone();}
            }
        }
    };
}
native_shadow_image_access!(ProbeExtension);
native_shadow_image_access!(DecalExtension);

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
struct DecalKey {
    bias: i32,
    surface: surface_gpu::SurfaceKey,
}

impl From<&DecalExtension> for DecalKey {
    fn from(extension: &DecalExtension) -> Self {
        Self {
            bias: extension.bias,
            surface: surface_gpu::SurfaceKey::from(&extension.surface),
        }
    }
}

impl MaterialExtension for DecalExtension {
    fn fragment_shader() -> ShaderRef {
        DECAL_SHADER_HANDLE.into()
    }

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
        if let Some(fragment)=descriptor.fragment.as_mut() {
            fragment.shader_defs.push("DH2_NATIVE_LOCAL_SHADOWS".into());
        }
        specialize_native_vertex_lighting(descriptor,layout,key.bind_group_data.surface);
        specialize_surface(descriptor, key.bind_group_data.surface);
        authored_basis_pipeline(descriptor, layout)
    }
}

#[derive(Resource, Default)]
struct SceneRenderQueue {
    instances: Vec<RenderInstance>,
    active: usize,
    /// Renderer +0x2060 / slice task +0x68 ownership condition. Original
    /// 23a98b calls factory295690 then stores its BWM owner at23a99e;
    /// factory allocates even an empty model. Publish only after actual BWM
    /// construction below, never from Tome/depth availability or mesh count.
    static_world_model_present: bool,
    tome_render_instances: TomeRenderInstanceMap,
    // FUN_14037ef90's sorted authored-ID output. Its render-entity consumer
    // is still unverified; never equate these IDs with unmatched BWM records.
    visible_authored_ids: Vec<u32>,
    // FUN_14037ef90 -> FUN_140a73580/FUN_140a70570 light-query inputs.
    visible_cell_indices: Vec<u32>,
    visible_cell_mask: Vec<u32>,
    light_query_state: Option<tome::RenderQueryState>,
    /// The current merged ordinary query consumed by native receiver/caster
    /// preparation (FUN_140330c10 -> FUN_140a707e0). Preserve its Tome-index
    /// domain and original depth, separately from color-instance mapping and
    /// the light-only diagnostic that deliberately removes the depth grid.
    shadow_ordinary_query: Option<tome::RenderQueryObjectOutput>,
    visibility_suppression_groups: Vec<RenderSuppressionGroup>,
    trace_bwm_lod: bool,
    world_lods_by_render_instance: Vec<Option<usize>>,
    last_bwm_lod_visibility_report: Option<BwmLodVisibilityReport>,
    diffuse_image_demands: Vec<texture::ProjectedAreaDemand>,
    diffuse_image_paths: Vec<String>,
    diffuse_image_materials: Vec<Vec<(MaterialHandle, surface_gpu::ImageRole)>>,
    native_material_images: HashMap<MaterialHandle, Vec<usize>>,
    diffuse_image_handles: Vec<Option<Handle<Image>>>,
    /// Initial native whole-view alpha casters are independent of color visibility.
    native_shadow_diffuse_slots: Vec<usize>,
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
    fn publish_shadow_ordinary_query(&mut self, output: &tome::RenderQueryObjectOutput) {
        self.shadow_ordinary_query = Some(output.clone());
    }

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
        // Diagnostic A/B keeps traversal identical and selects the existing
        // no-grid descriptor path only for downstream model/light queries.
        if env::var_os("DH2_PERF_PROFILE").is_some()
            && env::var_os("DH2_PERF_WITHOUT_QUERY_DEPTH").is_some()
        {
            if let Some(state) = self.light_query_state.as_mut() {
                state.depth_grid = None;
            }
        }
        Ok(())
    }
}

/// FUN_140330c10 copies each prepared model's +0x1840/+0x1850 bounds
/// before invoking FUN_140a70710. This list is independent of forward-camera
/// visibility and of the per-surface shadow-program admission tests.
/// Initial boxes use the owned resource bounds and FUN_140235c80's transform.
/// The skinned-model bounds virtual (+0x80) is FUN_14025da70: resource+0xe0,
/// not an AABB reconstructed from the currently drawn/rest-pose vertices.
#[derive(Resource, Default)]
struct NativeShadowModelBoundsSources(Vec<NativeShadowModelBoundsSource>);

struct NativeShadowModelBoundsSource {
    placement: usize,
    instances: std::ops::Range<usize>,
    bounds: [[f32; 3]; 2],
}

impl NativeShadowModelBoundsSources {
    fn active_bounds(
        &self,
        scene: &SceneRenderQueue,
        pickups: &PickupState,
    ) -> Result<Vec<[[f32; 3]; 2]>, String> {
        let mut bounds = Vec::with_capacity(self.0.len());
        for source in &self.0 {
            if source.instances.is_empty() || source.instances.end > scene.instances.len() {
                return Err("native shadow model bounds reference an invalid owner range".into());
            }
            if !source
                .bounds
                .iter()
                .flatten()
                .all(|value| value.is_finite())
                || (0..3).any(|axis| source.bounds[0][axis] > source.bounds[1][axis])
            {
                return Err("native shadow model owner has invalid prepared bounds".into());
            }
            let mut active = false;
            for instance in &scene.instances[source.instances.clone()] {
                let placement = match instance.source.as_ref().map(|value| &value.surface) {
                    Some(RenderModelSurface::Bmodel { placement, .. })
                    | Some(RenderModelSurface::Bmd6 { placement, .. }) => *placement,
                    _ => return Err("native shadow model bounds have no model owner".into()),
                };
                if placement != source.placement {
                    return Err("native shadow model bounds disagree with their model owner".into());
                }
                active |= instance
                    .pickup_id
                    .as_deref()
                    .is_none_or(|id| pickups.world.pickup_available(id));
            }
            if active {
                bounds.push(source.bounds);
            }
        }
        Ok(bounds)
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

type BwmLodVisibilityCounts = BTreeMap<usize, (usize, usize)>;
type BwmLodVisibilityReport = (BwmLodVisibilityCounts, BwmLodVisibilityCounts);

/// Count world render instances by authored BWM LOD tag. Values are
/// `(total instances, visible instances)`; `None` tags are non-world draws.
fn bwm_lod_visibility_counts(
    lod_by_render_instance: &[Option<usize>],
    visible: &[bool],
) -> BwmLodVisibilityCounts {
    let mut counts = BTreeMap::<usize, (usize, usize)>::new();
    for (lod, &is_visible) in lod_by_render_instance.iter().zip(visible) {
        if let Some(lod) = lod {
            let entry = counts.entry(*lod).or_default();
            entry.0 += 1;
            entry.1 += usize::from(is_visible);
        }
    }
    counts
}

#[cfg(test)]
mod bwm_lod_visibility_tests {
    use super::bwm_lod_visibility_counts;
    use std::collections::BTreeMap;

    #[test]
    fn counts_authored_lod_instances_before_and_after_suppression() {
        let lod_by_instance = [Some(0), Some(0), Some(1), None, Some(1)];
        let before_suppression = [true, true, false, true, true];
        let after_suppression = [true, false, false, true, true];
        assert_eq!(
            bwm_lod_visibility_counts(&lod_by_instance, &before_suppression),
            BTreeMap::from([(0, (2, 2)), (1, (2, 1))])
        );
        assert_eq!(
            bwm_lod_visibility_counts(&lod_by_instance, &after_suppression),
            BTreeMap::from([(0, (2, 1)), (1, (2, 1))])
        );
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

#[derive(Clone, Copy, Debug)]
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
    let block_bytes =
        streamed_block_bytes(descriptor.format).ok_or("unsupported streamed GPU image format")?;
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

#[derive(Clone, Copy, Debug)]
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
        let size = width.div_ceil(4) as usize * height.div_ceil(4) as usize * format.block_bytes();
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

fn gpu_image_format(format: texture::CompressedFormat) -> TextureFormat {
    use texture::CompressedFormat::*;
    match format {
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

fn streamed_block_bytes(format: TextureFormat) -> Option<usize> {
    match format {
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

fn empty_streamed_image(width: u32, height: u32, mip_count: u32, format: TextureFormat) -> Image {
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
    mut diagnostics: Option<ResMut<StreamingDiagnostics>>,
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
                for &slot in slots {
                    requested[slot] = true;
                }
            } else if let Some(slot) = instance.diffuse_image_slot {
                if let Some(needed) = requested.get_mut(slot) {
                    *needed = true;
                }
            }
        }
    }
    for &slot in &queue.native_shadow_diffuse_slots {
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
            state.map(tome::RenderAabbQuery::from_state).transpose()?
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
        self.submit_material_texture_demands(
            visible_tome_indices,
            area_metrics,
            render_instances,
            image_demands,
            &HashMap::new(),
        )
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
                    demand.submit(if self.zero_metric_instances.contains(&render_index) {
                        0.0
                    } else {
                        *area_metric
                    });
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

// Diagnostic replay preserves the saved view; it does not change game culling.
fn f8_camera_pose(report: &str) -> Option<Transform> {
    fn vector(report: &str, label: &str) -> Option<[f32; 3]> {
        let text = report
            .lines()
            .find_map(|line| line.strip_prefix(label))?
            .trim();
        let values = text
            .strip_prefix('[')?
            .strip_suffix(']')?
            .split(',')
            .map(|part| part.trim().parse::<f32>())
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let values: [f32; 3] = values.try_into().ok()?;
        values
            .iter()
            .all(|value| value.is_finite())
            .then_some(values)
    }
    let eye = game_to_view(vector(report, "Camera game_xyz:")?);
    let direction = game_to_view(vector(report, "Camera direction game_xyz:")?);
    let up = if report
        .lines()
        .any(|line| line.starts_with("Camera up game_xyz:"))
    {
        game_to_view(vector(report, "Camera up game_xyz:")?)
    } else {
        Vec3::Y
    };
    (direction.length_squared() > 0.0
        && up.length_squared() > 0.0
        && direction.cross(up).length_squared() > 0.0)
        .then(|| Transform::from_translation(eye).looking_to(direction, up))
}

// Replay metadata is diagnostic input, not a replacement for the registered
// 80-degree default. Original live capture can contain a user-adjusted FOV.
fn f8_reference_fov(report: &str) -> Option<f32> {
    let value = report
        .lines()
        .find_map(|line| line.strip_prefix("Reference horizontal FOV degrees:"))
        .map_or(Some(ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES), |text| {
            text.trim().parse::<f32>().ok()
        })?;
    (value > 0.0).then_some(())?;
    tome::original_render_vertical_fov_radians(value)
        .ok()
        .map(|_| value)
}

fn reference_fov_from_projection(projection: &Projection) -> Result<f32, String> {
    if let Projection::Custom(custom) = projection {
        if let Some(native) = custom.get::<native_camera::NativeGameProjection>() {
            return Ok(native.reference_horizontal_degrees);
        }
    }
    let Projection::Perspective(perspective) = projection else {
        return Err("ordinary original world visibility requires a perspective projection".into());
    };
    let vertical = perspective.fov;
    if !vertical.is_finite() || vertical <= 0.0 || vertical >= std::f32::consts::PI {
        return Err("camera has an invalid vertical field of view".into());
    }
    // Inverse of the original FUN_140890b50 reference-aspect conversion.
    Ok((2.0 * ((vertical * 0.5).tan() / 0.5625).atan()).to_degrees())
}

fn native_view_distance_scale(
    projection: &Projection, viewport: UVec2, settings: Option<&NativeViewSettings>,
) -> Result<f32,String> {
    let settings = match settings {
        Some(settings) => settings.0.clone()?,
        None => original_settings::view_settings("")?,
    };
    let horizontal = tome::original_render_fov_radians(reference_fov_from_projection(projection)?,
        viewport.x,viewport.y,1.0)?.0;
    Ok(render_shadow::caster_distance_scale(horizontal,settings.horizontal_fov_degrees,
        settings.lod_zoom_multiplier))
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

fn authored_area_light_transform(position:Vec3,rotation:[[f32;3];3])->Option<Transform> {
    // arealight::CreateAreaLightContext: Z=SpotDir, X=AreaLightX,
    // Y=cross(Z,X). Bevy's right is -X for this emitting normal;
    // the rectangle's corner set and emitting side are unchanged by that sign.
    let direction=game_to_view(rotation[0]).normalize_or_zero();
    let up=direction.cross(game_to_view(rotation[1])).normalize_or_zero();
    if direction==Vec3::ZERO || up==Vec3::ZERO {return None;}
    Some(Transform::from_translation(position).looking_to(direction,up))
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
    } else if kind == entities::LightType::Parallel {
        normalized_intensity * render_light::ARKANE_TO_WORLD_LUMINANCE
    } else {
        // Original HDR -> world units, then cancel Bevy's lumen-to-candela
        // division shared by point, half-point and spot extraction.
        normalized_intensity * render_light::ARKANE_TO_WORLD_LUMINANCE
            * (4.0 * std::f32::consts::PI)
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

    let native_kind=if kind==entities::LightType::Spot {2}else{0};
    let cosine=if native_kind==2 {cutoff_cosine?}else{0.0};
    let (_,inverse)=render_shadow::local_light_volume_projection(
        view_to_game(position),rotation,native_kind,range,cosine).ok()?;
    let native=Mat4::from_cols_array_2d(&std::array::from_fn(|c|
        std::array::from_fn(|r|inverse[r][c])));
    let game_to_host=Mat4::from_cols(Vec4::X,Vec4::new(0.0,0.0,-1.0,0.0),
        Vec4::Y,Vec4::W);
    let clip_to_world=game_to_host*native;
    clip_to_world.is_finite().then_some(clip_to_world)
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

fn environment_sun_ray(environment: entities::EnvironmentLighting) -> Option<Vec3> {
    // FUN_140017d30 uses environment theta as a polar angle and environment
    // phi directly. FUN_14032b280/140330c10 negate its toward-sun vector.
    // Do not substitute an authored parallel entity's yaw or an elevation.
    let toward_sun = render_shadow::environment_sun_direction(
        environment.sun_theta_degrees?,
        environment.sun_phi_degrees?,
    );
    Some(-game_to_view(toward_sun))
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
    let query = tome::RenderAabbQuery::from_state(state)?;
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
        placement
            .resource_path
            .strip_prefix("generated/decls/blueprint/")?
            .strip_suffix(".decl")?
            .to_owned()
    };
    if placement
        .scale
        .iter()
        .any(|scale| !(scale * 100.0).is_finite())
    {
        return None;
    }
    let key = physworld::physics_shape_key(
        &source,
        physworld::physics_shape_kind(kind),
        placement.scale,
        0.01,
        hd,
    )?;
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
    let roots: HashMap<_, _> = placements
        .iter()
        .map(|placement| (placement.id.as_str(), placement))
        .collect();
    let mut cached = HashMap::<String, Option<Vec<[[f32; 3]; 3]>>>::new();
    let mut triangles = Vec::new();
    let mut owners = Vec::new();
    let mut matched = 0usize;
    let mut failed = 0usize;
    for body in bodies {
        let Some(&placement) = roots.get(body.entity_id.as_str()) else {
            continue;
        };
        // Dynamic/keyframed bodies need simulation, not frozen walk colliders.
        if body
            .physics_behavior
            .as_deref()
            .and_then(entities::rigid_body_behavior_kind)
            != Some(0)
        {
            continue;
        }
        let kind = match body
            .shape_type
            .as_deref()
            .and_then(entities::rigid_body_shape_kind)
        {
            Some(index) => ["hkt", "cvx", "msh", "box", "cvd"][index as usize],
            other => {
                failed += 1;
                eprintln!(
                    "Collision {}: unsupported declared shape {:?} ({other:?})",
                    placement.id, body.shape_type
                );
                continue;
            }
        };
        let Some(path) = model_havok_path(placement, kind, false) else {
            failed += 1;
            eprintln!("Collision {}: unsupported resource recipe", placement.id);
            continue;
        };
        if body.extra_hd_material.is_some() {
            eprintln!(
                "Collision {}: primary shape only; ExtraHD acquisition still unsupported",
                placement.id
            );
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
        let body_basis =
            physworld::rigid_body_basis(placement.rotation.map(|[x, y, z]| [x, y, z, 0.0]));
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

fn native_render_indices(indices: &[u16]) -> Indices {
    // FUN_1402c53e0 uses clockwise fronts (D3D FrontCounterClockwise=0).
    // Imported DH2 triangles are CW; Bevy's raster fronts are CCW. Convert
    // render indices once, preserving native normals and collision indices.
    assert_eq!(indices.len() % 3, 0, "incomplete native triangle");
    Indices::U16(
        indices
            .chunks_exact(3)
            .flat_map(|t| [t[0], t[2], t[1]])
            .collect(),
    )
}

fn mirrored_render_mesh(mut mesh: Mesh, indices: &[u16], flipped: bool) -> Mesh {
    // A reflected affine instance reverses winding again. Preserve the native
    // CW indices for that variant so its final raster fronts remain CCW.
    if flipped {
        mesh.insert_indices(Indices::U16(indices.to_vec()));
    }
    mesh
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
    .with_inserted_indices(native_render_indices(indices))
}

fn mean_game_position(points: &[Vec3]) -> [f32; 3] {
    let mean = points.iter().copied().sum::<Vec3>() / points.len().max(1) as f32;
    view_to_game(mean)
}

fn authored_mesh<const N: usize>(
    points: &[Vec3],
    uvs: &[[f32; 2]],
    indices: &[u16],
    packed: &[[u8; N]],
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
        if N == 16 {
            colors.push(std::array::from_fn::<_, 4, _>(|i| p[12 + i] as f32 / 255.0));
        }
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        points.iter().map(|p| p.to_array()).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs.to_vec())
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, uv1)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_TANGENT, tangents)
    .with_inserted_attribute(DH2_BITANGENT, bitangents)
    .with_inserted_indices(native_render_indices(indices));
    if N == 16 {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    let flipped = transform_direction(Vec3::X)
        .cross(transform_direction(Vec3::Y))
        .dot(transform_direction(Vec3::Z))
        < 0.0;
    mirrored_render_mesh(mesh, indices, flipped)
}

fn source_mesh<const N: usize>(
    positions: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u16],
    packed: &[[u8; N]],
    flipped: bool,
) -> Mesh {
    let points: Vec<Vec3> = positions.iter().copied().map(game_to_view).collect();
    mirrored_render_mesh(
        authored_mesh(&points, uvs, indices, packed, |n| {
            game_to_view(n.to_array())
        }),
        indices,
        flipped,
    )
}

/// VS14200 consumes original game-local POSITION, SNORM NORMAL.xy and UNORM
/// COLOR.xyz before wind and placement. Forward positions/normals may already
/// include axis conversion, placement and shear, and cannot supply those inputs.
/// These additional attributes are uploaded only for actual vegetation programs;
/// the forward mesh and its winding/UVs/basis remain unchanged.
fn retain_native_world_shadow_channels<const N: usize>(
    mesh: &mut Mesh,
    positions: &[[f32; 3]],
    packed: &[[u8; N]],
) -> Result<usize, String> {
    if N != 12 && N != 16 {
        return Err(format!("unsupported native world channel stride {N}"));
    }
    if positions.len() != packed.len() || mesh.count_vertices() != positions.len() {
        return Err("native world source/forward vertex counts disagree".into());
    }
    let normals: Vec<[f32; 2]> = packed
        .iter()
        .map(|p| [p[4], p[5]].map(|b| ((b as i8 as f32) / 127.0).max(-1.0)))
        .collect();
    mesh.insert_attribute(
        native_shadow_scene::NATIVE_VEGETATION_POSITION,
        positions.to_vec(),
    );
    mesh.insert_attribute(native_shadow_scene::NATIVE_VEGETATION_NORMAL, normals);
    if N == 16 {
        let colors: Vec<[f32; 3]> = packed
            .iter()
            .map(|p| [p[12], p[13], p[14]].map(|b| b as f32 / 255.0))
            .collect();
        mesh.insert_attribute(native_shadow_scene::NATIVE_VEGETATION_COLOR, colors);
    } else {
        mesh.remove_attribute(native_shadow_scene::NATIVE_VEGETATION_COLOR);
    }
    Ok(positions.len())
}

fn retain_native_vegetation_channels<const N: usize>(
    mesh: &mut Mesh,
    positions: &[[f32; 3]],
    packed: &[[u8; N]],
) -> Result<(), String> {
    if N != 16 {
        return Err(format!(
            "native vegetation requires authored COLOR; {N}-byte channels have no COLOR"
        ));
    }
    retain_native_world_shadow_channels(mesh, positions, packed).map(|_| ())
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

/// Resolve the slot-admission fields from the same dictionaries as 1402d5210.
/// In particular +0x48 bit4 is MAIN-fallback isDecal, not invisible. This is
/// source selection only, not a claim that all other material flags are loaded.
fn resolve_native_shadow_source_inputs(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    program: &[u8],
    main_program: Option<&str>,
) -> Result<material::ShadowSourceInputs, String> {
    let mut source = material::shadow_source_inputs(bytes, program)?;
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let main = main_program.ok_or("shadow source owner has no main program")?;
    let state = library
        .definition(main.as_bytes())?
        .ok_or_else(|| format!("missing native main program {main}"))?
        .native_render_state();
    // 1402d662d passes all nine MAIN words to 1402c3bc0. A line scan
    // misses compact state blocks and can mistake nested option names for
    // drawing commands, changing native shadow-bank admission.
    let material_state =
        render_state::material_render_state(&String::from_utf8_lossy(bytes), state)?;
    source.main_blend_state = material_state[0];
    source.main_render_state = Some(material_state);
    // These constructor fields precede per-pass specialization. Read the
    // material dictionary, not a block already overwritten for shadow slot 10.
    let overrides = native_material_option_overrides(library, bytes)?;
    let no_shadow = library.registered_option_value(&overrides, b"noShadow")? != 0;
    let is_decal = library.option_value(main.as_bytes(), &overrides, b"isDecal")? & 1 != 0;
    source.hair_type = library.option_value(main.as_bytes(), &overrides, b"hairType")? & 1 != 0;
    source.sky_type = library.option_value(main.as_bytes(), &overrides, b"skyType")? as u32 & 3;
    source.is_cloth = library.option_value(main.as_bytes(), &overrides, b"isCloth")? & 1 != 0;
    source.needs_wind = library.option_value(main.as_bytes(), &overrides, b"needsWind")? & 1 != 0;
    source.source = material::shadow_program_source(
        false,
        no_shadow,
        is_decal,
        source.override_program.is_some(),
        source.hair_type,
        source.main_blend_state,
        source.is_cloth,
    );
    source.cloth_source = material::shadow_program_source(
        true,
        no_shadow,
        is_decal,
        source.cloth_override_program.is_some(),
        source.hair_type,
        source.main_blend_state,
        source.is_cloth,
    );
    // 1402d6e59..1402d6f39 subsequently clears rigid slot 10 in ALL three
    // banks. An authored shadow override does not bypass this final stage.
    // MAIN resolved above; a HasParm miss is the native implicit-main path.
    let class = material::material_program_class(
        true,
        material::authored_program_override(bytes, "mainProgram").is_none(),
        source.main_blend_state,
        library.option_value(main.as_bytes(), &overrides, b"invisible")? != 0,
    );
    if material::finalize_material_programs(class, source.hair_type, source.sky_type).clear_slots
        & (1 << 10)
        != 0
    {
        source.source = material::ShadowProgramSource::Disabled;
    }
    Ok(source)
}

/// Native material +0x48 bit0 is the finalized base bank's slot10 pointer,
/// not the world bank2 pointer and not the subset of shader ABIs we support.
/// All current level sources are resident; a missing cache row still requires
/// the original runtime compile path and must not become `false` here.
fn resolve_native_shadow_slot_presence(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
) -> Result<bool, String> {
    let name = match source.source {
        material::ShadowProgramSource::Disabled => return Ok(false),
        material::ShadowProgramSource::Authored => source.override_program.as_deref(),
        material::ShadowProgramSource::MainProgram => main_program,
    }
    .ok_or("shadow slot owner has no resolved program name")?;
    if name.is_empty() {
        return Ok(false);
    }
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let overrides = native_shadow_variant_overrides(library, bytes, source, main_program, name)?;
    if library
        .serialized_variant(name.as_bytes(), &overrides)?
        .is_none()
    {
        return Err(format!(
            "native base shadow slot needs runtime compilation: {name}"
        ));
    }
    Ok(true)
}

fn resolve_native_rigid_shadow_source(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
) -> Result<Option<std::sync::Arc<shader_cache::CachedRigidShadowSource>>, String> {
    let name = match source.source {
        material::ShadowProgramSource::Disabled => return Ok(None),
        material::ShadowProgramSource::Authored => source.override_program.as_deref(),
        material::ShadowProgramSource::MainProgram => main_program,
    }
    .ok_or("shadow source has no resolved program name")?;
    // HasParm with an authored null source is not a request for the main shader.
    if name.is_empty() {
        return Ok(None);
    }
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let overrides = native_shadow_variant_overrides(library, bytes, source, main_program, name)?;
    library
        .rigid_shadow_source(name.as_bytes(), &overrides)?
        .map(Some)
        .ok_or_else(|| format!("no resident rigid shadow variant for {name}"))
}

/// FUN_1402d5210 bank2 at 1402d66a0..1402d66f0. The direct-instancing
/// exclusion is read BEFORE pass overrides and uses registered defaults,
/// not a MAIN-fallback value. Resident absence is an error, not "no shadow".
fn resolve_native_world_shadow_source(
    library: Option<&mut Result<shader_cache::NativeProgramLibrary, String>>,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
) -> Result<Option<std::sync::Arc<shader_cache::CachedWorldShadowSource>>, String> {
    let name = match source.source {
        material::ShadowProgramSource::Disabled => return Ok(None),
        material::ShadowProgramSource::Authored => source.override_program.as_deref(),
        material::ShadowProgramSource::MainProgram => main_program,
    }
    .ok_or("world shadow source has no resolved program name")?;
    if name.is_empty() {
        return Ok(None);
    }
    let library = library
        .ok_or("native program library is unavailable")?
        .as_mut()
        .map_err(|error| format!("native program library: {error}"))?;
    let original = native_material_option_overrides(library, bytes)?;
    let multi = library.registered_option_value(&original, b"useMultiInstancing")? != 0;
    let direct = library.registered_option_value(&original, b"useDirectInstancing")? != 0;
    if material::instancing_bank_override(2, multi, direct)?.is_none() {
        return Ok(None);
    }
    let mut overrides =
        native_shadow_variant_overrides(library, bytes, source, main_program, name)?;
    let value = render_program::SerializedParameterValue::Words([1, 0, 0, 0]);
    if let Some(parameter) = overrides
        .parameters
        .iter_mut()
        .find(|p| p.name.eq_ignore_ascii_case(b"useMultiInstancing"))
    {
        parameter.value = value;
    } else {
        overrides
            .parameters
            .push(render_program::SerializedParameter {
                name: b"useMultiInstancing".to_vec(),
                kind: render_program::RenderParameterKind::Option,
                value,
            });
    }
    library
        .world_shadow_source(name.as_bytes(), &overrides)?
        .map(Some)
        .ok_or_else(|| format!("no resident world shadow variant for {name}"))
}

fn resolve_native_shadow_material_parameters(
    library: &mut shader_cache::NativeProgramLibrary,
    catalog: &archive::Catalog,
    bytes: &[u8],
    class: shader_cache::CachedRigidShadowClass,
    model_depth:bool,
) -> Result<NativeShadowMaterialParameters, String> {
    use shader_cache::CachedRigidShadowClass;
    let names: &[&[u8]] = match class {
        CachedRigidShadowClass::Opaque | CachedRigidShadowClass::VistaAlpha => &[],
        CachedRigidShadowClass::AlphaTested => &[b"layer0_scalebias"],
        CachedRigidShadowClass::Vegetation => &[
            b"layer0_scalebias",
            b"vegetanim/mainbendingvar",
            b"vegetanim/mainbendingfreq",
            b"vegetanim/mainbendingamplitude",
            b"vegetanim/leafminwindstrength",
            b"vegetanim/leaffreq",
            b"vegetanim/leafamplitude",
            b"vegetanim/branchamplitude",
        ],
    };
    let mut names=names.to_vec();
    if model_depth {names.push(b"modeldepthhack");}
    let values = entities::material_float_parameters(&entities::native_declaration_text(bytes), |name| {
        names.iter().any(|required| *required == name.as_bytes())
    })?;
    let overrides = render_program::SerializedParameterBlock {
        flags: 0,
        parameters: values
            .into_iter()
            .map(|(name, words)| render_program::SerializedParameter {
                name: name.into_bytes(),
                kind: render_program::RenderParameterKind::Uniform,
                value: render_program::SerializedParameterValue::Words(words),
            })
            .collect(),
    };
    let empty = render_program::SerializedParameterBlock {
        flags: 0,
        parameters: Vec::new(),
    };
    let defaults = names
        .iter()
        .map(|name| {
            library
                .registered_float_vector(catalog, &empty, name, None)
                .map(|words| (name.to_vec(), words))
        })
        .collect::<Result<_, _>>()?;
    Ok(NativeShadowMaterialParameters {
        overrides,
        defaults,
    })
}

fn native_material_option_overrides(
    library: &shader_cache::NativeProgramLibrary,
    bytes: &[u8],
) -> Result<render_program::SerializedParameterBlock, String> {
    let values = material::material_option_parameters(bytes, |name| {
        library.parameter_kinds.get(name.as_bytes())
            == Some(&render_program::RenderParameterKind::Option)
    })?;
    Ok(native_option_block(values))
}

fn native_option_block(values: Vec<(String, i32)>) -> render_program::SerializedParameterBlock {
    render_program::SerializedParameterBlock {
        flags: 0,
        parameters: values
            .into_iter()
            .map(|(name, value)| render_program::SerializedParameter {
                name: name.into_bytes(),
                kind: render_program::RenderParameterKind::Option,
                value: render_program::SerializedParameterValue::Words([value as u32, 0, 0, 0]),
            })
            .collect(),
    }
}

fn native_shadow_variant_overrides(
    library: &mut shader_cache::NativeProgramLibrary,
    bytes: &[u8],
    source: &material::ShadowSourceInputs,
    main_program: Option<&str>,
    selected_program: &str,
) -> Result<render_program::SerializedParameterBlock, String> {
    let original = native_material_option_overrides(library, bytes)?;
    // Material +0x4b bit4 uses MAIN, even with an authored shadow source.
    let main = main_program.ok_or("shadow alpha-test owner has no main program")?;
    let alpha_test = library.option_value(main.as_bytes(), &original, b"alphaTest")?;
    // isDecal uses MAIN, but decalType is material/registered-default only.
    let is_decal = library.option_value(main.as_bytes(), &original, b"isDecal")? & 1 != 0;
    let decal_type = library.registered_option_value(&original, b"decalType")? as u32;
    let is_alpha = material::shadow_is_alpha(
        source.main_blend_state,
        is_decal,
        decal_type,
        source.sky_type,
    );
    // 1402d68bb..1402d697d: R15B starts as isAlpha, then ORs three
    // nonzero tests. Each getter uses the SELECTED program +0x2b8 and
    // the UNMODIFIED material block. Sky type is not noScreenShadows.
    // Evaluate every getter even when an earlier operand is true.
    let no_screen_shadows = [
        b"noScreenShadows".as_slice(),
        b"translucency_hq/enable",
        b"translucency/enable",
    ]
    .into_iter()
    .try_fold(is_alpha, |combined, name| {
        library
            .option_value(selected_program.as_bytes(), &original, name)
            .map(|value| combined | (value != 0))
    })?;
    let values = material::rigid_shadow_option_parameters(
        bytes,
        alpha_test,
        is_alpha,
        no_screen_shadows,
        |name| {
            library.parameter_kinds.get(name.as_bytes())
                == Some(&render_program::RenderParameterKind::Option)
        },
    )?;
    Ok(native_option_block(values))
}

struct MaterialResolver<'a> {
    catalog: Option<&'a archive::Catalog>,
    native_programs: Option<&'a mut Result<shader_cache::NativeProgramLibrary, String>>,
    probe_materials: &'a mut Assets<ProbeMaterial>,
    decal_materials: &'a mut Assets<DecalMaterial>,
    probe_sh_buffers: &'a mut Assets<ShaderBuffer>,
    probe_sh_buffer: Handle<ShaderBuffer>,
    vertex_probe_buffer: Handle<ShaderBuffer>,
    native_shadow_buffer: Handle<ShaderBuffer>,
    native_local_shadow_buffer: Handle<ShaderBuffer>,
    native_shadow_map: Handle<Image>,
    probe_sh_records: Vec<[Vec4; 9]>,
    probes: Option<&'a probe_lighting::ProbeLightingSet>,
    cache: HashMap<String, Option<CachedMaterial>>,
    shadow_materials: NativeShadowMaterials,
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
    fn native_caster_admission(&mut self,path:&str)->Result<u8,String> {
        let catalog=self.catalog.ok_or("native caster material catalog is unavailable")?;
        let bytes=catalog.read(&format!("generated/decls/material/{path}.decl"))
            .map_err(|error|error.to_string())?.ok_or_else(||format!("native caster material is not indexed: {path}"))?;
        let main=material_surface::Surface::parse(&bytes).main_program;
        let program=main.as_ref().map(|name|catalog.read(&format!(
            "generated/decls/renderprog/{}.decl",name.to_ascii_lowercase()))
            .map_err(|error|error.to_string())?.ok_or_else(||format!("native MAIN program missing: {name}")))
            .transpose()?.unwrap_or_default();
        let source=resolve_native_shadow_source_inputs(self.native_programs.as_deref_mut(),
            &bytes,&program,main.as_deref())?;
        let present=resolve_native_shadow_slot_presence(self.native_programs.as_deref_mut(),
            &bytes,&source,main.as_deref())?;
        Ok(u8::from(present)|(u8::from(source.needs_wind)<<6))
    }
    fn append_probe_record(&mut self, game_position: [f32; 3]) -> u32 {
        let sh = if self.lit {
            match self
                .probes
                .and_then(|probes| probes.interpolate_static_sh(game_position))
            {
                Some(lighting) => {
                    self.probe_samples += 1;
                    lighting.render_lighting_sh
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
            && !(0..=self.preferred_mip.unwrap_or(15))
                .any(|mip| catalog.contains(&format!("{stem}_mip{mip}")))
        {
            return None;
        }
        if let Some(&slot) = self.diffuse_image_slots.get(&stem) {
            return Some(slot);
        }
        let slot = self.diffuse_image_paths.len();
        self.diffuse_image_paths.push(stem.clone());
        self.diffuse_image_slots.insert(stem, slot);
        Some(slot)
    }

    fn get(
        &mut self,
        path: &str,
        _color_seed: usize,
        game_position: [f32; 3],
    ) -> Option<ResolvedMaterial> {
        if let Some(cached) = self.cache.get(path) {
            let cached = cached.clone()?;
            let probe_index = self.append_probe_record(game_position);
            return Some(ResolvedMaterial {
                handle: cached.handle,
                diffuse_image_slot: cached.diffuse_image_slot,
                probe_index,
                native_vertex_lighting: cached.native_vertex_lighting,
            });
        }
        let key = path.to_owned();
        let bytes = self.catalog.and_then(|c| {
            c.read(&format!("generated/decls/material/{path}.decl"))
                .ok()
                .flatten()
        });
        let bytes = bytes.as_deref().unwrap_or(&[]);
        if material::is_screen_gate_impostor(path, bytes) {
            self.cache.insert(key, None);
            return None;
        }
        let surface = material_surface::Surface::parse(bytes);
        if surface.extra_layers > 2
            || surface
                .layers
                .iter()
                .any(|l| l.mask_channel > 3 || l.mask_mode > 2)
        {
            eprintln!("Unsupported material layer inputs {path}");
            self.cache.insert(key, None);
            return None;
        }
        let mut uniform = surface_gpu::SurfaceUniform::from(&surface);
        let program = surface.main_program.as_ref().and_then(|name| {
            self.catalog.and_then(|c| {
                c.read(&format!(
                    "generated/decls/renderprog/{}.decl",
                    name.to_ascii_lowercase()
                ))
                .ok()
                .flatten()
            })
        });
        let program = program.as_deref().unwrap_or(&[]);
        if matches!(uniform.misc.z,1|2) {
            let receiver=(||->Result<UVec4,String>{
                let source=resolve_native_shadow_source_inputs(self.native_programs.as_deref_mut(),bytes,program,surface.main_program.as_deref())?;
                let library=self.native_programs.as_deref_mut().ok_or("native program library is unavailable")?
                    .as_mut().map_err(|error|error.clone())?;
                let main=surface.main_program.as_deref().ok_or("native receiver MAIN is absent")?;
                let overrides=native_shadow_variant_overrides(library,bytes,&source,Some(main),main)?;
                let mut option=|name:&[u8]|library.option_value(main.as_bytes(),&overrides,name);
                let direct=option(b"noScreenShadows")?!=0;
                let flags=u32::from(option(b"shadow/useNormalOffset")?!=0)
                    |(u32::from(option(b"shadow/useSlopeScale")?!=0)<<1)
                    |(u32::from(option(b"shadow/useSlopeScaleBias")?!=0)<<2)
                    |(u32::from(option(b"shadow/slopeScaleIsPostProjection")?!=0)<<3)
                    |(u32::from(option(b"translucency/enable")?==0)<<4);
                Ok(UVec4::new(u32::from(direct),flags,option(b"shadowFilteringQuality")? as u32,2))
            })();
            match receiver {Ok(value)=>uniform.native_shadow=value,Err(error)=>eprintln!("Native forward shadow receiver unavailable for {path}: {error}")}
        }
        // FUN_1402d5210 encodes inherited `invisible` as material+0x4b's
        // excluded state; FUN_14033b350 rejects it before creating any rows.
        if material::program_option(bytes, program, "invisible") {
            self.cache.insert(key, None);
            return None;
        }
        let transparency = material::transparency_with_program(bytes, program);
        let program_is_decal = material::program_option(bytes, program, "isDecal");
        let two_sided_lighting = material::program_option(bytes, program, "twoSidedLighting");
        let alpha_test = material::program_option(bytes, program, "alphaTest");
        uniform.misc.w = u32::from(alpha_test)
            | (u32::from(program_is_decal) << 1)
            | (u32::from(two_sided_lighting) << 2);
        if env::var("DH2_TRACE_MATERIAL").ok().as_deref()==Some(path) {
            println!("NATIVE_MATERIAL {path}: {uniform:#?}");
        }
        let decal_bias = material::decal_depth_bias(bytes);
        let mut images = Vec::new();
        for index in 0..=surface.extra_layers {
            let layer = &surface.layers[index];
            let candidates = [
                (
                    surface_gpu::ImageRole::diffuse(index),
                    layer.diffuse.as_deref(),
                    index == 0 || layer.reuse[0] == 0,
                ),
                (
                    surface_gpu::ImageRole::bump(index),
                    layer.bump.as_deref(),
                    surface.has_bump && (index == 0 || layer.reuse[1] == 0),
                ),
                (
                    surface_gpu::ImageRole::gloss(index),
                    layer.gloss.as_deref(),
                    surface.has_gloss
                        && !surface.gloss_in_alpha
                        && (index == 0 || layer.reuse[2] == 0),
                ),
                (
                    surface_gpu::ImageRole::metallic(index),
                    layer.metallic.as_deref(),
                    surface.has_metallic && (index == 0 || layer.reuse[3] == 0),
                ),
                (surface_gpu::ImageRole::specular(index),layer.specular.as_deref(),
                    surface.has_specular && (index==0 || layer.specular_reuse==0)),
                (
                    surface_gpu::ImageRole::mask(index),
                    layer.mask.as_deref(),
                    (index != 0 || layer.use_color) && layer.mask_mode != 0 && layer.reuse[4] == 0,
                ),
            ];
            for (role, name, enabled) in candidates {
                if !enabled {
                    continue;
                }
                if let Some(name) = name {
                    if let Some(slot) = self.register_image(name) {
                        images.push((slot, role));
                    } else {
                        eprintln!("Material image missing {path} role={} name={name}", role.0);
                    }
                }
            }
        }
        // HQ/AQ AO samples the red channel at layer-zero UVs. LQ decals
        // return 1.0 and do not request this texture.
        if surface.has_occlusion && !program_is_decal {
            if let Some(name) = surface.occlusion.as_deref() {
                if let Some(slot) = self.register_image(name) {
                    images.push((slot, surface_gpu::ImageRole::occlusion()));
                } else {
                    eprintln!("Material image missing {path} role=15 name={name}");
                }
            }
        }
        let diffuse_image_slot = images
            .iter()
            .find(|(_, role)| role.0 == 0)
            .map(|&(slot, _)| slot);
        if diffuse_image_slot.is_none() {
            self.misses += 1;
        }
        let alpha_mode = match transparency {
            material::Transparency::Opaque => AlphaMode::Opaque,
            material::Transparency::Cutout => {
                self.cutout += 1;
                AlphaMode::Mask(surface.forward_alpha_cutoff())
            }
            material::Transparency::Coverage => {
                if surface
                    .main_program
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case("arkDecal"))
                {
                    self.blended += 1;
                    AlphaMode::Blend
                } else {
                    self.cutout += 1;
                    AlphaMode::AlphaToCoverage
                }
            }
            material::Transparency::Blend => {
                self.blended += 1;
                AlphaMode::Blend
            }
        };
        let is_decal = program_is_decal || decal_bias.is_some();
        let cull_mode = match material::cull_mode_with_program(bytes, program) {
            Ok(material::CullMode::None) => None,
            Ok(material::CullMode::Front) => Some(bevy::render::render_resource::Face::Front),
            Ok(material::CullMode::Back) => Some(bevy::render::render_resource::Face::Back),
            Err(error) => {
                eprintln!("Material {path}: {error}");
                self.misses += 1;
                return None;
            }
        };
        let base = StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: None,
            alpha_mode,
            depth_bias: 0.0,
            unlit: !self.lit || path.contains("/vista/") || path.contains("/skies/"),
            perceptual_roughness: 1.0 - surface.ward[1],
            metallic: surface.metallic_min_max[1],
            double_sided: two_sided_lighting,
            cull_mode,
            ..default()
        };
        let native_vertex_lighting = matches!(uniform.misc.z, 1 | 2);
        let handle = if is_decal {
            MaterialHandle::Decal(self.decal_materials.add(DecalMaterial {
                base,
                extension: DecalExtension {
                    bias: decal_bias.unwrap_or(1678.0).round() as i32,
                    sh:self.probe_sh_buffer.clone(),
                    vertex_probes:self.vertex_probe_buffer.clone(),
                    native_shadow: self.native_shadow_buffer.clone(),
                    native_local_shadow: self.native_local_shadow_buffer.clone(),
                    native_shadow_map: Some(self.native_shadow_map.clone()),
                    native_local_shadow_map: Some(self.native_shadow_map.clone()),
                    native_screen_map: None,
                    native_cloud_map: None,
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
                    specular0:None,specular1:None,specular2:None,
                },
            }))
        } else {
            MaterialHandle::Probe(self.probe_materials.add(ProbeMaterial {
                base,
                extension: ProbeExtension {
                    sh: self.probe_sh_buffer.clone(),
                    vertex_probes: self.vertex_probe_buffer.clone(),
                    native_shadow: self.native_shadow_buffer.clone(),
                    native_local_shadow: self.native_local_shadow_buffer.clone(),
                    native_local_shadow_map: Some(self.native_shadow_map.clone()),
                    native_shadow_map: Some(self.native_shadow_map.clone()),
                    native_screen_map: None,
                    native_cloud_map: None,
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
                    specular0:None,specular1:None,specular2:None,
                },
            }))
        };
        let shadow = resolve_native_shadow_source_inputs(
            self.native_programs.as_deref_mut(),
            bytes,
            program,
            surface.main_program.as_deref(),
        )
        .map(|source| {
            let base_slot10_present = resolve_native_shadow_slot_presence(
                self.native_programs.as_deref_mut(),
                bytes,
                &source,
                surface.main_program.as_deref(),
            );
            let rigid_base_source = resolve_native_rigid_shadow_source(
                self.native_programs.as_deref_mut(),
                bytes,
                &source,
                surface.main_program.as_deref(),
            );
            let world_source = resolve_native_world_shadow_source(
                self.native_programs.as_deref_mut(),
                bytes,
                &source,
                surface.main_program.as_deref(),
            );
            let world_parameters = match &world_source {
                Ok(None) => Ok(None),
                Err(error) => Err(error.clone()),
                Ok(Some(compiled)) => (|| {
                    let catalog = self
                        .catalog
                        .ok_or("native world shadow parameter catalog is unavailable")?;
                    let library = self
                        .native_programs
                        .as_deref_mut()
                        .ok_or("native program library is unavailable")?
                        .as_mut()
                        .map_err(|error| error.clone())?;
                    let class = compiled.class.material_parameter_class();
                    resolve_native_shadow_material_parameters(library, catalog, bytes, class,false)
                        .map(Some)
                })(),
            };
            let parameters = match &rigid_base_source {
                Ok(None) => Ok(None),
                Err(error) => Err(error.clone()),
                Ok(Some(compiled)) => (|| {
                    let catalog = self
                        .catalog
                        .ok_or("native shadow parameter catalog is unavailable")?;
                    let library = self
                        .native_programs
                        .as_deref_mut()
                        .ok_or("native program library is unavailable")?
                        .as_mut()
                        .map_err(|error| error.clone())?;
                    resolve_native_shadow_material_parameters(
                        library,
                        catalog,
                        bytes,
                        compiled.class,
                        true,
                    )
                    .map(Some)
                })(),
            };
            NativeShadowMaterialInputs {
                source,
                rigid_base_source,
                base_slot10_present,
                world_source,
                world_parameters,
                main_program: surface.main_program.clone(),
                alpha_test,
                world_uv_set: surface.layers.first().map(|layer| layer.uv_set),
                diffuse_image_slot,
                parameters,
                cull_mode,
            }
        });
        self.shadow_materials.0.insert(handle.clone(), shadow);
        let cached = CachedMaterial {
            handle,
            diffuse_image_slot,
            images,
            native_vertex_lighting,
        };
        self.cache.insert(key, Some(cached.clone()));
        let probe_index = self.append_probe_record(game_position);
        Some(ResolvedMaterial {
            handle: cached.handle,
            diffuse_image_slot: cached.diffuse_image_slot,
            probe_index,
            native_vertex_lighting: cached.native_vertex_lighting,
        })
    }
}

fn setup(
    mut commands: Commands,
    mut scene: ResMut<SceneData>,
    mut native_programs: Option<ResMut<NativeProgramCacheSource>>,
    navigation: Option<Res<NavigationWorld>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut probe_materials: ResMut<Assets<ProbeMaterial>>,
    mut probe_sh_buffers: ResMut<Assets<ShaderBuffer>>,
    render_device: Res<RenderDevice>,
    mut decal_materials: ResMut<Assets<DecalMaterial>>,
    mut exposure_curves: ResMut<Assets<AutoExposureCompensationCurve>>,
    view_settings: Res<NativeViewSettings>,
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
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.7)),
            ZIndex(100),
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("FPS: measuring"),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                FrameRateOverlay,
            ));
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
    let trace_bwm_lod = env::var("DH2_TRACE_BWM_LOD").as_deref() == Ok("1");
    if trace_bwm_lod {
        eprintln!(
            "BWM LOD visibility trace enabled; reports appear when authored-tag visibility counts change"
        );
    }
    if trace_startup {
        eprintln!(
            "Startup trace: beginning world setup ({} BWM objects, {} world meshes, {} placed models)",
            scene.world.objects.len(),
            scene.world.meshes.len(),
            scene.placed_models.len()
        );
    }
    // Preserve ordered primary probe IDs in binding100; per-object records
    // are appended after them. World geometry uses its PNW instance offset.
    let mut records = scene
        .environment_probes
        .as_ref()
        .map(|set| {
            set.probes
                .iter()
                .map(|probe| {
                    probe.render_lighting_sh.map(|v| {
                        if lit {
                            Vec4::new(v[0], v[1], v[2], 0.0)
                        } else {
                            Vec4::ZERO
                        }
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if records.is_empty() {
        records.push([Vec4::ZERO; 9]);
    }
    let words = scene
        .environment_probes
        .as_ref()
        .map(probe_lighting::ProbeLightingSet::vertex_shader_words)
        .unwrap_or_else(|| vec![0u32; 8]);
    let bytes = words.len() as u64 * 4;
    let limits = render_device.limits();
    assert!(
        bytes <= u64::from(limits.max_storage_buffer_binding_size)
            && bytes <= limits.max_buffer_size,
        "native PNW vertex lighting requires a {bytes}-byte buffer; device storage limit={}, buffer limit={}",
        limits.max_storage_buffer_binding_size,
        limits.max_buffer_size
    );
    println!(
        "Native vertex-probe GPU data: {} primary records, {bytes} bytes (device storage limit {})",
        records.len(),
        limits.max_storage_buffer_binding_size
    );
    let vertex_probe_buffer = probe_sh_buffers.add(ShaderBuffer::from(words));
    let probe_sh_buffer = probe_sh_buffers.add(ShaderBuffer::from(records.clone()));
    let native_shadow_buffer = probe_sh_buffers.add(ShaderBuffer::from(
        native_shadow_sampling::NativeParallelSamplingUniform::default(),
    ));
    let native_shadow_map = images.add(
        native_shadow_gpu::shadow_depth_image(
            bevy::render::render_resource::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureFormat::Depth24PlusStencil8,
        )
        .expect("unavailable depth binding descriptor"),
    );
    commands.insert_resource(native_shadow_sampling::NativeParallelSamplingBuffer {
        buffer: native_shadow_buffer.clone(),
        unavailable_depth: native_shadow_map.clone(),
    });
    let local_capacity=scene.placed_lights.len().max(1).next_power_of_two() as u32;
    let native_local_shadow_buffer=probe_sh_buffers.add(ShaderBuffer::from(vec![0u32;
        native_shadow_sampling::local_sampling_word_count(local_capacity)]));
    commands.insert_resource(native_shadow_sampling::NativeLocalSamplingBuffer {
        buffer:native_local_shadow_buffer.clone(),light_capacity:local_capacity,
    });
    let mut resolver = MaterialResolver {
        catalog,
        native_programs: native_programs.as_deref_mut().map(|source| &mut source.0),
        probe_materials: &mut probe_materials,
        decal_materials: &mut decal_materials,
        probe_sh_buffers: &mut probe_sh_buffers,
        probe_sh_buffer,
        vertex_probe_buffer,
        native_shadow_buffer,
        native_local_shadow_buffer,
        native_shadow_map,
        probe_sh_records: records,
        probes: scene.environment_probes.as_ref(),
        cache: HashMap::new(),
        shadow_materials: NativeShadowMaterials::default(),
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
    let mut native_world_shadow_mesh_sources =
        NativeWorldShadowMeshSources::prepare(&scene.world, &mut meshes);
    let mut native_world_admissions = HashMap::<String,Result<u8,String>>::new();
    let mut native_vegetation_meshes = NativeVegetationMeshInputs::default();
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
        // Preserve every serialized BWM representation until the native
        // model/world draw selector is traced. FUN_140a82710 is the Tome
        // object-visibility query implemented in tome.rs, not an LOD selector;
        // filtering to LOD 0 here would discard authored representations.
        let skip_color = material::is_editor_helper(&object.material_path)
            || object.material_path == "models/campaign/boat_shared/boat_gabarit_01.material";
        let source = &scene.world.meshes[object.mesh_index];
        // The auxiliary whole-view pass owns native +0x18 material pointers
        // independently of the color-visible render entities. Resolve this
        // object's actual material even when its debug/helper color mesh is
        // intentionally absent; never turn that absence into a no-caster flag.
        let admission = native_world_admissions.entry(object.material_path.clone())
            .or_insert_with(||resolver.native_caster_admission(&object.material_path)).clone();
        native_world_shadow_mesh_sources.objects[object_number].gather_flags48 = Some(admission);
        let first_material = if let Some(matrix) = scene.world.matrices.get(object.matrix_start) {
            let points: Vec<Vec3> = source
                .positions
                .iter()
                .map(|&position| transformed(matrix, position))
                .collect();
            resolver.get(
                &object.material_path,
                object_number,
                mean_game_position(&points),
            )
        } else {
            None
        };
        if let Some(material) = &first_material {
            native_world_shadow_mesh_sources
                .bind_material(object_number, &material.handle)
                .expect("resolved native BWM object/material ownership");
        } else {
            native_world_shadow_mesh_sources
                .record_material_error(
                    object_number,
                    format!(
                        "native BWM object {object_number} material {} could not be resolved",
                        object.material_path
                    ),
                )
                .expect("prepared native BWM object association");
        }
        // This BWM surface is a boat volume proxy (stageSort sortboatvolume),
        // not the boat's visible hull. It has no normal color draw, but remains
        // associated with its actual native whole-view material source above.
        if skip_color {
            continue;
        }
        // layer0_vtxcolormaskchannel blends material layers, not final opacity.
        // Using it as vertex alpha made 1,059 authored world objects translucent.
        // Reuse the unbaked shared forward mesh, which also retains explicit
        // raw local attributes for native world-shadow vertex declarations.
        let mesh = native_world_shadow_mesh_sources
            .object(object_number)
            .expect("prepared native BWM object association")
            .mesh
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
            let mat = if matrix_index == object.matrix_start {
                first_material.clone()
            } else {
                resolver.get(
                    &object.material_path,
                    object_number,
                    mean_game_position(&points),
                )
            };
            let Some(mat) = mat else {
                native_world_shadow_mesh_sources
                    .record_material_error(
                        object_number,
                        format!(
                            "native BWM object {object_number} material {} could not be resolved",
                            object.material_path
                        ),
                    )
                    .expect("prepared native BWM object association");
                continue;
            };
            native_world_shadow_mesh_sources
                .bind_material(object_number, &mat.handle)
                .expect("resolved BWM object/material ownership");
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
                let baked = meshes.add(authored_mesh(
                    &points,
                    &source.uvs,
                    &source.indices,
                    &source.packed_channels,
                    |v| transformed_direction(matrix, v),
                ));
                let render_instance = RenderInstance {
                    source: Some(RenderInstanceSource {
                        surface: RenderModelSurface::World {
                            object: object_number,
                            matrix: matrix_index,
                        },
                        world_baked: true,
                    }),
                    mesh: baked,
                    material: mat.handle.clone(),
                    diffuse_image_slot: mat.diffuse_image_slot,
                    probe_index: if mat.native_vertex_lighting && scene.environment_probes.is_some()
                    {
                        0x80000000 | matrix_index as u32
                    } else {
                        mat.probe_index
                    },
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
                    source: Some(RenderInstanceSource {
                        surface: RenderModelSurface::World {
                            object: object_number,
                            matrix: matrix_index,
                        },
                        world_baked: false,
                    }),
                    mesh: mesh.clone(),
                    material: mat.handle.clone(),
                    diffuse_image_slot: mat.diffuse_image_slot,
                    probe_index: if mat.native_vertex_lighting && scene.environment_probes.is_some()
                    {
                        0x80000000 | matrix_index as u32
                    } else {
                        mat.probe_index
                    },
                    transform: instance,
                    bounds,
                    pickup_id: None,
                    entity: None,
                    visible: false,
                }
            };
            native_vegetation_meshes.prepare(
                resolver
                    .shadow_materials
                    .uses_vegetation(&render_instance.material),
                &render_instance.mesh,
                &mut meshes,
                &source.positions,
                &source.packed_channels,
            );
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
    let mut model_mesh_handles = HashMap::<(String, usize, bool, u8), Handle<Mesh>>::new();
    let pickup_ids: HashSet<&str> = scene
        .pickups
        .iter()
        .map(|pickup| pickup.entity_id.as_str())
        .collect();
    let mut model_instances = 0usize;
    let mut model_surfaces = 0usize;
    let mut placed_model_queries = Vec::new();
    let mut native_shadow_model_bounds = Vec::new();
    let mut native_model_shadow_sources=Vec::new();
    let mut native_skin_palettes=HashMap::<String,Result<Arc<native_shadow_live::NativeRestSkinning>,String>>::new();
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
                let skinning=native_skin_palettes.entry(placement.resource_path.clone()).or_insert_with(||
                    native_rest_skinning(catalog.as_ref().ok_or("native skin catalog is unavailable")?,
                        &placement.resource_path,model,resolver.probe_sh_buffers)).clone();
                model_instances += 1;
                if model.has_skeleton {
                    md6_rest_pose_instances += 1;
                }
                let model_render_start = render_instances.len();
                let caster_surfaces=model.surfaces.iter().enumerate().map(|(surface_number,surface)| {
                    let path=placement.custom_materials.get(surface_number).and_then(Option::as_deref)
                        .unwrap_or(&surface.material_path);
                    let flags=native_world_admissions.entry(path.to_owned())
                        .or_insert_with(||resolver.native_caster_admission(path)).clone();
                    (surface_number,surface.vertex_count as i32,flags)
                }).collect();
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
                    let world_baked =
                        needs_exact_world_mesh(&instance, &surface.positions, &points);
                    let skin_influences=resolver.shadow_materials.skin_influences(&mat.handle);
                    let (mesh, transform) = if world_baked {
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
                                .entry((
                                    placement.resource_path.clone(),
                                    surface_number,
                                    instance.to_matrix().determinant() < 0.0,
                                    skin_influences,
                                ))
                                .or_insert_with(|| {
                                    meshes.add(source_mesh(
                                        &surface.positions,
                                        &surface.uvs,
                                        &surface.indices,
                                        &surface.packed_channels,
                                        instance.to_matrix().determinant() < 0.0,
                                    ))
                                })
                                .clone(),
                            instance,
                        )
                    };
                    retain_native_model_shadow_position(&mut meshes,&mesh,&surface.positions);
                    if let Some(mut mesh)=meshes.get_mut(&mesh) {
                        retain_native_skin_inputs(&mut mesh,surface);
                        if skin_influences!=0 {
                            match skinning.as_ref().map_err(Clone::clone).and_then(|skin|
                                apply_native_rest_pose(&mut mesh,surface,&skin.matrices,skin_influences,placement,world_baked)) {
                                Ok(())=>{},Err(error)=>eprintln!("Native model rest pose pending {}: {error}",placement.resource_path),
                            }
                        }
                    }
                    native_vegetation_meshes.prepare(
                        resolver.shadow_materials.uses_vegetation(&mat.handle),
                        &mesh,
                        &mut meshes,
                        &surface.positions,
                        &surface.packed_channels,
                    );
                    render_instances.push(RenderInstance {
                        source: Some(RenderInstanceSource {
                            surface: RenderModelSurface::Bmd6 {
                                placement: number,
                                surface: surface_number,
                                cpu_vertex_declaration: surface.cpu_vertex_declaration,
                                gpu_vertex_declaration: surface.gpu_vertex_declaration,
                                skin_influences: surface.skin_influences,
                                matrix_offset: surface.matrix_offset,
                            },
                            world_baked,
                        }),
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
                let caster_bounds=render_model::transformed_bounds(model.authored_bounds,
                    placement.position,placement.rotation,placement.scale);
                native_model_shadow_sources.push(native_shadow_live::NativeModelShadowSource {
                    placement:number,bounds:caster_bounds,
                    skinned:true,surfaces:caster_surfaces,skinning:Some(skinning),
                    instances:model_render_start..render_instances.len(),
                    pickup_id:pickup_ids.contains(placement.id.as_str()).then(||placement.id.clone()),
                    flags_1c30:render_model::model_owner_bounds_flags(0,
                        [caster_bounds[0][0],caster_bounds[0][1],caster_bounds[0][2],0.0],
                        [caster_bounds[1][0],caster_bounds[1][1],caster_bounds[1][2],0.0]),
                    flags_18bc:render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC,flags_18bd:0,
                });
                if model_render_start < render_instances.len() {
                    native_shadow_model_bounds.push(NativeShadowModelBoundsSource {
                        placement: number,
                        instances: model_render_start..render_instances.len(),
                        bounds: render_model::transformed_bounds(
                            model.authored_bounds,
                            placement.position,
                            placement.rotation,
                            placement.scale,
                        ),
                    });
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
        let caster_surfaces=model.surfaces.iter().enumerate().map(|(surface_number,surface)| {
            let path=placement.custom_materials.get(surface_number).and_then(Option::as_deref)
                .unwrap_or(&surface.material_path);
            let flags=native_world_admissions.entry(path.to_owned())
                .or_insert_with(||resolver.native_caster_admission(path)).clone();
            (surface_number,surface.positions.len() as i32,flags)
        }).collect();
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
            let world_baked = needs_exact_world_mesh(&instance, &surface.positions, &points);
            let (mesh, transform) = if world_baked {
                baked_shear_instances += 1;
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
                        .entry((
                            placement.resource_path.clone(),
                            surface_number,
                            instance.to_matrix().determinant() < 0.0,
                            0,
                        ))
                        .or_insert_with(|| {
                            meshes.add(source_mesh(
                                &surface.positions,
                                &surface.uvs,
                                &surface.indices,
                                &surface.packed_channels,
                                instance.to_matrix().determinant() < 0.0,
                            ))
                        })
                        .clone(),
                    instance,
                )
            };
            retain_native_model_shadow_position(&mut meshes,&mesh,&surface.positions);
            native_vegetation_meshes.prepare(
                resolver.shadow_materials.uses_vegetation(&mat.handle),
                &mesh,
                &mut meshes,
                &surface.positions,
                &surface.packed_channels,
            );
            render_instances.push(RenderInstance {
                source: Some(RenderInstanceSource {
                    surface: RenderModelSurface::Bmodel {
                        placement: number,
                        surface: surface_number,
                    },
                    world_baked,
                }),
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
        let caster_bounds=render_model::transformed_bounds(model.authored_bounds,
            placement.position,placement.rotation,placement.scale);
        native_model_shadow_sources.push(native_shadow_live::NativeModelShadowSource {
            placement:number,bounds:caster_bounds,
            skinned:false,surfaces:caster_surfaces,skinning:None,instances:model_render_start..render_instances.len(),
            pickup_id:pickup_ids.contains(placement.id.as_str()).then(||placement.id.clone()),
            flags_1c30:render_model::model_owner_bounds_flags(0,
                [caster_bounds[0][0],caster_bounds[0][1],caster_bounds[0][2],0.0],
                [caster_bounds[1][0],caster_bounds[1][1],caster_bounds[1][2],0.0]),
            flags_18bc:render_model::INITIAL_MODEL_CONTEXT_FLAGS_18BC,flags_18bd:0,
        });
        if let Some(input) = query_input {
            if model_render_start < render_instances.len() {
                native_shadow_model_bounds.push(NativeShadowModelBoundsSource {
                    placement: number,
                    instances: model_render_start..render_instances.len(),
                    bounds: input.bounds,
                });
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
        native_world_shadow_mesh_sources.shared_mesh_count,
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
    let mut reference_horizontal_fov = view_settings.0.as_ref()
        .map(|settings|settings.horizontal_fov_degrees)
        .unwrap_or(ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES);
    if let Some(path) = env::var_os("DH2_REPLAY_F8") {
        let report = fs::read_to_string(&path).expect("cannot read DH2_REPLAY_F8 capture");
        camera = f8_camera_pose(&report).expect("DH2_REPLAY_F8 has an invalid camera pose");
        reference_horizontal_fov = f8_reference_fov(&report)
            .expect("DH2_REPLAY_F8 has an invalid reference horizontal FOV");
        println!(
            "Replaying saved F8 camera from {} at reference FOV {reference_horizontal_fov}",
            PathBuf::from(path).display()
        );
    }
    // Replayed F8 poses are benchmark inputs, not spawn points. Gravity must
    // not move the saved camera while comparing rendering at that same pose.
    let walking = navigation.is_some()
        && scene.player_start.is_some()
        && env::var_os("DH2_REPLAY_F8").is_none();
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
        tome::original_render_vertical_fov_radians(reference_horizontal_fov)
            .expect("the registered game FOV default is valid");
    // The original registers r_multiSamples=0 and clamps physical count to1.
    // Override only in explicit profiling runs for same-source A/B testing.
    let multi_samples = env::var("DH2_PERF_MULTI_SAMPLES")
        .ok()
        .filter(|_| env::var("DH2_PERF_PROFILE").as_deref() == Ok("1"))
        .map(|value| {
            value
                .parse::<i32>()
                .expect("invalid DH2_PERF_MULTI_SAMPLES")
        })
        .unwrap_or(render_model::ORIGINAL_DEFAULT_MULTI_SAMPLES);
    let physical_samples = render_model::original_renderer_sample_count(multi_samples);
    assert!(
        [1, 2, 4, 8].contains(&physical_samples),
        "DH2_PERF_MULTI_SAMPLES={multi_samples} requests {physical_samples} samples; this renderer supports 1, 2, 4, or 8"
    );
    let renderer_msaa = Msaa::from_samples(physical_samples);
    let native_coarse_depth_source = native_coarse_depth::diagnostics_enabled()
        || scene.shadow_cascade_controls.is_some_and(|controls|controls.tighten_parallel_shadows);
    let camera_3d = if native_coarse_depth_source {
        Camera3d {
            depth_texture_usages: (bevy::render::render_resource::TextureUsages::RENDER_ATTACHMENT
                | bevy::render::render_resource::TextureUsages::TEXTURE_BINDING)
                .into(),
            ..default()
        }
    } else {
        Camera3d::default()
    };
    eprintln!(
        "[renderer] r_multiSamples={multi_samples} physical_samples={}",
        renderer_msaa.samples()
    );
    let mut camera_entity = commands.spawn((
        camera_3d,
        Projection::custom(native_camera::NativeGameProjection::new(
            reference_horizontal_fov, ORIGINAL_DEFAULT_NEAR, ORIGINAL_DEFAULT_FAR,
        )),
        camera,
        renderer_msaa,
    ));
    if native_coarse_depth_source {
        camera_entity.insert(native_coarse_depth::NativeCoarseDepthSource);
    }
    // Direct submission avoids Bevy's per-bin indirect-buffer uploads/unpacking.
    // Mesh transforms, shaders and CPU/Tome visibility stay unchanged.
    // Set only at camera creation; DH2_DIRECT_MESH_DRAWS=0 retains the A/B control.
    if env::var("DH2_DIRECT_MESH_DRAWS").as_deref() != Ok("0") {
        camera_entity.insert(bevy::render::view::NoIndirectDrawing);
    }
    // Standalone depth profiling remains explicit. The native screen-shadow
    // owner separately enables its required same-view depth source.
    if env::var("DH2_ZPREPASS").as_deref() == Ok("1") {
        camera_entity.insert(DepthPrepass);
    }
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
        // lightcontext HQ/AQ/LQ_ComputeAmbientLighting explicitly disable
        // the old env/ambient high/low term. Probe irradiance owns ambient;
        // adding Bevy's global ambient would illuminate every shadow again.
        commands.insert_resource(GlobalAmbientLight::NONE);
        let mut spawned = 0;
        let mut unsupported = 0;
        let mut energy_shortened_ranges = 0;
        if scene_lights_enabled {
            for (light_index,light) in scene.placed_lights.iter().enumerate() {
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
                // Native normalized RGB is clamped before scaling. Point/spot
                // and parallel paths convert native HDR with the original
                // world-luminance factor; area profile photometry is separate.
                let color = Color::linear_rgb(
                    linear_color.red.clamp(0.0, 1.0),
                    linear_color.green.clamp(0.0, 1.0),
                    linear_color.blue.clamp(0.0, 1.0),
                );
                // Native area SizeX/Y are half extents (arealight's clipping
                // corners are +/-SizeX/Y); RectLight takes full dimensions.
                let rect_size = light.area_size.map(|size| 2.0 * size.max(0.01));
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
                            native_local_shadow_live::NativeLocalLightSource(light_index),
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
                            native_local_shadow_live::NativeLocalLightSource(light_index),
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
                        // Bevy's spot-light implementation requires a sub-90В°
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
                            native_local_shadow_live::NativeLocalLightSource(light_index),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Area => {
                        // LIGHT_AREA uses the separate paraboloid path in
                        // renderprog/lighting.decl; Bevy RectLight has no matching
                        // shadow path. The authored area profile/projection and
                        // area-specific shadow controls remain unported.
                        let Some(transform)=authored_area_light_transform(position,light.rotation) else {
                            unsupported += 1;
                            continue;
                        };
                        commands.spawn((
                            RectLight {
                                color,
                                intensity,
                                range,
                                width: rect_size[0],
                                height: rect_size[1],
                            },
                            transform,
                            cull.expect("local light cull data"),
                            native_local_shadow_live::NativeLocalLightSource(light_index),
                        ));
                        spawned += 1;
                    }
                    entities::LightType::Parallel => {
                        let native_parallel_color = if is_sun {
                            render_light::sun_ground_color(scene.environment_lighting
                                .and_then(|environment|environment.sun_color)
                                .unwrap_or([0.215861;3]))
                        } else { preset.color };
                        let sun_color=Color::linear_rgb(native_parallel_color[0],
                            native_parallel_color[1],native_parallel_color[2]);
                        let authored_direction =
                            game_to_view(light.rotation[0]).normalize_or_zero();
                        let direction = if light.preset.as_deref() == Some("LIGHT_PRESET_SUN") {
                            scene
                                .environment_lighting
                                .and_then(environment_sun_ray)
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
                                // Native lighting uses normalized HDR radiance;
                                // PostFX/Luminance/common supplies world units.
                                illuminance: render_light::ARKANE_TO_WORLD_LUMINANCE,
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
        native_material_images.insert(
            cached.handle.clone(),
            cached.images.iter().map(|&(slot, _)| slot).collect(),
        );
        for &(slot, role) in &cached.images {
            diffuse_image_materials[slot].push((cached.handle.clone(), role));
        }
    }
    let diffuse_image_count = resolver.diffuse_image_paths.len();
    commands.insert_resource(RenderSourcePlacements(expanded_placements));
    commands.insert_resource(PlacedModelQueries(placed_model_queries));
    commands.insert_resource(NativeShadowModelBoundsSources(native_shadow_model_bounds));
    commands.insert_resource(native_shadow_live::NativeModelShadowSources(native_model_shadow_sources));
    commands.insert_resource(std::mem::take(&mut resolver.shadow_materials));
    let vegetation_vertices: usize = native_vegetation_meshes
        .0
        .values()
        .filter_map(|result| result.as_ref().ok())
        .sum();
    let vegetation_meshes = native_vegetation_meshes
        .0
        .values()
        .filter(|result| result.is_ok())
        .count();
    println!(
        "Native vegetation inputs: {vegetation_meshes} shared meshes, {vegetation_vertices} vertices, {} decoded attribute bytes; {} unavailable",
        vegetation_vertices * std::mem::size_of::<[f32; 8]>(),
        native_vegetation_meshes.0.len() - vegetation_meshes
    );
    commands.insert_resource(native_vegetation_meshes);
    commands.insert_resource(native_world_shadow_mesh_sources);
    let world_lods_by_render_instance = if trace_bwm_lod {
        render_instances
            .iter()
            .map(|instance| {
                instance.source.and_then(|source| match source.surface {
                    RenderModelSurface::World { object, .. } => scene
                        .world
                        .objects
                        .get(object)
                        .map(|world_object| world_object.lod),
                    RenderModelSurface::Bmodel { .. } | RenderModelSurface::Bmd6 { .. } => None,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    commands.insert_resource(SceneRenderQueue {
        instances: render_instances,
        active: 0,
        static_world_model_present: true,
        tome_render_instances,
        visible_authored_ids: Vec::new(),
        visible_cell_indices: Vec::new(),
        visible_cell_mask: Vec::new(),
        light_query_state: None,
        shadow_ordinary_query: None,
        visibility_suppression_groups,
        trace_bwm_lod,
        world_lods_by_render_instance,
        last_bwm_lod_visibility_report: None,
        diffuse_image_demands: (0..diffuse_image_count)
            .map(|_| texture::ProjectedAreaDemand::default())
            .collect(),
        diffuse_image_paths: std::mem::take(&mut resolver.diffuse_image_paths),
        diffuse_image_materials,
        native_material_images,
        diffuse_image_handles: vec![None; diffuse_image_count],
        native_shadow_diffuse_slots: Vec::new(),
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
        window.title = format!("Dishonored 2 Rust | picked up {quantity} Г— {path}");
    }
    println!("Picked up {id}: {quantity} Г— {path}; inventory total {inventory_total}");
}

fn snapshot_once(
    mut commands: Commands,
    request: Option<Res<SnapshotPath>>,
    time: Res<Time>,
    native_screen:Option<Res<native_screen_shadow_scene::NativeScreenShadowFrame>>,
    pipelines:Option<Res<native_shadow_scene::NativeShadowCaptureReadiness>>,
    mut captured: Local<bool>,
    mut ready_since:Local<Option<f32>>,
) {
    let Some(request) = request else {
        return;
    };
    let shadows_ready=native_screen.as_ref().is_none_or(|screen|
        screen.produced.load(std::sync::atomic::Ordering::Acquire)>=30);
    let materials_ready=pipelines.as_ref().is_none_or(|ready|
        ready.0.load(std::sync::atomic::Ordering::Acquire)>=30);
    if !shadows_ready || !materials_ready {*ready_since=None;return;}
    // Let exposure and texture residency settle after drawable materials exist,
    // rather than counting shader-compilation time as rendered warmup.
    let since=ready_since.get_or_insert(time.elapsed_secs());
    if !*captured && time.elapsed_secs()-*since > 6.0 {
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
    projection: &Projection,
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
    let reference_fov = match reference_fov_from_projection(projection) {
        Ok(fov) => fov,
        Err(error) => return format!("Tome camera matrix unavailable: {error}"),
    };
    let camera_pose = game_pose_from_view_transform(transform);
    let matrix = match tome::original_world_visibility_query_matrix(
        reference_fov,
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
            "Tome matrix from current camera projection: viewport={}x{}, pixel aspect=1, camera_game_xyz={camera_position:?}, rows={matrix:?}\nGlobal spatial-record candidates from geometric prefilter: {nodes:?}. This geometric prefilter is not the final visibility result; the live query's traversal, accepted-object output, mapping and upload diagnostics are in the matching streaming history.",
            viewport.x, viewport.y
        ),
        Err(error) => format!(
            "Tome matrix from current camera projection: viewport={}x{}, camera_game_xyz={camera_position:?}, rows={matrix:?}\nGlobal spatial-record prefilter failed: {error}",
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
        "Tome {label} {point:?}: point-containment prefilter touches {} authored spatial records (not the full visibility result); {} gate-state words; cell-slot rows {} Г— {} bits",
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
    cameras: Query<(&Camera, &Transform, &Projection), With<Camera3d>>,
    diagnostics: Option<Res<StreamingDiagnostics>>,
) {
    if !keys.just_pressed(KeyCode::F8) {
        return;
    }
    let Ok((camera_component, camera, projection)) = cameras.single() else {
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
            tome_camera_matrix_text(tome_world, camera_component, camera, projection),
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
        "Level: {}\nCamera game_xyz: {:?}\nCamera direction game_xyz: {:?}\nCamera up game_xyz: {:?}\nReference horizontal FOV degrees: {}\nFeet probe game_xyz: {:?}\nMode: {}\nWalker supported: {}\nWalker vertical speed: {:.3} m/s\n{}\n{}\n{}\nScreenshot: {}\nNote: In free-fly, feet probe is 1.7 m below the camera; walker state is inactive. BWM ray tests static world geometry; placed visual models are identified in the screenshot. Collision ray tests the combined loaded collision index. Tome object candidates precede the game's final visibility tests.\n",
        scene.source_path.display(),
        view_to_game(camera.translation),
        view_to_game(direction),
        view_to_game(camera.up().as_vec3()),
        reference_fov_from_projection(projection)
            .expect("F8 perspective camera must have a valid FOV"),
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
        let texture_event_history = if diagnostics.trace_texture_streaming {
            diagnostics
                .texture_event_history
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            "disabled (set DH2_TRACE_TEXTURE_STREAMING=1 before launch)".to_owned()
        };
        let history = format!(
            "Level: {}\nSession log: {}\n{}\nRecent texture-stream events (up to 256 retained):\n{}",
            scene.source_path.display(),
            diagnostics.log_path.display(),
            diagnostics
                .history
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n"),
            texture_event_history,
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

#[derive(PartialEq, Eq)]
struct TomeRenderQueryKey {
    camera_point: [u32; 3],
    render_rows: [[u32; 4]; 4],
    cropped_rows: [[u32; 4]; 4],
    initial_tile_rect: [u8; 4],
    camera_imprecision: [u32; 3],
    object_imprecision: u32,
    minimum_area_metric: u32,
    global_distance_threshold: u32,
    object_distance_scale: u32,
    winding_flip: bool,
    gate_states: Option<Vec<u32>>,
    output_capacity: usize,
    job_count: u32,
}

impl TomeRenderQueryKey {
    fn new(input: &tome::OrdinaryDirectQueryInput<'_>, job_count: u32) -> Self {
        // Exhaustive destructuring makes added query inputs a compile error.
        let tome::OrdinaryDirectQueryInput {
            camera_point,
            render_rows,
            cropped_rows,
            initial_tile_rect,
            camera_imprecision,
            object_imprecision,
            minimum_area_metric,
            global_distance_threshold,
            object_distance_scale,
            winding_flip,
            gate_states,
            output_capacity,
        } = *input;
        Self {
            camera_point: camera_point.map(f32::to_bits),
            render_rows: render_rows.map(|row| row.map(f32::to_bits)),
            cropped_rows: cropped_rows.map(|row| row.map(f32::to_bits)),
            initial_tile_rect,
            camera_imprecision: camera_imprecision.map(f32::to_bits),
            object_imprecision: object_imprecision.to_bits(),
            minimum_area_metric: minimum_area_metric.to_bits(),
            global_distance_threshold: global_distance_threshold.to_bits(),
            object_distance_scale: object_distance_scale.to_bits(),
            winding_flip,
            gate_states: gate_states.map(<[u32]>::to_vec),
            output_capacity,
            job_count,
        }
    }
}

#[derive(Default)]
struct TomeRenderQueryCache {
    key: Option<TomeRenderQueryKey>,
    output: Option<tome::RenderQueryObjectOutput>,
    diagnostics: Vec<tome::RenderQueryJobDiagnostics>,
}

impl TomeRenderQueryCache {
    fn matches(&self, key: &TomeRenderQueryKey, world_changed: bool) -> bool {
        !world_changed && self.key.as_ref() == Some(key)
    }

    fn store(
        &mut self,
        key: TomeRenderQueryKey,
        result: &Result<Option<tome::RenderQueryObjectOutput>, String>,
        diagnostics: &[tome::RenderQueryJobDiagnostics],
    ) {
        self.key = None;
        if diagnostics.iter().any(|job| job.error.is_some()) {
            return;
        }
        if let Ok(output) = result {
            self.output = output.clone();
            self.diagnostics = diagnostics.to_vec();
            self.key = Some(key);
        }
    }
}

// The ordinary direct-Tome query selects mapped static BWM instances and
// forwards their projected image demand. The installed worker clears only
// its own output on error, then returns; merge the successful siblings, log
// every failed job, and never retain a failed job's old or partial results.
fn stream_visible_models(
    mut commands: Commands,
    cameras: Query<(Option<&Camera>, Option<&Transform>, Option<&Projection>), With<Camera3d>>,
    mut scene: ResMut<SceneRenderQueue>,
    tome_world: Option<Res<TomeWorld>>,
    placed_model_queries: Option<Res<PlacedModelQueries>>,
    pickups: Res<PickupState>,
    mut timing: Option<ResMut<VisibilityElapsedTiming>>,
    mut diagnostics: Option<ResMut<StreamingDiagnostics>>,
    mut last_tome_error: Local<Option<String>>,
    mut query_cache: Local<TomeRenderQueryCache>,
    view_settings: Option<Res<NativeViewSettings>>,
) {
    let started = Instant::now();
    if tome_world.as_ref().is_none_or(|world| world.is_changed()) {
        *query_cache = TomeRenderQueryCache::default();
    }
    // Every frame replaces these outputs, including frames without a usable
    // camera. Do not expose a previous view's cells or IDs to light selection.
    scene.visible_authored_ids.clear();
    scene.visible_cell_indices.clear();
    scene.visible_cell_mask.clear();
    scene.light_query_state = None;
    scene.shadow_ordinary_query = None;
    // An unavailable view must still replace model visibility below.
    let view = cameras.single().ok();
    let (camera, transform, projection) = view.unwrap_or_default();
    let mut query_ns = 0;
    let mut query_cached = false;
    let mut mapping_ns = 0;
    let mut status = if view.is_none() {
        "camera_unavailable".to_owned()
    } else if tome_world.is_some() {
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
    if let (Some(tome_world), Some(camera), Some(transform), Some(projection)) =
        (tome_world.as_ref(), camera, transform, projection)
    {
        if let Some(viewport) = camera.physical_viewport_size() {
            query_viewport = Some([viewport.x, viewport.y]);
            scene.texture_viewport_pixels = viewport.x.saturating_mul(viewport.y);
            let query_started = Instant::now();
            let query = reference_fov_from_projection(projection)
                .and_then(|reference_fov| {
                    tome::original_world_visibility_query_matrix(
                        reference_fov,
                        viewport.x,
                        viewport.y,
                        1.0,
                        game_pose_from_view_transform(transform),
                        ORIGINAL_DEFAULT_NEAR,
                        ORIGINAL_DEFAULT_FAR,
                    )
                })
                .and_then(|rows| {
                    query_rows = Some(rows);
                    let query_settings=match view_settings.as_deref() {Some(settings)=>settings.0.clone()?,None=>original_settings::view_settings("")?};
                    let input = tome::OrdinaryDirectQueryInput {
                        camera_point: view_to_game(transform.translation),
                        render_rows: rows,
                        cropped_rows: rows,
                        initial_tile_rect: [0, 0, 8, 16],
                        camera_imprecision: [query_settings.umbra_distance_imprecision; 3],
                        global_distance_threshold: query_settings.umbra_accurate_threshold,
                        object_distance_scale: native_view_distance_scale(projection,viewport,view_settings.as_deref())?,
                        object_imprecision: query_settings.umbra_distance_imprecision,
                        minimum_area_metric: 25.0 / (viewport.x as f32 * viewport.y as f32),
                        winding_flip: tome::query_matrix_winding_flip(rows),
                        gate_states: Some(&tome_world.gate_states),
                        output_capacity: tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
                    };
                    let key = TomeRenderQueryKey::new(&input, tome_world.query_job_count);
                    if query_cache.matches(&key, tome_world.is_changed()) {
                        query_cached = true;
                        query_diagnostics = query_cache.diagnostics.clone();
                        for job in &mut query_diagnostics {
                            job.elapsed_ns = 0;
                        }
                        return Ok(query_cache.output.clone());
                    }
                    query_cache.key = None;
                    let jobs = run_tome_render_jobs(tome_world, &input);
                    let result = tome::merge_render_query_outputs(&jobs);
                    job_errors = jobs
                        .iter()
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
                    // Only the immutable Tome query is reused. Mapping, moving
                    // models/pickups, light gathering and texture demand still run.
                    // Failed/partial jobs are always retried, never cached.
                    query_cache.store(key, &result, &query_diagnostics);
                    result
                });
            query_ns = query_started.elapsed().as_nanos();
            let mapping_started = Instant::now();
            match query {
                Ok(Some(output)) => {
                    // Native shadow preparation consumes the merged ordinary
                    // descriptor, not color-visible BWM IDs or a prior frame.
                    // Forward mapping errors do not invalidate this query.
                    scene.publish_shadow_ordinary_query(&output);
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
                            let bwm_lod_before_suppression = scene.trace_bwm_lod.then(|| {
                                bwm_lod_visibility_counts(
                                    &scene.world_lods_by_render_instance,
                                    &visible,
                                )
                            });
                            RenderSuppressionGroup::apply_all(
                                &scene.visibility_suppression_groups,
                                &mut visible,
                            );
                            suppressed =
                                mapped_count - visible.iter().filter(|&&visible| visible).count();
                            let bwm_lod_report = bwm_lod_before_suppression.map(|before| {
                                let after = bwm_lod_visibility_counts(
                                    &scene.world_lods_by_render_instance,
                                    &visible,
                                );
                                (before, after)
                            });
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
                            if let Some(report) = bwm_lod_report {
                                if scene.last_bwm_lod_visibility_report.as_ref() != Some(&report) {
                                    eprintln!(
                                        "BWM LOD visibility changed: before_parent_suppression={:?} after_parent_suppression={:?} requested_tome={} query_cached={} camera={:?}",
                                        report.0,
                                        report.1,
                                        requested,
                                        query_cached,
                                        game_pose_from_view_transform(transform),
                                    );
                                    scene.last_bwm_lod_visibility_report = Some(report);
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
        let depth_grid_bound = scene
            .light_query_state
            .as_ref()
            .is_some_and(|state| state.depth_grid.is_some());
        diagnostics.pending = format!(
            "frame={} status={:?} jobs={} distance_rejected_job_candidates={distance_rejected_job_candidates} authored_ids={} authored_id_consumer=static_light_id_gate visible_cell_entries={visible_cell_entries} visible_cell_unique={visible_cell_unique} cell_mask_words={visible_cell_mask_words} light_binding=authored_ids_and_cells light_consumer=see_light_stage query_cached={query_cached} viewport={query_viewport:?} merged_query_tile={merged_query_tile:?} depth_grid_bound={depth_grid_bound} requested={requested} mapped={mapped_count} spatial_tested={} spatial_accepted_bwm={spatial_accepted} suppressed={suppressed} active={active}/{} spawned={spawned} shown={shown} hidden={hidden} query_ms={:.3} mapping_ms={:.3} elapsed_ms={:.3}\npose={pose:?}\nrender_rows={query_rows:?} winding_flip={:?}\naccepted_sample={accepted_sample:?}\nauthored_id_sample={authored_id_sample:?}\nvisible_cell_sample={visible_cell_sample:?}\n{query_diagnostics:?}",
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
        .map(tome::RenderAabbQuery::from_state)
        .transpose();
    stats.world_query_bound = matches!(&world_query, Ok(Some(_)));
    stats.world_query_depth_grid_bound = stats.world_query_bound
        && scene
            .as_ref()
            .and_then(|scene| scene.light_query_state.as_ref())
            .is_some_and(|state| state.depth_grid.is_some());
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
        "light_source_class[static,dynamic]={:?} static_name_hash_matches={} static_id_culled={} zero_preset_rgb={} rgb_eligibility=unsupported identity_binding={} projection_bounds_unverified={} world_query_bound={} world_query_unverified={} world_query_error={:?} projection_binding={} sphere_path=initial_single_tome sphere_candidates={} sphere_culled={} sphere_unverified={} sphere_query_ms={:.3} sphere_error={:?}\n",
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
        if light_culling.world_query_depth_grid_bound {
            "query_depth_grid"
        } else {
            "no_depth"
        },
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
    mut frame_times: Local<Vec<f64>>,
    render_diagnostics: Option<Res<bevy::diagnostic::DiagnosticsStore>>,
    mut render_metrics: Local<std::collections::BTreeMap<String, (u64, f64, f64)>>,
    cpu_profile: Option<Res<frame_profile::CpuFrameProfile>>,
    mut cpu_metrics: Local<(
        u64,
        [u128; frame_profile::COUNT],
        [u64; frame_profile::COUNT],
    )>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(target) = target else {
        return;
    };
    let previous_elapsed = *elapsed;
    *elapsed += time.delta_secs();
    if previous_elapsed >= target.1 {
        measured.0 += 1;
        measured.1 += time.delta_secs_f64();
        frame_times.push(time.delta_secs_f64() * 1000.0);
        if let Some(profile) = cpu_profile.as_ref() {
            let latest = profile.latest_ns();
            cpu_metrics.0 += 1;
            for (i, ns) in latest.into_iter().enumerate() {
                cpu_metrics.1[i] += ns as u128;
                cpu_metrics.2[i] = cpu_metrics.2[i].max(ns);
            }
        }
        if let Some(store) = render_diagnostics.as_deref() {
            for diagnostic in store.iter() {
                let path = diagnostic.path().as_str();
                if path.starts_with("render/")
                    && (path.ends_with("elapsed_gpu") || path.ends_with("shader_invocations"))
                {
                    if let Some(value) = diagnostic.value().filter(|value| value.is_finite()) {
                        let metric = render_metrics
                            .entry(path.to_owned())
                            .or_insert((0, 0.0, 0.0));
                        metric.0 += 1;
                        metric.1 += value;
                        metric.2 = metric.2.max(value);
                    }
                }
            }
        }
    }
    if *elapsed >= target.0 {
        for (i, label) in frame_profile::LABELS
            .iter()
            .enumerate()
            .filter(|_| cpu_metrics.0 > 0)
        {
            println!(
                "BENCHMARK_CPU phase={} samples={} avg_ms={:.3} max_ms={:.3}",
                label,
                cpu_metrics.0,
                cpu_metrics.1[i] as f64 / cpu_metrics.0 as f64 / 1_000_000.0,
                cpu_metrics.2[i] as f64 / 1_000_000.0
            );
        }
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
        frame_times.sort_unstable_by(f64::total_cmp);
        if !frame_times.is_empty() {
            let quantile =
                |q: f64| frame_times[((frame_times.len() - 1) as f64 * q).round() as usize];
            println!(
                "BENCHMARK frame_ms[p50,p95,p99,max]={:.3},{:.3},{:.3},{:.3} warmup_seconds={:.1}",
                quantile(0.5),
                quantile(0.95),
                quantile(0.99),
                quantile(1.0),
                target.1
            );
        }
        for (path, (samples, total, maximum)) in render_metrics.iter() {
            println!(
                "BENCHMARK_GPU {path} samples={samples} avg={:.3} max={maximum:.3}",
                total / *samples as f64
            );
        }
        exit.write(AppExit::Success);
    }
}

fn show_frame_rate(
    time: Res<Time<bevy::time::Real>>,
    scene: Res<SceneRenderQueue>,
    mut window: Single<&mut Window>,
    mut overlays: Query<&mut Text, With<FrameRateOverlay>>,
    mut elapsed: Local<f32>,
    mut frames: Local<u32>,
) {
    *elapsed += time.delta_secs();
    *frames += 1;
    if *elapsed >= 1.0 {
        let fps = *frames as f32 / *elapsed;
        let frame_ms = *elapsed * 1000.0 / *frames as f32;
        window.title = format!(
            "Dishonored 2 Rust | {:.0} FPS | {} loaded",
            fps, scene.active
        );
        for mut overlay in &mut overlays {
            **overlay = format!("{fps:.0} FPS | {frame_ms:.1} ms");
        }
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
    // The original compiled programs live beside the archives in shaderCache,
    // not necessarily in generated/renderprogs2/*.vbin archive entries.
    let program_cache = match (game_base.as_deref(), catalog.as_ref()) {
        (Some(base), Some(catalog)) => shader_cache::NativeProgramLibrary::open(
            base,
            shader_cache::ShaderCacheBackend::Pc,
            catalog,
        ),
        _ => Err("original game base/catalog missing".to_owned()),
    };
    match &program_cache {
        Ok(library) => println!(
            "Installed native program cache: {} programs, {} stage records, {} shared names, {} parameter declarations (data streams/definitions lazy)",
            library.cache.programs.len(),
            library.cache.stages.len(),
            library.cache.names.len(),
            library.parameter_kinds.len()
        ),
        Err(error) => eprintln!("Native program cache unavailable: {error}"),
    }
    // Resolve the user's ORIGINAL quality cvar and the installed declaration
    // once. No per-frame filesystem/catalog scans and no copied capture values.
    let original_shadow_settings = original_settings::read_shadow_settings();
    let shadow_depth_bias_clamp = original_shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.depth_bias_clamp);
    let shadow_sort_ccw = original_shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.sort_shadows_ccw);
    let shadow_cascade_controls = original_shadow_settings
        .as_ref()
        .ok()
        .map(|(_, settings)| settings.cascade_controls);
    let shadow_cascade_fars = original_shadow_settings
        .as_ref()
        .ok()
        .and_then(|(_, settings)| settings.cascade_fars);
    let (shadow_quality_preset, shadow_quality, shadow_target_dimensions) = match (
        original_shadow_settings,
        catalog.as_ref(),
    ) {
        (
            Ok((
                config_path,
                original_settings::ShadowSettings {
                    quality_preset: Some(preset),
                    parallel_slice_size,
                    depth_bias_clamp,
                    ..
                },
            )),
            Some(catalog),
        ) => {
            let settings =
                load_installed_shadow_quality(catalog, preset).map_err(std::io::Error::other)?;
            let target = settings
                .as_ref()
                .map(|quality| native_shadow_target_dimensions(parallel_slice_size, quality))
                .transpose()
                .map_err(std::io::Error::other)?;
            println!(
                "Native shadow quality from {}: preset={preset}, slice_size={parallel_slice_size}, depth_bias_clamp={depth_bias_clamp}, settings={settings:?}, target={target:?}",
                config_path.display()
            );
            (Some(preset), settings, target)
        }
        (Err(error), _) => {
            eprintln!("Native shadow quality unavailable: {error}");
            (None, None, None)
        }
        _ => {
            eprintln!(
                "Native shadow quality unavailable: original config setting or asset catalog missing"
            );
            (None, None, None)
        }
    };
    let far_shadow_bounds = entity_document
        .as_ref()
        .map(entities::world_far_shadow_bounds)
        .transpose()
        .map_err(std::io::Error::other)?
        .flatten();
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
    let world_environment = entity_document
        .as_ref()
        .and_then(entities::world_environment);
    let environment_data = match (world_environment.as_deref(), catalog.as_ref()) {
        (Some(environment), Some(catalog)) => {
            let resource = format!("generated/decls/env/{environment}.decl");
            catalog
                .read(&resource)?
                .map(|bytes| {
                    let source = String::from_utf8_lossy(&bytes).into_owned();
                    let lighting =
                        entities::environment_lighting_with_inheritance(&source, |parent| {
                            let parent_resource = format!("generated/decls/env/{parent}.decl");
                            catalog
                                .read(&parent_resource)
                                .map(|bytes| {
                                    bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                                })
                                .map_err(|error| error.to_string())
                        })
                        .map_err(std::io::Error::other)?;
                    let render_parameters =
                        entities::environment_render_parameters_with_inheritance(
                            environment,
                            &source,
                            |parent| {
                                let parent_resource = format!("generated/decls/env/{parent}.decl");
                                catalog
                                    .read(&parent_resource)
                                    .map(|bytes| {
                                        bytes.map(|bytes| {
                                            String::from_utf8_lossy(&bytes).into_owned()
                                        })
                                    })
                                    .map_err(|error| error.to_string())
                            },
                        )
                        .map_err(std::io::Error::other)?;
                    Ok::<_, std::io::Error>((lighting, render_parameters))
                })
                .transpose()?
        }
        _ => None,
    };
    let (environment_lighting, environment_render_parameters) = environment_data
        .map(|(lighting, parameters)| (Some(lighting), Some(parameters)))
        .unwrap_or((None, None));
    let probe_start = Instant::now();
    let mut environment_probes = match (catalog.as_ref(), path.file_stem().and_then(|s| s.to_str())) {
        (Some(catalog), Some(stem)) => {
            probe_lighting::load_for_world(catalog, stem).map_err(std::io::Error::other)?
        }
        _ => None,
    };
    if let (Some(probes),Some(environment))=(&mut environment_probes,environment_lighting) {
        let parameters=probe_lighting::ProbeLightingParameters {
            static_factor:environment.bounce_static.unwrap_or([1.0;3]),
            sun_factor:environment.bounce_sun.unwrap_or([1.0;3]),
            sun_ground_color:render_light::sun_ground_color(environment.sun_color.unwrap_or([0.215861;3])),
            sky_zenith:environment.bounce_sky_zenith.unwrap_or([3.88954449,8.88057613,20.3924446]),
            sky_horizon:environment.bounce_sky_horizon.unwrap_or([33.8765373,44.5004768,50.7929688]),
            ambient:environment.indirect_ambient.unwrap_or([0.0;3]),
            // Original r_ILProbeUpdate_ContributionFactorMultiBounces default3,
            // r_ILProbeUpdate_MultiBouncesDebugFlags default7 (all sources).
            multi_bounce:3.0,multi_bounce_flags:7,
        };
        probes.initialize_indirect_lighting(&parameters);
        println!("Native probe accumulation: {} probes, source-backed static/sun/occluded sky/per-probe ambient; sun_ground={:?}",probes.probes.len(),parameters.sun_ground_color);
    }
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
    if let Some(parameters) = &environment_render_parameters {
        let value_count = parameters
            .layers
            .iter()
            .map(|layer| layer.parameters.len())
            .sum::<usize>();
        println!(
            "World environment render parameters retained: {} layers, {value_count} raw entries (not yet consumed by fog/water/sky/postfx rendering)",
            parameters.layers.len(),
        );
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
            "Loaded original Tome from {}: {} spatial nodes, {} initially enabled gates; cell-slot rows {} Г— {} bits",
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
        match tome::shadow_caster::ShadowCasterTome::parse(&tome_bytes) {
            Ok(shadow_tome) => {
                println!(
                    "Native shadow Tome cache: {} original caster objects",
                    shadow_tome.object_count()
                );
                app.insert_resource(NativeShadowCasterWorld(shadow_tome));
            }
            Err(error) => eprintln!("Native shadow Tome cache unavailable: {error}"),
        }
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
            let bodies = entity_document
                .as_ref()
                .map(entities::rigid_body_settings)
                .unwrap_or_default();
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
        let total = seconds.parse::<f32>()?.max(3.0);
        let warmup = env::var("DH2_BENCHMARK_WARMUP_SECONDS")
            .unwrap_or_else(|_| "2".to_owned())
            .parse::<f32>()?;
        if !total.is_finite() || !warmup.is_finite() || warmup < 0.0 || warmup >= total {
            return Err(
                "benchmark duration must be finite and exceed its non-negative warmup".into(),
            );
        }
        app.insert_resource(BenchmarkSeconds(total, warmup));
    }
    app.insert_resource(NativeProgramCacheSource(program_cache))
        .insert_resource(ClearColor(Color::srgb(0.055, 0.045, 0.06)))
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
            environment_render_parameters,
            environment_probes,
            shadow_quality_preset,
            shadow_quality,
            shadow_target_dimensions,
            shadow_depth_bias_clamp,
            shadow_sort_ccw,
            shadow_cascade_controls,
            shadow_cascade_fars,
            far_shadow_bounds,
            pickups,
            catalog,
        })
        .add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: Some(bevy::window::Window {
                        // Match the installed game's measured r_fullscreen=2 borderless mode.
                        // DH2_BORDERLESS=0 remains available for windowed debugging.
                        mode: if env::var("DH2_BORDERLESS").as_deref() != Ok("0") {
                            bevy::window::WindowMode::BorderlessFullscreen(
                                bevy::window::MonitorSelection::Primary,
                            )
                        } else {
                            bevy::window::WindowMode::Windowed
                        },
                        desired_maximum_frame_latency: env::var("DH2_FRAME_LATENCY")
                            .ok()
                            .and_then(|s| s.parse().ok()),
                        // The user's original r_swapInterval=0; retain that uncapped default.
                        present_mode: if env::var("DH2_NO_VSYNC").as_deref() != Ok("0") {
                            bevy::window::PresentMode::AutoNoVsync
                        } else {
                            bevy::window::Window::default().present_mode
                        },
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::render::RenderPlugin {
                    render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                        bevy::render::settings::WgpuSettings {
                            // Original shadow world draws disable depth clipping.
                            // The captured `decompressedDepth` input uses an
                            // R24G8 typeless depth-stencil backing (not a DXGI
                            // format number passed through unchanged).
                            features: bevy::render::settings::WgpuSettings::default().features
                                | bevy::render::settings::WgpuFeatures::DEPTH_CLIP_CONTROL
                                | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
                            ..default()
                        },
                    )),
                    ..default()
                }),
        );
    if env::var("DH2_RENDER_PROFILE").as_deref() == Ok("1") {
        println!(
            "Bevy render profiling enabled (GPU timestamps and pipeline stats require Vulkan or DX12)"
        );
        app.add_plugins((
            bevy::diagnostic::FrameTimeDiagnosticsPlugin::default(),
            bevy::render::diagnostic::RenderDiagnosticsPlugin,
        ));
        frame_profile::install(&mut app);
    }
    app.add_plugins(AutoExposurePlugin)
        .add_plugins(game_clock_runtime::NativeClockRuntimePlugin)
        .add_plugins(native_shadow_scene::NativeShadowScenePlugin)
        .add_plugins(native_screen_shadow_scene::NativeScreenShadowScenePlugin)
        .add_plugins(native_coarse_depth::NativeCoarseDepthPlugin)
        .init_resource::<native_shadow_live::NativeParallelShadowCpu>()
        .init_resource::<native_local_shadow_live::NativeLocalShadowCpu>()
        .init_resource::<native_local_shadow_live::NativeLocalShadowViewControls>()
        .init_resource::<native_local_shadow_live::NativeLocalShadowDraws>()
        .init_resource::<native_shadow_live::NativeParallelWorldDraws>()
        .init_resource::<native_shadow_live::NativeModelCasterFrame>()
        .init_resource::<native_shadow_live::NativeParallelModelDraws>()
        .insert_resource(NativeViewSettings(original_settings::read_view_settings().map(|(_,settings)|settings)))
        .add_plugins(MaterialPlugin::<ProbeMaterial>::default())
        .add_plugins(MaterialPlugin::<DecalMaterial>::default())
        .insert_resource(VisibilityElapsedTiming::default())
        .insert_resource(OriginalLocalLightCullEnabled(original_light_area_cull))
        .insert_resource(OriginalLocalLightCullStats::default())
        .insert_resource(StreamingDiagnostics::new(&path))
        .add_systems(
            Startup,
            (native_shadow_live::initialize_native_screen_shadow_source,
                setup, initialize_native_parallel_shadow_owner,
                native_shadow_live::initialize_native_parallel_receiver_source,
                native_local_shadow_live::initialize_native_local_shadow_owner).chain(),
        )
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
            publish_initial_native_whole_view_depth
                .after(game_clock_runtime::NativeClockSystems::PublishView)
                .before(bevy::asset::AssetEventSystems),
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
            native_shadow_live::prepare_native_parallel_shadow_cpu
                .after(stream_visible_models)
                .after(native_shadow_live::collect_native_model_casters)
                .after(game_clock_runtime::NativeClockSystems::PublishView),
        )
        .add_systems(
            PostUpdate,
            native_shadow_live::collect_native_model_casters.after(stream_visible_models),
        )
        .add_systems(
            PostUpdate,
            native_shadow_live::prepare_native_parallel_world_draws
                .after(native_shadow_live::prepare_native_parallel_shadow_cpu)
                .before(bevy::asset::AssetEventSystems),
        )
        .add_systems(
            PostUpdate,
            native_shadow_live::prepare_native_parallel_model_draws
                .after(native_shadow_live::prepare_native_parallel_shadow_cpu),
        )
        .add_systems(
            PostUpdate,
            native_shadow_live::publish_native_parallel_shadow_frame
                .after(native_shadow_live::prepare_native_parallel_world_draws)
                .after(native_shadow_live::prepare_native_parallel_model_draws)
                .after(publish_initial_native_whole_view_depth),
        )
        .add_systems(PostUpdate,native_shadow_live::prepare_native_screen_shadow_frame
            .after(native_shadow_live::publish_native_parallel_shadow_frame)
            .before(bevy::asset::AssetEventSystems))
        .add_systems(PostUpdate,native_local_shadow_live::prepare_native_local_shadow_cpu
            .after(cull_original_small_local_lights)
            .after(native_shadow_live::collect_native_model_casters)
            .before(bevy::asset::AssetEventSystems))
        .add_systems(PostUpdate,native_local_shadow_live::publish_native_local_shadow_draws
            .after(native_local_shadow_live::prepare_native_local_shadow_cpu)
            .before(bevy::asset::AssetEventSystems))
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
    bevy::asset::load_internal_asset!(
        app,
        PROBE_PREPASS_SHADER_HANDLE,
        "../../assets/shaders/dh2_probe_prepass.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        INDIRECT_SHADER_HANDLE,
        "../../assets/shaders/dh2_indirect.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        SURFACE_SHADER_HANDLE,
        "../../assets/shaders/dh2_surface.wgsl",
        Shader::from_wgsl
    );
    bevy::asset::load_internal_asset!(
        app,
        DECAL_SHADER_HANDLE,
        "../../assets/shaders/dh2_decal.wgsl",
        Shader::from_wgsl
    );
    native_shadow_sampling::register_shaders(&mut app);
    app.add_systems(
        PostUpdate,
        (sync_native_shadow_target::<ProbeExtension>,sync_native_shadow_target::<DecalExtension>).after(bevy::asset::AssetEventSystems)
            .after(native_shadow_live::publish_native_parallel_shadow_frame),
    );
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

/// Original numeric quality -> the installed declaration's selected row.
/// Never substitutes an extracted capture or an arbitrary quality profile.
fn load_installed_shadow_quality(
    catalog: &archive::Catalog,
    preset: u32,
) -> Result<Option<entities::ShadowQualitySettings>, String> {
    let candidates = catalog.paths_containing("menusectionoptionvisuals");
    if candidates.len() != 1 {
        return Err(format!(
            "expected one installed visuals declaration, found {}",
            candidates.len()
        ));
    }
    let resource = &candidates[0];
    let bytes = catalog
        .read(resource)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("indexed visuals declaration vanished: {resource}"))?;
    let source = std::str::from_utf8(&bytes).map_err(|e| e.to_string())?;
    entities::shadow_quality_settings_for_preset(source, preset)
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
    #[ignore = "requires installed original catalog and Saved Games config"]
    fn actual_shadow_quality_inputs_resolve_from_original_config_and_assets() {
        let (_, settings) = original_settings::read_shadow_settings().unwrap();
        let preset = settings.quality_preset.unwrap();
        let catalog = archive::Catalog::open(Path::new(
            r"D:\Programs\Steam\steamapps\common\Dishonored2\base",
        ))
        .unwrap();
        let quality = load_installed_shadow_quality(&catalog, preset)
            .unwrap()
            .unwrap();
        let target =
            native_shadow_target_dimensions(settings.parallel_slice_size, &quality).unwrap();
        assert!(quality.cull_distance.is_finite());
        assert_eq!(target.layers, quality.parallel_slice_count + 1);
        assert_eq!(
            target.width,
            settings.parallel_slice_size / (1 << quality.parallel_resolution_divider)
        );
        assert_eq!(target.width, target.height);
        assert_eq!(target.depth_32_bits, quality.depth_32_bits);
        println!("original shadow preset {preset}: {quality:?}; target={target:?}");
    }

    #[test]
    fn native_shadow_model_bounds_keep_hidden_owners_once_not_surface_boxes() {
        let instance = |surface| RenderInstance {
            source: Some(RenderInstanceSource {
                surface,
                world_baked: false,
            }),
            mesh: Handle::default(),
            material: MaterialHandle::Probe(Handle::default()),
            diffuse_image_slot: None,
            probe_index: 0,
            transform: Transform::default(),
            bounds: Aabb::from_min_max(Vec3::splat(1000.0), Vec3::splat(1001.0)),
            pickup_id: None,
            entity: None,
            visible: false,
        };
        let mut scene = SceneRenderQueue::default();
        scene.instances = vec![
            instance(RenderModelSurface::Bmodel {
                placement: 11,
                surface: 0,
            }),
            instance(RenderModelSurface::Bmodel {
                placement: 11,
                surface: 1,
            }),
            instance(RenderModelSurface::Bmd6 {
                placement: 12,
                surface: 0,
                cpu_vertex_declaration: 0,
                gpu_vertex_declaration: 0,
                skin_influences: 1,
                matrix_offset: 0,
            }),
        ];
        let first = [[-1.0, -2.0, -3.0], [4.0, 5.0, 6.0]];
        let second = [[-7.0, -8.0, -9.0], [10.0, 11.0, 12.0]];
        let mut sources = NativeShadowModelBoundsSources(vec![
            NativeShadowModelBoundsSource {
                placement: 11,
                instances: 0..2,
                bounds: first,
            },
            NativeShadowModelBoundsSource {
                placement: 12,
                instances: 2..3,
                bounds: second,
            },
        ]);
        let pickups = PickupState {
            items: Vec::new(),
            world: runtime::WorldState::from_pickups(&[]).unwrap(),
        };
        assert_eq!(
            sources.active_bounds(&scene, &pickups).unwrap(),
            [first, second],
            "hidden/unspawned model owners remain receiver inputs, once per placement"
        );
        for item in &mut scene.instances[..2] {
            item.pickup_id = Some("removed-owner".into());
        }
        assert_eq!(sources.active_bounds(&scene, &pickups).unwrap(), [second]);
        sources.0[1].placement = 99;
        assert!(
            sources.active_bounds(&scene, &pickups).is_err(),
            "no owner substitution"
        );
        sources.0[1].placement = 12;
        sources.0[1].instances.end = 4;
        assert!(
            sources.active_bounds(&scene, &pickups).is_err(),
            "no partial range fallback"
        );
    }

    #[test]
    fn native_shadow_ordinary_query_retains_unmodified_current_depth() {
        let mut output = tome::RenderQueryObjectOutput::default();
        output.state = Some(tome::RenderQueryState {
            render_rows: Mat4::IDENTITY.to_cols_array_2d(),
            cropped_rows: Mat4::IDENTITY.to_cols_array_2d(),
            camera_point: [1.0, 2.0, 3.0],
            camera_imprecision: [0.6; 3],
            winding_flip: false,
            viewport: [0, 0, 64, 64],
            maximum_depth: 0.75,
            depth_grid: Some(Box::new([0.5; 4096])),
        });
        let mut scene = SceneRenderQueue::default();
        scene.publish_shadow_ordinary_query(&output);
        // The existing no-depth diagnostic is exclusively a light consumer.
        scene.light_query_state = output.state.clone();
        scene.light_query_state.as_mut().unwrap().depth_grid = None;
        output.state.as_mut().unwrap().depth_grid.as_mut().unwrap()[0] = 0.25;
        let state = scene
            .shadow_ordinary_query
            .as_ref()
            .unwrap()
            .state
            .as_ref()
            .unwrap();
        assert_eq!(state.depth_grid.as_ref().unwrap()[0], 0.5);
        assert_eq!(state.maximum_depth, 0.75);
        assert_eq!(state.camera_point, [1.0, 2.0, 3.0]);
        // A fresh, empty ordinary descriptor replaces the previous state too.
        scene.publish_shadow_ordinary_query(&tome::RenderQueryObjectOutput::default());
        assert!(
            scene
                .shadow_ordinary_query
                .as_ref()
                .unwrap()
                .state
                .is_none()
        );
    }

    #[test]
    fn native_shadow_ordinary_query_clears_when_camera_is_unavailable() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let mut scene = SceneRenderQueue::default();
        scene.publish_shadow_ordinary_query(&tome::RenderQueryObjectOutput::default());
        world.insert_resource(scene);
        world.insert_resource(PickupState {
            items: Vec::new(),
            world: runtime::WorldState::from_pickups(&[]).unwrap(),
        });
        world.run_system_once(stream_visible_models).unwrap();
        assert!(
            world
                .resource::<SceneRenderQueue>()
                .shadow_ordinary_query
                .is_none(),
            "a missing view must not retain the previous frame's query"
        );
    }

    #[test]
    fn native_world_object_retains_its_actual_material_owner() {
        let mut sources = NativeWorldShadowMeshSources {
            objects: vec![NativeWorldShadowMeshSource {
                object_id: 0,
                mesh_index: 0,
                matrix_start: 0,
                matrix_end: 2,
                lod: 0,
                flipped: false,
                mesh: Handle::default(),
                raw_vertex_count: Ok(3),
                material: None,
                material_error: None,
                gather_flags48: None,
            }],
            shared_mesh_count: 1,
        };
        let material = MaterialHandle::Probe(Handle::default());
        assert!(sources.object(0).unwrap().material.is_none());
        sources.bind_material(0, &material).unwrap();
        sources.bind_material(0, &material).unwrap();
        assert!(sources.object(0).unwrap().material.as_ref() == Some(&material));
        assert!(
            sources
                .bind_material(0, &MaterialHandle::Decal(Handle::default()))
                .is_err()
        );
        assert!(
            sources.object(0).unwrap().material.as_ref() == Some(&material),
            "a conflicting instance must not replace the object's material"
        );
        assert!(sources.bind_material(1, &material).is_err());
    }

    #[cfg(test)]
    fn native_world_shadow_test_world() -> world_mesh::WorldMesh {
        let matrix = |translation: f32| {
            [
                1.0,
                0.0,
                0.0,
                translation,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
            ]
        };
        world_mesh::WorldMesh {
            meshes: vec![world_mesh::Mesh {
                positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
                packed_channels: vec![[0; 16]; 3],
                indices: vec![0, 1, 2],
            }],
            matrices: vec![
                matrix(0.0),
                matrix(1.0),
                matrix(2.0),
                matrix(3.0),
                matrix(4.0),
            ],
            visibility: (0..5)
                .map(|index| world_mesh::WorldVisibility {
                    bounds: [[-1.0; 3], [1.0; 3]],
                    umbra_id: index,
                    trailing_field: 0x1200 + index as u16,
                })
                .collect(),
            auxiliary_instance_groups: std::array::from_fn(|_| Vec::new()),
            visibility_groups: Vec::new(),
            tome_object_instances: Vec::new(),
            spatial_query_instances: Vec::new(),
            objects: vec![
                world_mesh::WorldObject {
                    mesh_index: 0,
                    material_path: "world/native-shadow-test.material".into(),
                    matrix_start: 0,
                    matrix_end: 2,
                    flipped: false,
                    lod: 3,
                },
                world_mesh::WorldObject {
                    mesh_index: 0,
                    material_path: "world/native-shadow-test.material".into(),
                    matrix_start: 2,
                    matrix_end: 4,
                    flipped: true,
                    lod: 2,
                },
                world_mesh::WorldObject {
                    mesh_index: 0,
                    material_path: "world/native-shadow-test.material".into(),
                    matrix_start: 4,
                    matrix_end: 5,
                    flipped: false,
                    lod: 1,
                },
            ],
        }
    }

    #[cfg(test)]
    fn native_world_shadow_test_owner(target_image: Handle<Image>) -> NativeParallelShadowOwner {
        let identity = Mat4::IDENTITY.to_cols_array_2d();
        NativeParallelShadowOwner {
            target_image,
            dimensions: native_shadow_scene::NativeParallelShadowTargetDimensions {
                width: 64,
                height: 64,
                layers: 5,
                depth_32_bits: true,
            },
            whole_view: render_shadow::DirectionalCascade {
                clip: identity,
                atlas: identity,
                inverse_atlas: identity,
                caster_clip: identity,
                camera: [0.0; 3],
                depth_offset: 0.0,
                near_far: [0.0, 1.0],
            },
            whole_view_initial_masks: vec![vec![1, 1, 1, 0, 0], vec![0, 0, 0, 0, 1]],
            hardware_bias: [1.25, 2.5, 0.25],
            shader_bias: [0.0; 3],
            sort_shadows_ccw: true,
            source_flag_04: game_clock::RESET_VIEW_LOCAL_TIME_FLAG,
            cascade_controls: original_settings::ShadowCascadeControls::default(),
            cascade_fars: [4.0, 12.0, 30.0, 75.0],
        }
    }

    #[cfg(test)]
    fn native_world_shadow_test_material(
        class: shader_cache::CachedWorldShadowClass,
    ) -> (MaterialHandle, NativeShadowMaterials) {
        use render_program::{
            D3dVariantData, D3dVariantHeader, RenderParameterKind, SerializedParameter,
            SerializedParameterBlock, SerializedParameterValue, VariantBindings,
        };
        let material = MaterialHandle::Probe(Handle::default());
        let mut main_state = [0; 9];
        main_state[0] = 0x1234_5678;
        main_state[5] = 1 << 27;
        let mut defaults = HashMap::new();
        let vector = |value: [f32; 4]| value.map(f32::to_bits);
        defaults.insert(b"layer0_scalebias".to_vec(), vector([1.0, 1.0, 0.0, 0.0]));
        for (name, value) in [
            (b"vegetanim/mainbendingvar".as_slice(), [1.0, 2.0, 3.0, 4.0]),
            (
                b"vegetanim/mainbendingfreq".as_slice(),
                [2.0, 3.0, 4.0, 5.0],
            ),
            (
                b"vegetanim/mainbendingamplitude".as_slice(),
                [3.0, 4.0, 5.0, 6.0],
            ),
            (
                b"vegetanim/leafminwindstrength".as_slice(),
                [0.1, 0.2, 0.3, 0.4],
            ),
            (b"vegetanim/leaffreq".as_slice(), [4.0, 5.0, 6.0, 7.0]),
            (b"vegetanim/leafamplitude".as_slice(), [5.0, 6.0, 7.0, 8.0]),
            (
                b"vegetanim/branchamplitude".as_slice(),
                [6.0, 7.0, 8.0, 9.0],
            ),
        ] {
            defaults.insert(name.to_vec(), vector(value));
        }
        let uv_override = [2.0, 3.0, 0.25, 0.5].map(f32::to_bits);
        let world_parameters = NativeShadowMaterialParameters {
            overrides: SerializedParameterBlock {
                flags: 0,
                parameters: vec![SerializedParameter {
                    name: b"layer0_scalebias".to_vec(),
                    kind: RenderParameterKind::Uniform,
                    value: SerializedParameterValue::Words(uv_override),
                }],
            },
            defaults,
        };
        let variant = D3dVariantData {
            header: D3dVariantHeader {
                source_crc: 0,
                flags: 0,
                shared_names: false,
                include_count: 0,
            },
            includes: Vec::new(),
            parameters: Vec::new(),
            stages: Vec::new(),
            bindings: VariantBindings {
                shared_names: None,
                parameters: Vec::new(),
            },
            layout: [0; 9],
        };
        let world_source = shader_cache::CachedWorldShadowSource {
            class,
            program: b"typed test source; CPU preparation does not compile it".to_vec(),
            variant_hash: 0,
            definition_render_state: main_state,
            variant,
        };
        let source = material::ShadowSourceInputs {
            source: material::ShadowProgramSource::MainProgram,
            cloth_source: material::ShadowProgramSource::Disabled,
            enable_refraction: false,
            translucency_hq: false,
            material_effects: false,
            override_program: None,
            cloth_override_program: None,
            main_blend_state: 0,
            main_render_state: Some(main_state),
            hair_type: false,
            sky_type: 0,
            is_cloth: false,
            use_skinning: false,
            use_skinning8: false,
            needs_wind: class == shader_cache::CachedWorldShadowClass::Vegetation,
        };
        let inputs = NativeShadowMaterialInputs {
            source,
            rigid_base_source: Ok(None),
            base_slot10_present: Ok(true),
            world_source: Ok(Some(std::sync::Arc::new(world_source))),
            world_parameters: Ok(Some(world_parameters)),
            main_program: Some("native-shadow-test".into()),
            alpha_test: class != shader_cache::CachedWorldShadowClass::Opaque,
            world_uv_set: Some(1),
            diffuse_image_slot: (class != shader_cache::CachedWorldShadowClass::Opaque)
                .then_some(0),
            parameters: Ok(None),
            cull_mode: None,
        };
        (
            material.clone(),
            NativeShadowMaterials(HashMap::from([(material, Ok(inputs))])),
        )
    }

    #[cfg(test)]
    fn native_world_shadow_test_scene(world: world_mesh::WorldMesh) -> SceneData {
        SceneData {
            source_path: PathBuf::from("native-shadow-test.bwm"),
            world,
            player_start: None,
            placed_models: Vec::new(),
            placed_lights: Vec::new(),
            environment_lighting: None,
            environment_render_parameters: None,
            environment_probes: None,
            shadow_quality_preset: None,
            shadow_quality: None,
            shadow_target_dimensions: None,
            shadow_depth_bias_clamp: None,
            shadow_sort_ccw: None,
            shadow_cascade_controls: None,
            shadow_cascade_fars: None,
            far_shadow_bounds: None,
            pickups: Vec::new(),
            catalog: None,
        }
    }

    #[test]
    fn native_world_shadow_initial_publish_retries_alpha_and_owns_one_mask_batch() {
        use bevy::ecs::system::RunSystemOnce;
        let (material, materials) =
            native_world_shadow_test_material(shader_cache::CachedWorldShadowClass::AlphaTested);
        let scene = native_world_shadow_test_scene(native_world_shadow_test_world());
        let mut mesh_assets = Assets::<Mesh>::default();
        let mut sources = NativeWorldShadowMeshSources::prepare(&scene.world, &mut mesh_assets);
        sources.bind_material(0, &material).unwrap();
        sources.bind_material(1, &material).unwrap();
        sources.bind_material(2, &material).unwrap();
        let mut image_assets = Assets::<Image>::default();
        let target_image = image_assets.add(Image::default());
        let owner = native_world_shadow_test_owner(target_image.clone());
        assert_eq!(owner.source_flag_04, game_clock::RESET_VIEW_LOCAL_TIME_FLAG);
        let mut queue = SceneRenderQueue::default();
        queue.diffuse_image_paths = vec!["textures/off-camera-world-alpha.dds".to_string()];
        queue.diffuse_image_handles = vec![None];
        queue.instances.push(RenderInstance {
            source: Some(RenderInstanceSource {
                surface: RenderModelSurface::Bmodel {
                    placement: 77,
                    surface: 0,
                },
                world_baked: false,
            }),
            mesh: sources.object(0).unwrap().mesh.clone(),
            material: material.clone(),
            diffuse_image_slot: Some(0),
            probe_index: 0,
            transform: Transform::default(),
            bounds: Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
            pickup_id: None,
            entity: None,
            visible: false,
        });
        let mut world = World::new();
        world.insert_resource(scene);
        world.insert_resource(owner);
        world.insert_resource(sources);
        world.insert_resource(materials);
        world.insert_resource(queue);
        world.insert_resource(mesh_assets);
        world.insert_resource(image_assets);
        world.insert_resource(Assets::<ShaderBuffer>::default());
        world.insert_resource(native_shadow_scene::NativeShadowSceneFrame::default());

        world
            .run_system_once(publish_initial_native_whole_view_depth)
            .unwrap();
        {
            let frame = world.resource::<native_shadow_scene::NativeShadowSceneFrame>();
            assert_eq!(frame.revision, 0);
            assert!(matches!(
                &frame.state,
                native_shadow_scene::NativeShadowSceneState::Unavailable(error)
                    if error.contains("pending through texture streamer")
            ));
            assert_eq!(
                world.resource::<Assets<ShaderBuffer>>().len(),
                0,
                "dependency retries must not leave provisional GPU-input assets"
            );
        }
        assert_eq!(
            world
                .resource::<SceneRenderQueue>()
                .native_shadow_diffuse_slots,
            [0],
            "off-camera alpha slots are requested by the existing texture streamer"
        );

        let diffuse = world.resource_mut::<Assets<Image>>().add(Image::default());
        {
            let mut queue = world.remove_resource::<SceneRenderQueue>().unwrap();
            queue.diffuse_image_handles[0] = Some(diffuse);
            let scene = world.resource::<SceneData>();
            let owner = world.resource::<NativeParallelShadowOwner>();
            let sources = world.resource::<NativeWorldShadowMeshSources>();
            let materials = world.resource::<NativeShadowMaterials>();
            let meshes = world.resource::<Assets<Mesh>>();
            let images = world.resource::<Assets<Image>>();
            let hidden = prepare_native_whole_view_masks(
                owner,
                &scene.world,
                sources,
                materials,
                meshes,
                images,
                &mut queue,
                None,
                Some("clock is not needed by this alpha shader"),
            )
            .unwrap();
            assert_eq!(
                hidden
                    .iter()
                    .map(|mask| mask.instance_ids.clone())
                    .collect::<Vec<_>>(),
                [vec![0, 1, 2], vec![4]],
                "initial BWM masks, not color-visible IDs, own the remap"
            );
            assert_eq!(
                hidden
                    .iter()
                    .map(|mask| mask.instances.len())
                    .collect::<Vec<_>>(),
                [2, 1]
            );
            queue.instances[0].visible = true;
            let visible = prepare_native_whole_view_masks(
                owner,
                &scene.world,
                sources,
                materials,
                meshes,
                images,
                &mut queue,
                None,
                Some("clock is not needed by this alpha shader"),
            )
            .unwrap();
            assert_eq!(
                visible
                    .iter()
                    .map(|mask| mask.instance_ids.clone())
                    .collect::<Vec<_>>(),
                [vec![0, 1, 2], vec![4]],
                "changing color visibility cannot change native whole-view IDs"
            );
            world.insert_resource(queue);
        }
        assert_eq!(
            world.resource::<Assets<ShaderBuffer>>().len(),
            0,
            "CPU validation alone must not mutate the ShaderBuffer asset table"
        );
        world
            .run_system_once(publish_initial_native_whole_view_depth)
            .unwrap();
        {
            let frame = world.resource::<native_shadow_scene::NativeShadowSceneFrame>();
            assert_eq!(frame.revision, 1);
            let native_shadow_scene::NativeShadowSceneState::WholeViewDepth {
                target_image: published_target,
                dimensions,
                layer,
            } = &frame.state
            else {
                panic!(
                    "initial native stage did not publish WholeViewDepth: {:?}",
                    frame.state
                );
            };
            assert_eq!(published_target.id(), target_image.id());
            assert_eq!(dimensions.layers, 5);
            assert_eq!(layer.instances.len(), 3);
            assert!(matches!(
                layer.cache,
                native_shadow_scene::NativeShadowCacheOp::None
            ));
            assert!(matches!(
                layer.ops.depth,
                native_shadow_scene::NativeShadowLoad::Clear(1.0)
            ));
            assert_eq!(
                layer.ops.stencil,
                native_shadow_scene::NativeStencilLoad::Clear(0)
            );
            let expected_states = [
                render_state::world_shadow_draw_state(
                    world
                        .resource::<NativeShadowMaterials>()
                        .0
                        .get(&material)
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .source
                        .main_render_state
                        .unwrap(),
                    1,
                    1.25,
                    2.5,
                    0.25,
                    0,
                ),
                render_state::world_shadow_draw_state(
                    world
                        .resource::<NativeShadowMaterials>()
                        .0
                        .get(&material)
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .source
                        .main_render_state
                        .unwrap(),
                    0,
                    1.25,
                    2.5,
                    0.25,
                    0,
                ),
                render_state::world_shadow_draw_state(
                    world
                        .resource::<NativeShadowMaterials>()
                        .0
                        .get(&material)
                        .unwrap()
                        .as_ref()
                        .unwrap()
                        .source
                        .main_render_state
                        .unwrap(),
                    0,
                    1.25,
                    2.5,
                    0.25,
                    0,
                ),
            ];
            let mut visible_buffers = Vec::new();
            let mut transform_buffers = Vec::new();
            for (index, instance) in layer.instances.iter().enumerate() {
                assert_eq!(
                    instance.program,
                    native_shadow_scene::NativeShadowProgram::WorldAlphaTested
                );
                assert_eq!(instance.draw_state, expected_states[index]);
                assert_eq!(
                    (instance.front_face, instance.cull_mode),
                    native_shadow_gpu::native_shadow_mesh_raster(
                        &expected_states[index],
                        index != 0
                    ),
                    "native winding must include source_mesh's index reversal and BWM flipped flag",
                );
                assert_eq!(instance.scissor.width, dimensions.width);
                assert_eq!(instance.scissor.height, dimensions.height);
                let Some(alpha) = &instance.alpha_material else {
                    panic!("alpha source binding omitted")
                };
                assert_eq!(alpha.uv_set, 1);
                assert_eq!(
                    alpha.uv_scale_bias,
                    [2.0, 3.0, 0.25, 0.5],
                    "alpha UV state comes from the typed source override, not the default"
                );
                let native_shadow_scene::NativeShadowInstanceConstants::World {
                    constants,
                    visible_indices,
                    matrices,
                    instance_count,
                } = &instance.per_instance_cb
                else {
                    panic!("world alpha packet used a non-world instance ABI")
                };
                let packet_offset = [2, 0, 0][index];
                assert_eq!(
                    *constants,
                    native_shadow_scene::native_world_alpha_shadow_view_cb(
                        &world
                            .resource::<NativeParallelShadowOwner>()
                            .whole_view
                            .clip,
                        packet_offset,
                        [2.0, 3.0, 0.25, 0.5],
                    )
                );
                assert_eq!(*instance_count, [1, 2, 1][index]);
                assert_eq!(
                    u32::from_le_bytes(constants[64..68].try_into().unwrap()),
                    packet_offset,
                    "packet remap offset is the selected ID stream start"
                );
                visible_buffers.push(visible_indices.id());
                transform_buffers.push(matrices.id());
            }
            assert_eq!(
                visible_buffers[0], visible_buffers[1],
                "packets in one mask share their exact remap buffer"
            );
            assert_ne!(
                visible_buffers[0], visible_buffers[2],
                "each initial mask owns a distinct ID buffer"
            );
            assert!(
                transform_buffers
                    .iter()
                    .all(|buffer| *buffer == transform_buffers[0]),
                "source 48-byte BWM affine matrix stream is shared without remapping"
            );
        }
        let buffers_after_publish = world.resource::<Assets<ShaderBuffer>>().len();
        assert_eq!(buffers_after_publish, 3);
        world
            .run_system_once(publish_initial_native_whole_view_depth)
            .unwrap();
        assert_eq!(
            world
                .resource::<native_shadow_scene::NativeShadowSceneFrame>()
                .revision,
            1,
            "the initial layer is never republished for the same owner generation"
        );
        assert_eq!(
            world.resource::<Assets<ShaderBuffer>>().len(),
            buffers_after_publish,
            "same-owner retry must not leak typed buffers"
        );
        assert!(
            !matches!(
                &world
                    .resource::<native_shadow_scene::NativeShadowSceneFrame>()
                    .state,
                native_shadow_scene::NativeShadowSceneState::Ready { .. }
            ),
            "whole-view depth is not a complete shadow frame"
        );
    }

    #[test]
    fn native_world_shadow_all_supported_programs_preserve_typed_material_inputs() {
        let world = native_world_shadow_test_world();
        let owner = native_world_shadow_test_owner(Handle::default());
        let mut mesh_assets = Assets::<Mesh>::default();
        let mut sources = NativeWorldShadowMeshSources::prepare(&world, &mut mesh_assets);
        let classes = [
            shader_cache::CachedWorldShadowClass::Opaque,
            shader_cache::CachedWorldShadowClass::AlphaTested,
            shader_cache::CachedWorldShadowClass::VistaAlpha,
            shader_cache::CachedWorldShadowClass::Vegetation,
            shader_cache::CachedWorldShadowClass::VegetationFixed,
        ];
        let mut image_assets = Assets::<Image>::default();
        let diffuse = image_assets.add(Image::default());
        for class in classes {
            let (material, materials) = native_world_shadow_test_material(class);
            sources.bind_material(0, &material).unwrap();
            sources.bind_material(1, &material).unwrap();
            sources.bind_material(2, &material).unwrap();
            let mut queue = SceneRenderQueue::default();
            if class != shader_cache::CachedWorldShadowClass::Opaque {
                queue.diffuse_image_paths = vec!["textures/native-world-test.dds".to_string()];
                queue.diffuse_image_handles = vec![Some(diffuse.clone())];
            }
            let plans = prepare_native_whole_view_masks(
                &owner,
                &world,
                &sources,
                &materials,
                &mesh_assets,
                &image_assets,
                &mut queue,
                Some([12.5, 0.125]),
                None,
            )
            .unwrap();
            assert_eq!(
                plans
                    .iter()
                    .map(|mask| mask.instance_ids.clone())
                    .collect::<Vec<_>>(),
                [vec![0, 1, 2], vec![4]]
            );
            if class == shader_cache::CachedWorldShadowClass::Vegetation {
                assert_eq!(plans[0].auxiliary, vec![[0; 8]; 3]);
                assert_eq!(plans[1].auxiliary, vec![[0; 8]]);
            } else {
                assert!(plans.iter().all(|mask| mask.auxiliary.is_empty()));
            }
            let mut buffers = Assets::<ShaderBuffer>::default();
            let layer =
                build_native_whole_view_layer(plans, &world, owner.dimensions, &mut buffers);
            assert_eq!(layer.instances.len(), 3);
            let expected_program = match class {
                shader_cache::CachedWorldShadowClass::Opaque => {
                    native_shadow_scene::NativeShadowProgram::WorldOpaque
                }
                shader_cache::CachedWorldShadowClass::AlphaTested => {
                    native_shadow_scene::NativeShadowProgram::WorldAlphaTested
                }
                shader_cache::CachedWorldShadowClass::VistaAlpha => {
                    native_shadow_scene::NativeShadowProgram::WorldVistaAlpha
                }
                shader_cache::CachedWorldShadowClass::Vegetation => {
                    native_shadow_scene::NativeShadowProgram::WorldVegetation
                }
                shader_cache::CachedWorldShadowClass::VegetationFixed => {
                    native_shadow_scene::NativeShadowProgram::WorldVegetationFixed
                }
            };
            assert!(
                layer
                    .instances
                    .iter()
                    .all(|instance| instance.program == expected_program)
            );
            if class != shader_cache::CachedWorldShadowClass::Opaque {
                let uv_set = if class == shader_cache::CachedWorldShadowClass::VistaAlpha {
                    0
                } else {
                    1
                };
                assert!(layer.instances.iter().all(|instance| {
                    instance
                        .alpha_material
                        .as_ref()
                        .is_some_and(|material| material.uv_set == uv_set)
                }));
            }
            let wind_buffers = layer
                .instances
                .iter()
                .filter_map(|instance| match &instance.per_instance_cb {
                    native_shadow_scene::NativeShadowInstanceConstants::WorldVegetation {
                        winds,
                        ..
                    } => winds.as_ref(),
                    _ => None,
                })
                .count();
            assert_eq!(
                wind_buffers,
                if class == shader_cache::CachedWorldShadowClass::Vegetation {
                    3
                } else {
                    0
                }
            );
        }
    }

    #[test]
    #[ignore = "requires installed original catalog/config and local EOTW world/entities"]
    fn native_shadow_owner_uses_level_sources_without_publishing_fake_frame() {
        use bevy::ecs::system::RunSystemOnce;
        let (_, settings) = original_settings::read_shadow_settings().unwrap();
        let preset = settings.quality_preset.unwrap();
        let catalog = archive::Catalog::open(Path::new(
            r"D:\Programs\Steam\steamapps\common\Dishonored2\base",
        ))
        .unwrap();
        let quality = load_installed_shadow_quality(&catalog, preset)
            .unwrap()
            .unwrap();
        let dimensions =
            native_shadow_target_dimensions(settings.parallel_slice_size, &quality).unwrap();
        let source_path = PathBuf::from("artifacts/eotw_p_lowchaos.bwm");
        let document =
            entities::parse(&fs::read_to_string(source_path.with_extension("entities")).unwrap())
                .unwrap();
        let environment = entities::world_environment(&document).unwrap();
        let source = String::from_utf8(
            catalog
                .read(&format!("generated/decls/env/{environment}.decl"))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let read_parent = |parent: &str| {
            catalog
                .read(&format!("generated/decls/env/{parent}.decl"))
                .map(|bytes| bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
                .map_err(|error| error.to_string())
        };
        let lighting =
            entities::environment_lighting_with_inheritance(&source, read_parent).unwrap();
        let parameters = entities::environment_render_parameters_with_inheritance(
            &environment,
            &source,
            read_parent,
        )
        .unwrap();
        let mut scene = SceneData {
            source_path: source_path.clone(),
            world: world_mesh::parse(&fs::read(&source_path).unwrap()).unwrap(),
            player_start: None,
            placed_models: Vec::new(),
            placed_lights: Vec::new(),
            environment_lighting: Some(lighting),
            environment_render_parameters: Some(parameters),
            environment_probes: None,
            shadow_quality_preset: Some(preset),
            shadow_quality: Some(quality),
            shadow_target_dimensions: Some(dimensions),
            shadow_depth_bias_clamp: Some(settings.depth_bias_clamp),
            shadow_sort_ccw: Some(settings.sort_shadows_ccw),
            shadow_cascade_controls: Some(settings.cascade_controls),
            shadow_cascade_fars: settings.cascade_fars,
            far_shadow_bounds: entities::world_far_shadow_bounds(&document).unwrap(),
            pickups: Vec::new(),
            catalog: None,
        };
        // Owner initialization must not depend on meshes/color visibility.
        // Native static query records survive even with no renderable geometry.
        scene.world.meshes.clear();
        // The native authored-area branch never reads the fallback root.
        let saved_area = scene.far_shadow_bounds;
        let saved_records = std::mem::take(&mut scene.world.visibility);
        let saved_objects = std::mem::take(&mut scene.world.objects);
        let saved_groups = std::mem::take(&mut scene.world.visibility_groups);
        let mut empty_images = Assets::<Image>::default();
        scene.far_shadow_bounds = Some([[-1.0, -2.0, -3.0], [1.0, 2.0, 3.0]]);
        let empty_owner = NativeParallelShadowOwner::from_scene(&scene, &mut empty_images).unwrap();
        assert!(empty_owner.whole_view_initial_masks.is_empty());
        assert_eq!(empty_owner.cascade_fars, [4.0, 12.0, 30.0, 75.0]);
        scene.shadow_cascade_fars = Some([3.0, 15.0, 45.0, 90.0]);
        let configured_owner = NativeParallelShadowOwner::from_scene(&scene, &mut empty_images).unwrap();
        assert_eq!(configured_owner.cascade_fars, [3.0, 15.0, 45.0, 90.0]);
        scene.shadow_cascade_fars = None;
        scene.far_shadow_bounds = Some([[0.0; 3]; 2]);
        assert!(NativeParallelShadowOwner::from_scene(&scene, &mut empty_images).is_err());
        assert_eq!(empty_images.len(), 1);
        scene.far_shadow_bounds = saved_area;
        scene.world.visibility = saved_records;
        scene.world.objects = saved_objects;
        scene.world.visibility_groups = saved_groups;
        let record_count = scene.world.visibility.len();
        let parents: Vec<usize> = scene
            .world
            .visibility_groups
            .iter()
            .map(|group| group.parent)
            .collect();
        let mut app = App::new();
        app.insert_resource(scene)
            .insert_resource(Assets::<Image>::default())
            .insert_resource(native_shadow_scene::NativeShadowSceneFrame::default())
            .add_systems(Startup, initialize_native_parallel_shadow_owner);
        app.update();
        let initial_target = app
            .world()
            .resource::<NativeParallelShadowOwner>()
            .target_image
            .id();
        app.update();
        let world = app.world_mut();
        let owner = world.resource::<NativeParallelShadowOwner>();
        assert_eq!(owner.target_image.id(), initial_target);
        assert_eq!(owner.dimensions, dimensions);
        assert_eq!(
            owner.whole_view_initial_masks.len(),
            record_count.div_ceil(4096)
        );
        for mask in &owner.whole_view_initial_masks {
            assert_eq!(mask.len(), record_count);
            assert!(parents.iter().all(|&parent| mask[parent] == 0));
        }
        assert!(
            owner
                .whole_view
                .clip
                .iter()
                .flatten()
                .all(|value| value.is_finite())
        );
        assert!(
            owner
                .hardware_bias
                .iter()
                .chain(&owner.shader_bias)
                .all(|value| value.is_finite())
        );
        let images = world.resource::<Assets<Image>>();
        assert_eq!(images.len(), 1);
        let image = images.get(&owner.target_image).unwrap();
        assert_eq!(
            image.texture_descriptor.size.depth_or_array_layers,
            dimensions.layers
        );
        assert_eq!(image.texture_descriptor.size.width, dimensions.width);
        assert!(matches!(
            world
                .resource::<native_shadow_scene::NativeShadowSceneFrame>()
                .state,
            native_shadow_scene::NativeShadowSceneState::Unavailable(_)
        ));
        // Missing source data must allocate nothing and never imply Ready.
        world.resource_mut::<SceneData>().environment_lighting = None;
        world
            .run_system_once(initialize_native_parallel_shadow_owner)
            .unwrap();
        assert!(!world.contains_resource::<NativeParallelShadowOwner>());
        assert_eq!(world.resource::<Assets<Image>>().len(), 1);
        assert!(matches!(
            world
                .resource::<native_shadow_scene::NativeShadowSceneFrame>()
                .state,
            native_shadow_scene::NativeShadowSceneState::Unavailable(_)
        ));
    }

    fn cache_test_input(gates: Option<&[u32]>) -> tome::OrdinaryDirectQueryInput<'_> {
        tome::OrdinaryDirectQueryInput {
            camera_point: [0.0; 3],
            render_rows: Mat4::IDENTITY.to_cols_array_2d(),
            cropped_rows: Mat4::IDENTITY.to_cols_array_2d(),
            initial_tile_rect: [0, 0, 8, 16],
            camera_imprecision: [0.6; 3],
            object_imprecision: 0.6,
            minimum_area_metric: 25.0 / (1920.0 * 1080.0),
            global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
            object_distance_scale: 1.0,
            winding_flip: false,
            gate_states: gates,
            output_capacity: tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
        }
    }

    #[test]
    fn tome_query_cache_requires_every_exact_input_and_unchanged_world() {
        let gates = [0u32, 1];
        let input = cache_test_input(Some(&gates));
        let key = || TomeRenderQueryKey::new(&input, 12);
        let cache = TomeRenderQueryCache {
            key: Some(key()),
            ..default()
        };
        assert!(cache.matches(&key(), false));
        assert!(!cache.matches(&key(), true));
        assert!(!cache.matches(&TomeRenderQueryKey::new(&input, 1), false));
        let changed = |input| assert!(!cache.matches(&TomeRenderQueryKey::new(&input, 12), false));
        for i in 0..3 {
            let mut other = input;
            other.camera_point[i] = -0.0; // Signed zero must not alias.
            changed(other);
            other = input;
            other.camera_point[i] = f32::from_bits(1); // Smallest camera change.
            changed(other);
            other = input;
            other.camera_imprecision[i] += 0.001;
            changed(other);
        }
        for row in 0..4 {
            for col in 0..4 {
                let mut other = input;
                other.render_rows[row][col] += 0.001;
                changed(other);
                other = input;
                other.cropped_rows[row][col] += 0.001;
                changed(other);
            }
            let mut other = input;
            other.initial_tile_rect[row] += 1;
            changed(other);
        }
        let mut other = input;
        other.object_imprecision += 0.001;
        changed(other);
        other = input;
        other.minimum_area_metric += 0.001;
        changed(other);
        other = input;
        other.global_distance_threshold += 0.001;
        changed(other);
        other = input;
        other.winding_flip = true;
        changed(other);
        other = input;
        other.output_capacity -= 1;
        changed(other);
        changed(cache_test_input(Some(&[0, 0])));
        changed(cache_test_input(None));
        changed(cache_test_input(Some(&[])));
    }

    #[test]
    fn tome_query_cache_never_keeps_failed_or_partial_job_results() {
        let input = cache_test_input(None);
        let key = || TomeRenderQueryKey::new(&input, 12);
        let mut cache = TomeRenderQueryCache::default();
        let mut output = tome::RenderQueryObjectOutput::default();
        output.state = Some(tome::RenderQueryState::from_input(&input, [0, 0, 16, 16]));
        let success = Ok(Some(output));
        cache.store(key(), &success, &[]);
        assert!(cache.matches(&key(), false));
        let mut copy = cache.output.clone().unwrap();
        copy.state.as_mut().unwrap().depth_grid.as_mut().unwrap()[0] = 1.0;
        assert_eq!(
            cache
                .output
                .as_ref()
                .unwrap()
                .state
                .as_ref()
                .unwrap()
                .depth_grid
                .as_ref()
                .unwrap()[0],
            0.0
        );
        cache.store(key(), &Err("merge failed".into()), &[]);
        assert!(!cache.matches(&key(), false));
        cache.store(key(), &success, &[]);
        let failed_job = tome::RenderQueryJobDiagnostics {
            job_index: 0,
            viewport: None,
            elapsed_ns: 1,
            progress: default(),
            error: Some("worker failed".into()),
        };
        cache.store(key(), &success, &[failed_job]);
        assert!(!cache.matches(&key(), false));
        cache.store(key(), &Ok(None), &[]);
        assert!(cache.matches(&key(), false));
        assert!(cache.output.is_none());
    }

    #[test]
    fn unavailable_or_unready_camera_replaces_model_and_texture_visibility() {
        use bevy::ecs::system::RunSystemOnce;
        for camera_count in [0, 1, 2] {
            let mut world = World::new();
            world.insert_resource(PickupState {
                items: Vec::new(),
                world: runtime::WorldState::default(),
            });
            world.insert_resource(VisibilityElapsedTiming::default());
            let entity = world.spawn(Visibility::Visible).id();
            world.insert_resource(SceneRenderQueue {
                active: 1,
                instances: vec![RenderInstance {
                    source: None,
                    mesh: Handle::default(),
                    material: MaterialHandle::Probe(Handle::default()),
                    diffuse_image_slot: None,
                    probe_index: 0,
                    transform: Transform::default(),
                    bounds: Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
                    pickup_id: None,
                    entity: Some(entity),
                    visible: true,
                }],
                visible_authored_ids: vec![1],
                visible_cell_indices: vec![1],
                visible_cell_mask: vec![2],
                light_query_state: Some(light_test_query_state()),
                ..default()
            });
            for _ in 0..camera_count {
                world.spawn(Camera3d::default());
            }
            world.run_system_once(stream_visible_models).unwrap();
            let scene = world.resource::<SceneRenderQueue>();
            assert_eq!(scene.active, 0, "camera count {camera_count}");
            assert!(!scene.instances[0].visible); // Texture streaming uses this flag too.
            assert_eq!(world.get::<Visibility>(entity), Some(&Visibility::Hidden));
            assert!(scene.visible_authored_ids.is_empty());
            assert!(scene.visible_cell_indices.is_empty());
            assert!(scene.visible_cell_mask.is_empty());
            assert!(scene.light_query_state.is_none());
            assert_eq!(world.resource::<VisibilityElapsedTiming>().samples, 1);
        }
    }

    #[test]
    fn native_mesh_winding_matches_authored_normals() {
        let points = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let raw = [0u16, 2, 1]; // Original clockwise front, outward normal +Z.
        let mesh = mesh_from_points(&points, &[[0.0; 2]; 3], &raw);
        assert_eq!(mesh.indices(), Some(&Indices::U16(vec![0, 1, 2])));
        let cross = (points[1] - points[0]).cross(points[2] - points[0]);
        let normals = mesh
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .unwrap()
            .as_float3()
            .unwrap();
        assert!(
            normals
                .iter()
                .all(|n| cross.dot(Vec3::from_array(*n)) > 0.0)
        );
        let mut packed = [[0u8; 12]; 3];
        for p in &mut packed {
            p[6] = 127;
            p[8] = 127;
            p[11] = 127;
        }
        let authored = authored_mesh(&points, &[[0.0; 2]; 3], &raw, &packed, |n| n);
        assert_eq!(authored.indices(), mesh.indices());
        let reflected: Vec<Vec3> = points
            .iter()
            .map(|p| *p * Vec3::new(-1.0, 1.0, 1.0))
            .collect();
        let mirrored = authored_mesh(&reflected, &[[0.0; 2]; 3], &raw, &packed, |n| {
            n * Vec3::new(-1.0, 1.0, 1.0)
        });
        assert_eq!(mirrored.indices(), Some(&Indices::U16(raw.to_vec())));
        let cross = (reflected[2] - reflected[0]).cross(reflected[1] - reflected[0]);
        assert!(cross.z > 0.0);
    }

    #[test]
    fn f8_replay_preserves_saved_camera_and_rejects_bad_input() {
        let report = "Camera game_xyz: [-22.121996, 93.85251, 12.256257]\nCamera direction game_xyz: [0.04506988, -0.98096955, 0.18885912]\n";
        let pose = f8_camera_pose(report).unwrap();
        assert_eq!(
            view_to_game(pose.translation),
            [-22.121996, 93.85251, 12.256257]
        );
        let expected = game_to_view([0.04506988, -0.98096955, 0.18885912]).normalize();
        assert!(pose.forward().as_vec3().distance(expected) < 1e-6);
        assert!(f8_camera_pose("").is_none());
        assert!(f8_camera_pose(&report.replace("12.256257", "NaN")).is_none());
        assert!(
            f8_camera_pose(&report.replace("0.04506988, -0.98096955, 0.18885912", "0, 0, 0"))
                .is_none()
        );
        assert_eq!(f8_reference_fov(report), Some(80.0));
        assert_eq!(
            f8_reference_fov(&format!("{report}Reference horizontal FOV degrees: 110\n")),
            Some(110.0)
        );
        for bad in ["0", "180", "NaN", "invalid"] {
            assert!(
                f8_reference_fov(&format!(
                    "{report}Reference horizontal FOV degrees: {bad}\n"
                ))
                .is_none()
            );
        }
        let rolled = format!("{report}Camera up game_xyz: [0.4, 0.2, 1]\n");
        let rolled_pose = f8_camera_pose(&rolled).unwrap();
        assert!(rolled_pose.forward().as_vec3().distance(expected) < 1e-6);
        assert!(rolled_pose.rotation.angle_between(pose.rotation) > 0.01);
        assert!(f8_camera_pose(&format!("{report}Camera up game_xyz: [0,0,0]\n")).is_none());
    }

    #[test]
    fn visibility_uses_actual_camera_fov_not_registered_default() {
        for reference in [55.0, 80.0, 110.0] {
            let projection = Projection::Perspective(PerspectiveProjection {
                fov: tome::original_render_vertical_fov_radians(reference).unwrap(),
                ..default()
            });
            assert!(
                (reference_fov_from_projection(&projection).unwrap() - reference).abs() < 0.00002
            );
        }
        assert!(
            reference_fov_from_projection(&Projection::Perspective(PerspectiveProjection {
                fov: f32::NAN,
                ..default()
            }))
            .is_err()
        );
    }

    #[test]
    fn replayed_original_pose_matches_live_query_matrix() {
        // Raw camera/query metadata captured at the user's 15-FPS location.
        let report = include_str!("../../analysis/original_user_dense_pose.f8");
        let pose = f8_camera_pose(report).unwrap();
        let reference = f8_reference_fov(report).unwrap();
        let projection = Projection::Perspective(PerspectiveProjection {
            fov: tome::original_render_vertical_fov_radians(reference).unwrap(),
            ..default()
        });
        let rows = tome::original_world_visibility_query_matrix(
            reference_fov_from_projection(&projection).unwrap(),
            1920,
            1080,
            1.0,
            game_pose_from_view_transform(&pose),
            ORIGINAL_DEFAULT_NEAR,
            ORIGINAL_DEFAULT_FAR,
        )
        .unwrap();
        for (index, actual) in rows.into_iter().enumerate() {
            let label = format!("Original query row{index}: ");
            let text = report
                .lines()
                .find_map(|line| line.strip_prefix(&label))
                .unwrap();
            let expected: Vec<f32> = text
                .trim_matches(['[', ']'])
                .split(',')
                .map(|part| part.trim().parse().unwrap())
                .collect();
            for lane in 0..4 {
                assert!(
                    (actual[lane] - expected[lane]).abs() < 0.00004,
                    "native row{index} lane{lane}: {} != {}",
                    actual[lane],
                    expected[lane]
                );
            }
        }
    }

    #[test]
    #[ignore="requires installed material/program cache; writes local skin shader evidence"]
    fn installed_model_skin_shadow_stage_evidence() {
        use render_program::StageBytecode;
        let base=Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base");
        let catalog=archive::Catalog::open(base).unwrap();
        let mut library=shader_cache::NativeProgramLibrary::open(base,shader_cache::ShaderCacheBackend::Pc,&catalog);
        let directory=Path::new("artifacts/native-actual-skin-shadow-source");fs::create_dir_all(directory).unwrap();
        let mut found=HashSet::new();let mut materials=0;let mut misses=0;
        for path in catalog.paths_containing("generated/decls/material/") {
            let bytes=catalog.read(&path).unwrap().unwrap();
            if !entities::native_declaration_text(&bytes).to_ascii_lowercase().contains("useskinning") {continue;}
            let surface=material_surface::Surface::parse(&bytes);
            let Some(main)=surface.main_program.as_deref() else {continue;};
            let Some(program)=catalog.read(&format!("generated/decls/renderprog/{}.decl",main.to_ascii_lowercase())).unwrap() else {continue;};
            let source=resolve_native_shadow_source_inputs(Some(&mut library),&bytes,&program,Some(main)).unwrap();
            if source.is_cloth {continue;}
            let name=match source.source {material::ShadowProgramSource::Disabled=>continue,
                material::ShadowProgramSource::Authored=>source.override_program.as_deref().unwrap(),
                material::ShadowProgramSource::MainProgram=>main};
            if name.is_empty() {continue;}
            let library=library.as_mut().unwrap();
            let overrides=native_shadow_variant_overrides(library,&bytes,&source,Some(main),name).unwrap();
            let skin4=library.option_value(name.as_bytes(),&overrides,b"useSkinning").unwrap()!=0;
            let skin8=library.option_value(name.as_bytes(),&overrides,b"useSkinning8").unwrap()!=0;
            if !skin4 && !skin8 {continue;}
            let Some(variant)=library.serialized_variant(name.as_bytes(),&overrides).unwrap() else {
                misses+=1;eprintln!("SKIN ACTUAL RESIDENT MISS {path}");continue;
            };
            materials+=1;
            for stage in variant.stages {
                let data=match stage.bytecode {StageBytecode::Absent=>continue,StageBytecode::Inline(bytes)=>bytes,
                    StageBytecode::Cached(id)=>library.cache.stage(id as usize).unwrap().bytes};
                let hash=data[4..20].iter().map(|v|format!("{v:02x}")).collect::<String>();
                if found.insert(hash.clone()) {
                    let file=directory.join(format!("skin{}-stage{}-{hash}.dxbc",if skin8 {8}else{4},stage.kind));
                    fs::write(&file,&data).unwrap();eprintln!("SKIN ACTUAL {} material={path}",file.display());
                }
            }
        }
        eprintln!("SKIN ACTUAL {materials} resident materials, {misses} misses, {} unique stages",found.len());
        assert!(!found.is_empty());
    }

    #[test]
    #[ignore = "requires a graphics adapter; validates both authored material pipelines offscreen"]
    fn authored_surface_gpu_pipelines_compile() {
        use bevy::render::render_resource::{CachedPipelineState, PipelineDescriptor};
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .set(bevy::render::RenderPlugin {
                    synchronous_pipeline_compilation: true,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        )
        .add_plugins(MaterialPlugin::<ProbeMaterial>::default())
        .add_plugins(MaterialPlugin::<DecalMaterial>::default());
        bevy::asset::load_internal_asset!(
            app,
            PROBE_SHADER_HANDLE,
            "../../assets/shaders/dh2_probe.wgsl",
            Shader::from_wgsl
        );
        bevy::asset::load_internal_asset!(
            app,
            PROBE_PREPASS_SHADER_HANDLE,
            "../../assets/shaders/dh2_probe_prepass.wgsl",
            Shader::from_wgsl
        );
        bevy::asset::load_internal_asset!(
            app,
            INDIRECT_SHADER_HANDLE,
            "../../assets/shaders/dh2_indirect.wgsl",
            Shader::from_wgsl
        );
        bevy::asset::load_internal_asset!(
            app,
            SURFACE_SHADER_HANDLE,
            "../../assets/shaders/dh2_surface.wgsl",
            Shader::from_wgsl
        );
        bevy::asset::load_internal_asset!(
            app,
            DECAL_SHADER_HANDLE,
            "../../assets/shaders/dh2_decal.wgsl",
            Shader::from_wgsl
        );
        native_shadow_sampling::register_shaders(&mut app);
        app.finish();
        app.cleanup();
        let target =
            app.world_mut()
                .resource_mut::<Assets<Image>>()
                .add(Image::new_target_texture(
                    64,
                    64,
                    TextureFormat::Rgba8UnormSrgb,
                    None,
                ));
        let mut mesh = Mesh::from(Cuboid::new(1.0, 1.0, 1.0));
        let n = mesh.count_vertices();
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.3, 0.7, 1.0, 1.0]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.25, 0.5]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.75, 0.5]; n]);
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, vec![[1.0, 0.0, 0.0, 1.0]; n]);
        mesh.insert_attribute(DH2_BITANGENT, vec![[0.0, 1.0, 0.0]; n]);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let sh = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(vec![[Vec4::ZERO; 9]]));
        let vertex_probes = app
            .world_mut()
            .resource_mut::<Assets<ShaderBuffer>>()
            .add(ShaderBuffer::from(vec![0u32; 8]));
        let native_shadow =
            app.world_mut()
                .resource_mut::<Assets<ShaderBuffer>>()
                .add(ShaderBuffer::from(
                    native_shadow_sampling::NativeParallelSamplingUniform::default(),
                ));
        let native_shadow_map = app.world_mut().resource_mut::<Assets<Image>>().add(
            native_shadow_gpu::shadow_depth_image(
                bevy::render::render_resource::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureFormat::Depth24PlusStencil8,
            )
            .unwrap(),
        );
        let native_local_shadow=app.world_mut().resource_mut::<Assets<ShaderBuffer>>().add(
            ShaderBuffer::from(vec![0u32;native_shadow_sampling::local_sampling_word_count(1)]));
        app.world_mut()
            .insert_resource(native_shadow_scene::NativeShadowSceneFrame::default());
        app.world_mut()
            .insert_resource(native_shadow_sampling::NativeParallelSamplingBuffer {
                buffer: native_shadow.clone(),
                unavailable_depth: native_shadow_map.clone(),
            });
        app.add_systems(
            PostUpdate,
            (sync_native_shadow_target::<ProbeExtension>,sync_native_shadow_target::<DecalExtension>).after(bevy::asset::AssetEventSystems),
        );
        let mut surface = surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(&[]));
        surface.misc = UVec4::new(1, 1, 1, 2);
        surface.flags = UVec4::new(2, 1, 0, 1);
        let probe = app
            .world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .add(ProbeMaterial {
                base: StandardMaterial::default(),
                extension: ProbeExtension {
                    sh,
                    vertex_probes,
                    native_shadow: native_shadow.clone(),
                    native_local_shadow: native_local_shadow.clone(),
                    native_local_shadow_map: Some(native_shadow_map.clone()),
                    native_shadow_map: Some(native_shadow_map.clone()),
                    native_screen_map: None,
                    native_cloud_map: None,
                    surface: surface.clone(),
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
                    specular0:None,specular1:None,specular2:None,
                },
            });
        // Establish the target, then add a material without a binding while the
        // target stays unchanged. The event-driven sync must repair it before
        // render extraction, without a full material scan every frame.
        app.update();
        let mut late = app
            .world()
            .resource::<Assets<ProbeMaterial>>()
            .get(&probe)
            .unwrap()
            .clone();
        late.extension.native_shadow_map = None;
        let late = app
            .world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .add(late);
        app.update();
        assert_eq!(
            app.world()
                .resource::<Assets<ProbeMaterial>>()
                .get(&late)
                .unwrap()
                .extension
                .native_shadow_map
                .as_ref(),
            Some(&native_shadow_map)
        );
        let mut vegetation = app
            .world()
            .resource::<Assets<ProbeMaterial>>()
            .get(&probe)
            .unwrap()
            .clone();
        vegetation.extension.surface =
            surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
                b"mainProgram arkVegetation\nhasBumpMap 1\nhasGlossMap 1\nalphaTest 1\n",
            ));
        vegetation.extension.surface.misc.w = 1 | 4;
        vegetation.base.alpha_mode = AlphaMode::AlphaToCoverage;
        vegetation.base.double_sided = true;
        let mut cutout = vegetation.clone();
        cutout.base.alpha_mode = AlphaMode::Mask(0.5);
        let cutout = app
            .world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .add(cutout);
        let vegetation = app
            .world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .add(vegetation);
        let decal_probe_buffers={let p=app.world().resource::<Assets<ProbeMaterial>>().get(&probe).unwrap();(p.extension.sh.clone(),p.extension.vertex_probes.clone())};
        let decal = app
            .world_mut()
            .resource_mut::<Assets<DecalMaterial>>()
            .add(DecalMaterial {
                base: StandardMaterial {
                    alpha_mode: AlphaMode::Blend,
                    ..default()
                },
                extension: DecalExtension {
                    bias: 0,
                    sh:decal_probe_buffers.0,
                    vertex_probes:decal_probe_buffers.1,
                    native_shadow,
                    native_local_shadow,
                    native_shadow_map: Some(native_shadow_map.clone()),
                    native_local_shadow_map: Some(native_shadow_map.clone()),
                    native_screen_map: None,
                    native_cloud_map: None,
                    surface,
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
                    specular0:None,specular1:None,specular2:None,
                },
            });
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(probe.clone()),
            Transform::from_xyz(-0.6, 0.0, 0.0),
        ));
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(decal.clone()),
            Transform::from_xyz(0.6, 0.0, 0.0),
        ));
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(vegetation),
            Transform::from_xyz(0.0, 0.6, 0.0),
        ));
        app.world_mut().spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(cutout),
            Transform::from_xyz(0.0, -0.6, 0.0),
        ));
        let camera = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                DepthPrepass,
                Msaa::Sample4,
                bevy::camera::RenderTarget::Image(target.clone().into()),
                Transform::from_xyz(0.0, 0.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
            ))
            .id();
        let mut validated = 0;
        let mut prepass_validated = 0;
        for _ in 0..100 {
            app.update();
            let cache = app
                .sub_app(bevy::render::RenderApp)
                .world()
                .resource::<bevy::render::render_resource::PipelineCache>();
            validated = 0;
            prepass_validated = 0;
            for p in cache.pipelines() {
                if let CachedPipelineState::Err(e) = &p.state {
                    panic!("GPU shader failure: {e}");
                }
                if let (
                    CachedPipelineState::Ok(_),
                    PipelineDescriptor::RenderPipelineDescriptor(d),
                ) = (&p.state, &p.descriptor)
                {
                    if let Some(fragment) = d.fragment.as_ref() {
                        if fragment.shader == PROBE_SHADER_HANDLE
                            || fragment.shader == DECAL_SHADER_HANDLE
                            || fragment.shader == PROBE_PREPASS_SHADER_HANDLE
                        {
                            validated += 1;
                        }
                        if fragment.shader == PROBE_PREPASS_SHADER_HANDLE {
                            prepass_validated += 1;
                        }
                    }
                }
            }
            if validated >= 5 && prepass_validated > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            validated >= 5 && prepass_validated > 0,
            "ordinary, decal, vegetation, and alpha-cutout prepass shaders must actually compile, got {validated} pipelines / {prepass_validated} prepass"
        );

        app.world_mut().entity_mut(camera).insert(Msaa::Off);
        // Sample the actual prepass attachment, not the main-pass depth that
        // subsequent forward drawing can overwrite. It has TEXTURE_BINDING
        // but no COPY_SRC, so a one-invocation compute probe reads its center.
        fn center_depth(app: &mut App) -> f32 {
            use bevy::core_pipeline::prepass::ViewPrepassTextures;
            use bevy::render::{
                render_resource::*,
                renderer::{RenderDevice, RenderQueue},
            };
            let world = app.sub_app_mut(bevy::render::RenderApp).world_mut();
            let mut views = world.query::<&ViewPrepassTextures>();
            let view = views
                .single(world)
                .unwrap()
                .depth
                .as_ref()
                .unwrap()
                .texture
                .texture
                .create_view(&TextureViewDescriptor {aspect:TextureAspect::DepthOnly,..default()});
            let device = world.resource::<RenderDevice>();
            let queue = world.resource::<RenderQueue>();
            let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
                label: Some("test prepass depth readback"),
                source: ShaderSource::Wgsl(
                    r#"
                    @group(0) @binding(0) var depth: texture_depth_2d;
                    @group(0) @binding(1) var<storage, read_write> result: f32;
                    @compute @workgroup_size(1) fn main() {
                        result = textureLoad(depth, vec2<i32>(32, 32), 0);
                    }
                "#
                    .into(),
                ),
            });
            let pipeline =
                device
                    .wgpu_device()
                    .create_compute_pipeline(&RawComputePipelineDescriptor {
                        label: Some("test prepass depth readback"),
                        layout: None,
                        module: &module,
                        entry_point: Some("main"),
                        compilation_options: default(),
                        cache: None,
                    });
            let output = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 4,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&BufferDescriptor {
                label: None,
                size: 4,
                usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let bind = device
                .wgpu_device()
                .create_bind_group(&BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(&view),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: output.as_entire_binding(),
                        },
                    ],
                });
            let mut encoder =
                device.create_command_encoder(&CommandEncoderDescriptor { label: None });
            {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 4);
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback.slice(..).map_async(MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
                .unwrap();
            rx.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
                .unwrap();
            let bytes = readback.slice(..).get_mapped_range();
            let depth = f32::from_le_bytes(bytes[..4].try_into().unwrap());
            drop(bytes);
            readback.unmap();
            depth
        }

        let entities: Vec<_> = app
            .world_mut()
            .query_filtered::<Entity, With<Mesh3d>>()
            .iter(app.world())
            .collect();
        for entity in entities {
            app.world_mut().despawn(entity);
        }
        let image = |alpha| {
            Image::new_fill(
                bevy::render::render_resource::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[255, 255, 255, alpha],
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            )
        };
        let solid = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(image(255));
        let empty = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(image(0));
        let translucent = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(image(191));
        let split = app
            .world_mut()
            .resource_mut::<Assets<Image>>()
            .add(Image::new_fill(
                bevy::render::render_resource::Extent3d {
                    width: 2,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                &[255, 255, 255, 0, 255, 255, 255, 255],
                TextureFormat::Rgba8UnormSrgb,
                bevy::asset::RenderAssetUsages::default(),
            ));
        let mut material = app
            .world()
            .resource::<Assets<ProbeMaterial>>()
            .get(&probe)
            .unwrap()
            .clone();
        material.base = StandardMaterial {
            alpha_mode: AlphaMode::Mask(0.5),
            ..default()
        };
        material.extension.surface =
            surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(
                b"mainProgram arkDefault\nalphaTest 1\nDefaultAlphaTest {0.5}\n",
            ));
        material.extension.surface.misc.w = 1;
        material.extension.diffuse0 = Some(solid);
        material.extension.diffuse1 = Some(empty.clone());
        let mut tinted = material.clone();
        tinted.extension.surface.colors[0].w = 0.0;
        tinted.extension.surface.mask_settings[0] = UVec4::new(0, 3, 1, 0);
        let mut layered = material.clone();
        layered.extension.surface.flags.x = 1;
        layered.extension.surface.mask_settings[1] = UVec4::new(0, 3, 0, 0);
        let mut gloss_alpha = material.clone();
        gloss_alpha.extension.diffuse0 = Some(empty.clone());
        gloss_alpha.extension.surface.flags.z = 1;
        let mut vegetation = material.clone();
        vegetation.extension.diffuse0 = Some(translucent);
        vegetation.extension.surface.misc.z = 2;
        vegetation.extension.surface.alpha.x = 0.9;
        let mut uv0 = material.clone();
        uv0.extension.diffuse0 = Some(split);
        let mut uv1 = uv0.clone();
        uv1.extension.surface.uv_sets[0].x = 1;
        let mut scaled = uv1.clone();
        scaled.extension.surface.uv[0] = Vec4::new(0.0, 0.0, 0.25, 0.5);
        let mut opaque = material.clone();
        opaque.base.alpha_mode = AlphaMode::Opaque;
        opaque.extension.diffuse0 = Some(empty);
        opaque.extension.surface.misc.w = 0;
        let cases = [
            ("opaque alpha", material.clone(), true),
            ("color constant must not erase depth", tinted, true),
            ("layer1 must not erase depth", layered, true),
            (
                "gloss alpha must not turn a hole opaque",
                gloss_alpha,
                false,
            ),
            (
                "vegetation Z uses DefaultAlphaTest, not forward 0.5",
                vegetation,
                false,
            ),
            ("layer0 UV0 selects transparent texel", uv0, false),
            ("layer0 UV1 selects opaque texel", uv1, true),
            ("layer0 scale/bias applies before clipping", scaled, false),
            ("opaque Z variant never samples alpha", opaque, true),
        ];
        let mut forward = material.clone();
        forward.base.unlit = true;
        let handle = app
            .world_mut()
            .resource_mut::<Assets<ProbeMaterial>>()
            .add(material);
        let entity = app
            .world_mut()
            .spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(handle.clone()),
                Transform::default(),
            ))
            .id();
        for (label, material, expected) in cases {
            *app.world_mut()
                .resource_mut::<Assets<ProbeMaterial>>()
                .get_mut(&handle)
                .unwrap() = material;
            for _ in 0..6 {
                app.update();
            }
            let depth = center_depth(&mut app);
            assert!(depth.is_finite(), "{label}: nonfinite depth {depth}");
            assert_eq!(
                depth > 0.0,
                expected,
                "{label}: actual prepass depth {depth}"
            );
        }
        // A valid prepass silhouette alone cannot catch a later forward discard.
        app.world_mut().entity_mut(camera).remove::<DepthPrepass>();
        app.world_mut()
            .entity_mut(camera)
            .get_mut::<Camera>()
            .unwrap()
            .clear_color = bevy::camera::ClearColorConfig::Custom(Color::BLACK);
        let mut tint = forward.clone();
        tint.extension.surface.colors[0].w = 0.0;
        tint.extension.surface.mask_settings[0] = UVec4::new(0, 3, 1, 0);
        let mut layer = forward.clone();
        layer.extension.surface.flags.x = 1;
        layer.extension.surface.mask_settings[1] = UVec4::new(0, 3, 0, 0);
        let mut hole = forward.clone();
        hole.extension.diffuse0 = hole.extension.diffuse1.clone();
        let mut vertex = forward.clone();
        vertex.extension.surface.misc.y = 1;
        let mut zero_alpha = app
            .world()
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .unwrap()
            .clone();
        let n = zero_alpha.count_vertices();
        zero_alpha.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.3, 0.7, 1.0, 0.0]; n]);
        let zero_alpha = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(zero_alpha);
        // Isolate one face: reversing a closed cube still exposes its far face.
        let mut face = app
            .world()
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .unwrap()
            .clone();
        let normals = face
            .attribute(Mesh::ATTRIBUTE_NORMAL)
            .unwrap()
            .as_float3()
            .unwrap();
        let indices: Vec<u32> = face.indices().unwrap().iter().map(|i| i as u32).collect();
        let indices: Vec<u32> = indices
            .chunks_exact(3)
            .filter(|t| normals[t[0] as usize][2] > 0.5)
            .flatten()
            .copied()
            .collect();
        assert_eq!(indices.len(), 6);
        // Exercise the actual CW asset -> CCW render adapter for both sides.
        let native_front: Vec<u16> = indices
            .chunks_exact(3)
            .flat_map(|t| [t[0] as u16, t[2] as u16, t[1] as u16])
            .collect();
        face.insert_indices(native_render_indices(&native_front));
        let mut reversed = face.clone();
        reversed.insert_indices(native_render_indices(
            &indices.iter().map(|i| *i as u16).collect::<Vec<_>>(),
        ));
        let mut mirrored = mirrored_render_mesh(face.clone(), &native_front, true);
        let positions = mirrored
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .unwrap()
            .as_float3()
            .unwrap()
            .iter()
            .map(|p| [-p[0], p[1], p[2]])
            .collect::<Vec<_>>();
        mirrored.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        let mirrored = app.world_mut().resource_mut::<Assets<Mesh>>().add(mirrored);
        let face = app.world_mut().resource_mut::<Assets<Mesh>>().add(face);
        let reversed = app.world_mut().resource_mut::<Assets<Mesh>>().add(reversed);
        let mut back_cull = forward.clone();
        back_cull.base.cull_mode = Some(bevy::render::render_resource::Face::Back);
        let mut front_cull = forward.clone();
        front_cull.base.cull_mode = Some(bevy::render::render_resource::Face::Front);
        let mut no_cull = forward.clone();
        no_cull.base.cull_mode = None;
        fn forward_center(app:&mut App,target:&Handle<Image>)->[u8;4] {
            let (tx,rx)=std::sync::mpsc::channel();
            app.world_mut().spawn(Screenshot::image(target.clone())).observe(
                move |event:On<bevy::render::view::screenshot::ScreenshotCaptured>| {
                    let bytes=event.image.data.as_ref().unwrap();
                    tx.send(<[u8;4]>::try_from(&bytes[(32*64+32)*4..(32*64+32)*4+4]).unwrap()).unwrap();
                });
            for _ in 0..200 {
                app.update();if let Ok(pixel)=rx.try_recv() {return pixel;}
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            panic!("forward pixel readback did not complete");
        }
        for (label, material, mesh, expected) in [
            ("raw diffuse solid", forward, mesh.clone(), true),
            ("raw diffuse hole", hole, mesh.clone(), false),
            (
                "material tint alpha is not a second clip",
                tint,
                mesh.clone(),
                true,
            ),
            (
                "layer alpha is not a second clip",
                layer,
                mesh.clone(),
                true,
            ),
            (
                "vertex alpha is not a second clip",
                vertex,
                zero_alpha,
                true,
            ),
            (
                "native BACK keeps mirrored CW fronts",
                back_cull.clone(),
                mirrored,
                true,
            ),
            (
                "native BACK keeps front-facing triangles",
                back_cull.clone(),
                face.clone(),
                true,
            ),
            (
                "native BACK rejects reverse coincident triangles",
                back_cull,
                reversed.clone(),
                false,
            ),
            (
                "native FRONT rejects front-facing triangles",
                front_cull.clone(),
                face.clone(),
                false,
            ),
            (
                "native FRONT keeps reverse-facing triangles",
                front_cull,
                reversed.clone(),
                true,
            ),
            (
                "native NONE keeps front-facing triangles",
                no_cull.clone(),
                face.clone(),
                true,
            ),
            (
                "native NONE keeps reverse-facing triangles",
                no_cull,
                reversed,
                true,
            ),
        ] {
            *app.world_mut()
                .resource_mut::<Assets<ProbeMaterial>>()
                .get_mut(&handle)
                .unwrap() = material;
            app.world_mut().entity_mut(entity).insert(Mesh3d(mesh));
            for _ in 0..6 {
                app.update();
            }
            let pixel = forward_center(&mut app,&target);
            assert_eq!(pixel[0] > 20, expected, "{label}: forward pixel {pixel:?}");
        }
        // Exercise the actual alpha-blended decal fragment path. A late
        // streamed decal must bind the same sun map as opaque materials, and
        // sun visibility must attenuate its direct lighting before blending.
        let mut receiving=app.world().resource::<Assets<DecalMaterial>>().get(&decal).unwrap().clone();
        receiving.base=StandardMaterial {alpha_mode:AlphaMode::Blend,cull_mode:None,..default()};
        receiving.extension.surface=surface_gpu::SurfaceUniform::from(&material_surface::Surface::parse(b"mainProgram arkDecal\n"));
        receiving.extension.diffuse0=app.world().resource::<Assets<ProbeMaterial>>().get(&handle).unwrap().extension.diffuse0.clone();
        receiving.extension.surface.native_shadow=UVec4::new(1,0,1,2);
        receiving.extension.native_shadow_map=None;
        let shadow_buffer=receiving.extension.native_shadow.clone();
        let receiving=app.world_mut().resource_mut::<Assets<DecalMaterial>>().add(receiving);
        app.world_mut().despawn(entity);
        app.world_mut().spawn((MeshMaterial3d(receiving.clone()),Mesh3d(face),Transform::default()));
        app.world_mut().insert_resource(GlobalAmbientLight::NONE);
        app.world_mut().spawn((DirectionalLight {illuminance:8000.0,shadow_maps_enabled:false,..default()},Transform::default()));
        for _ in 0..6 {app.update();}
        for _ in 0..100 {
            app.update();
            let cache=app.sub_app(bevy::render::RenderApp).world().resource::<bevy::render::render_resource::PipelineCache>();
            if cache.pipelines().all(|p|matches!(p.state,CachedPipelineState::Ok(_))) {break;}
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(app.world().resource::<Assets<DecalMaterial>>().get(&receiving).unwrap().extension.native_shadow_map.as_ref(),Some(&native_shadow_map));
        for _ in 0..6 {app.update();}
        let lit=forward_center(&mut app,&target);
        let mut shadowed=native_shadow_sampling::NativeParallelSamplingUniform::default();
        // LESS against depth>=1 cannot pass at reference2, independent of the
        // unavailable texture's contents. The source-backed PCF is still used.
        shadowed.world_to_shadow=[Mat4::from_cols_array_2d(&[[0.,0.,0.,0.5],[0.,0.,0.,0.5],[0.,0.,0.,2.],[0.,0.,0.,1.]]);5];
        shadowed.cascade_fars=Vec4::splat(10.0);shadowed.shadow_size=Vec4::new(1.,1.,0.,0.);
        shadowed.config=UVec4::new(1,0,0,1);shadowed.quality=IVec4::new(2,1,0,0);
        *app.world_mut().resource_mut::<Assets<ShaderBuffer>>().get_mut(&shadow_buffer).unwrap()=ShaderBuffer::from(shadowed);
        for _ in 0..6 {app.update();}
        let dark=forward_center(&mut app,&target);
        let energy=|p:[u8;4]|p[..3].iter().map(|&v|u32::from(v)).sum::<u32>();
        assert!(energy(lit)>50 && energy(dark)*5<energy(lit),"decal must receive native sun visibility: lit={lit:?} shadowed={dark:?}");
        eprintln!("GPU decal native shadow attenuation: lit={lit:?}, shadowed={dark:?}; late material atlas association passed");
        // Compile the separate native specular-map branch, including layered
        // reuse. A valid shadow remains required after this material change.
        {
            let mut materials=app.world_mut().resource_mut::<Assets<DecalMaterial>>();
            let extension=&mut materials.get_mut(&receiving).unwrap().extension;
            extension.surface.specular_reuse=UVec4::new(0,1,2,1);
            extension.surface.flags.x=2;
            extension.specular0=extension.diffuse0.clone();
        }
        for _ in 0..6 {app.update();}
        let specular_dark=forward_center(&mut app,&target);
        assert!(energy(specular_dark)*5<energy(lit),"specular-map decal must retain shadow receiving: {specular_dark:?}");
        // Native arkDefault writes finalized lighting RGB independently of
        // diffuse alpha. arkDecal applies coverage once, in hardware blending.
        // Exercise both through the real material draw with no direct light.
        app.world_mut().entity_mut(camera).insert((
            bevy::camera::Exposure { ev100: 0.0 },
            bevy::core_pipeline::tonemapping::Tonemapping::None,
        ));
        let probe_buffer=app.world().resource::<Assets<DecalMaterial>>()
            .get(&receiving).unwrap().extension.sh.clone();
        let mut coefficients=[Vec4::ZERO;9];coefficients[0]=Vec4::splat(0.25/render_light::ARKANE_TO_WORLD_LUMINANCE);
        *app.world_mut().resource_mut::<Assets<ShaderBuffer>>()
            .get_mut(&probe_buffer).unwrap()=ShaderBuffer::from(vec![coefficients]);
        let mut pixels=Vec::new();
        for (mode,alpha) in [(AlphaMode::Opaque,1.0),(AlphaMode::Opaque,0.0),
            (AlphaMode::Blend,1.0),(AlphaMode::Blend,0.5),(AlphaMode::Blend,0.0)] {
            {
                let mut materials=app.world_mut().resource_mut::<Assets<DecalMaterial>>();
                let mut material=materials.get_mut(&receiving).unwrap();
                material.base.alpha_mode=mode;
                material.extension.surface=surface_gpu::SurfaceUniform::from(
                    &material_surface::Surface::parse(b"mainProgram arkDecal\n"));
                material.extension.surface.native_shadow=UVec4::new(1,0,1,2);
                material.extension.surface.colors[0].w=alpha;
                material.extension.surface.mask_settings[0]=UVec4::new(0,3,1,0);
                material.extension.specular0=None;
            }
            for _ in 0..12 {app.update();}
            pixels.push(forward_center(&mut app,&target));
        }
        assert!(energy(pixels[0])>30,"probe lighting must be visible: {pixels:?}");
        for c in 0..3 {assert!((i32::from(pixels[0][c])-[52,51,51][c]).abs()<=2,
            "native HDR must convert to world luminance once: {pixels:?}");}
        for c in 0..3 {assert!((i32::from(pixels[0][c])-i32::from(pixels[1][c])).abs()<=2,
            "opaque indirect light must ignore coverage alpha: {pixels:?}");}
        let linear=|v:u8|{let x=f32::from(v)/255.0;if x<=0.04045 {x/12.92}else{((x+0.055)/1.055).powf(2.4)}};
        for c in 0..3 {
            assert!((linear(pixels[3][c])/linear(pixels[2][c])-0.5).abs()<0.06,
                "decal indirect light must blend alpha once: {pixels:?}");
        }
        assert!(energy(pixels[4])<10,"zero-coverage decal must remain transparent: {pixels:?}");
        eprintln!("GPU native indirect coverage: opaque alpha1/0, blended alpha1/0.5/0 = {pixels:?}");
        native_probe_shader_matches_original(&mut app);
        tiled_depth_shader_matches_reference(&mut app);
    }

    // Execute the actual shared WGSL functions, not a test-only reimplementation.
    // Compare the complete one/two-sided packed paths against installed HLSL
    // executed by D3D11, plus native PNW vertex-reference fetch/interpolation.
    fn tiled_depth_shader_matches_reference(app: &mut App) {
        tiled_depth_shader_matches_case(app, None);
        for id in 0..64 {
            tiled_depth_shader_matches_case(app, Some(id));
        }
        eprintln!("GPU TLF depth: 4 mixed tiles and 64 original-HLSL cases passed");
        let mut raw_light = [0_u8; 0xe8];
        raw_light[0] = 1;
        raw_light[0x60..0x64].copy_from_slice(&3_u32.to_le_bytes());
        assert!(tiled_lighting::includes_local_light(0, 3, 0, raw_light[0]));
        raw_light[0x30..0x34].copy_from_slice(&(-2.0_f32).to_le_bytes());
        raw_light[0x80..0x84].copy_from_slice(&1.0_f32.to_le_bytes());
        raw_light[0x9c..0xa0].copy_from_slice(&1.0_f32.to_le_bytes());
        let mut lighting = [0_u32; 28];
        let mut used_shadows = 0;
        assert_eq!(
            render_light::prepare_tlf_lighting_packet(
                &raw_light,
                0,
                false,
                &mut used_shadows,
                &mut lighting
            ),
            (6, None)
        );
        assert_eq!(lighting[2], (-2.0_f32).to_bits());
        assert_eq!(lighting[24], u32::MAX);
        let mut point = [0_u32; 12];
        tiled_lighting::fill_light_cull_packet([0.0, 0.0, -2.0], &lighting, &mut point);
        for scattering in [false, true] {
            assert_eq!(
                tiled_cull_gpu_lists(app, &point, scattering),
                [vec![0], vec![], vec![], vec![]]
            );
        }
        let fixture = include_str!("../../analysis/native_tlf_cull_cases.txt");
        let mut checked = 0;
        for case in fixture.split("\nCASE ").skip(1) {
            let header = case.lines().next().unwrap();
            let packets = case
                .lines()
                .find_map(|line| line.strip_prefix("LIGHTS id "))
                .unwrap();
            let mut lights = Vec::new();
            for (id, packet) in packets.split_whitespace().enumerate() {
                let (index, hex) = packet.split_once(':').unwrap();
                assert_eq!(index.parse::<usize>().unwrap(), id);
                assert_eq!(hex.len(), 96);
                for word in hex.as_bytes().chunks_exact(8) {
                    lights
                        .push(u32::from_str_radix(std::str::from_utf8(word).unwrap(), 16).unwrap());
                }
            }
            let mut expected: [Vec<u32>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            for line in case.lines().filter(|line| line.starts_with("ACCEPT type=")) {
                let (kind, ids) = line
                    .strip_prefix("ACCEPT type=")
                    .unwrap()
                    .split_once(" ids=")
                    .unwrap();
                expected[kind.trim().parse::<usize>().unwrap()] = ids
                    .trim()
                    .split(',')
                    .filter(|id| !id.is_empty())
                    .map(|id| id.trim().parse().unwrap())
                    .collect();
            }
            let scattering = header.contains("scatter=1");
            let actual = tiled_cull_gpu_lists(app, &lights, scattering);
            assert_eq!(actual, expected, "original HLSL {header}");
            if scattering {
                assert_eq!(
                    tiled_cull_gpu_lists_with_prior(app, &lights, true, true),
                    expected,
                    "normal/scatter must share the pool without a counter reset between passes"
                );
            }
            checked += 1;
        }
        assert_eq!(checked, 2);
        eprintln!("GPU TLF culling: 512 original light packets and packed node chains passed");
        tiled_light_iterator_matches_reference(app);
    }

    fn tiled_light_iterator_matches_reference(app: &mut App) {
        use bevy::render::{
            render_resource::*,
            renderer::{RenderDevice, RenderQueue},
        };
        let render = app.sub_app(RenderApp).world();
        let device = render.resource::<RenderDevice>();
        let queue = render.resource::<RenderQueue>();
        let mut pool = Vec::new();
        let mut heads = [tiled_lighting::END_OF_LIGHT_LIST; 16];
        // Two by two tiles, with four horizontal type planes. Distinct chains
        // expose tile/type addressing and preserve deliberately unsorted IDs.
        for (head_index, head) in heads.iter_mut().enumerate() {
            for i in 0..40 {
                let node =
                    tiled_lighting::PackedLightNode::new(*head, (i * 37 + head_index as u32) & 511);
                *head = pool.len() as u32;
                pool.push(node);
            }
        }
        let mut source = include_str!("../../assets/shaders/dh2_tlf_list.wgsl")
            .lines()
            .filter(|line| !line.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");
        source.push_str(
            r#"
@group(0) @binding(0) var<storage> pool: array<u32>;
@group(0) @binding(1) var<storage> heads: array<u32>;
@group(0) @binding(2) var<storage, read_write> result: array<u32>;
@compute @workgroup_size(64)
fn exercise_lists(@builtin(global_invocation_id) gid: vec3<u32>) {
    if gid.x >= 64u { return; }
    let tile = gid.x & 3u;
    let kind = (gid.x >> 2u) & 3u;
    let maximum = array<u32, 4>(0u, 1u, 32u, 64u)[gid.x >> 4u];
    let pixel = vec2<u32>((tile & 1u) * 16u + 11u, (tile >> 1u) * 16u + 7u);
    let coordinate = light_head_coordinate(pixel, kind, 4u, 2u);
    var list = begin_light_list(heads[coordinate.x + coordinate.y * 8u]);
    let base = gid.x * 65u;
    while has_light(list, maximum) {
        let packed = pool[list.head];
        let index = advance_light(&list, packed);
        result[base + list.count] = index;
    }
    result[base] = list.count;
}
"#,
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("native PC LightContext consumer"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let pipeline =
            device
                .wgpu_device()
                .create_compute_pipeline(&RawComputePipelineDescriptor {
                    label: Some("native PC LightContext consumer"),
                    layout: None,
                    module: &module,
                    entry_point: Some("exercise_lists"),
                    compilation_options: default(),
                    cache: None,
                });
        let bytes = |words: &[u32]| {
            words
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<_>>()
        };
        let pool_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes(&pool.iter().map(|node| node.0).collect::<Vec<_>>()),
            usage: BufferUsages::STORAGE,
        });
        let heads_buffer = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes(&heads),
            usage: BufferUsages::STORAGE,
        });
        let size = 64 * 65 * 4;
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size,
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
                        resource: pool_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: heads_buffer.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: output.as_entire_binding(),
                    },
                ],
            });
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(MapMode::Read, move |result| tx.send(result).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let mapped = readback.slice(..).get_mapped_range();
        let words = mapped
            .chunks_exact(4)
            .map(|word| u32::from_le_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        let grid = tiled_lighting::TileGrid::new(32, 32, 16).unwrap();
        for case in 0..64usize {
            let tile = case & 3;
            let coordinate = grid.light_head_coordinate(
                [((tile & 1) * 16 + 11) as u32, ((tile >> 1) * 16 + 7) as u32],
                ((case >> 2) & 3) as u32,
            );
            let head = heads[(coordinate[0] + coordinate[1] * 8) as usize];
            let mut expected = Vec::new();
            tiled_lighting::visit_light_list(&pool, head, [0, 1, 32, 64][case >> 4], |id| {
                expected.push(id)
            })
            .unwrap();
            assert_eq!(
                words[case * 65] as usize,
                expected.len(),
                "PC LightContext case {case}"
            );
            assert_eq!(
                &words[case * 65 + 1..case * 65 + 1 + expected.len()],
                expected.as_slice(),
                "PC LightContext order {case}"
            );
        }
        drop(mapped);
        readback.unmap();
        eprintln!("GPU TLF PC consumer: 64 tile/type/limit/order cases passed");
    }

    fn tiled_cull_gpu_lists(app: &mut App, lights: &[u32], scattering: bool) -> [Vec<u32>; 4] {
        tiled_cull_gpu_lists_with_prior(app, lights, scattering, false)
    }

    fn tiled_cull_gpu_lists_with_prior(
        app: &mut App,
        lights: &[u32],
        scattering: bool,
        prior_normal: bool,
    ) -> [Vec<u32>; 4] {
        assert!(!prior_normal || scattering);
        use bevy::render::{
            render_resource::*,
            renderer::{RenderDevice, RenderQueue},
        };
        use std::sync::mpsc;
        assert_eq!(lights.len() % 12, 0);
        let count = lights.len() / 12;
        assert!(count > 0 && count <= tiled_lighting::LIGHT_BUFFER_CAPACITY);
        let render = app.sub_app_mut(RenderApp).world_mut();
        render.init_resource::<tiled_lighting_gpu::NativeTlfPipelines>();
        let device = render.resource::<RenderDevice>();
        let queue = render.resource::<RenderQueue>();
        let pipeline = &render
            .resource::<tiled_lighting_gpu::NativeTlfPipelines>()
            .cull;
        let mut params = vec![
            0.0f32.to_bits(),
            0.0f32.to_bits(),
            1.0f32.to_bits(),
            1.0f32.to_bits(),
        ];
        params.extend([1.0f32.to_bits(); 4]);
        for _ in 0..2 {
            for row in 0..4 {
                for column in 0..4 {
                    params.push(if row == column { 1.0f32.to_bits() } else { 0 });
                }
            }
        }
        params.extend([count.div_ceil(64) as u32 * 64, u32::from(scattering), 0, 0]);
        assert_eq!(params.len(), 44);
        let bytes = |values: &[u32]| {
            values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes(&params),
            usage: BufferUsages::UNIFORM,
        });
        let buffers = tiled_lighting_gpu::NativeTlfBuffers::new(device);
        // The original-HLSL fixture supplies cull records. Reconstruct its
        // corresponding lighting fields to exercise the shared producer/upload
        // path too; the 1-light case additionally dispatches all 63 padding lanes.
        let lighting = lights
            .chunks_exact(12)
            .map(|cull| {
                let mut record = [0_u32; 28];
                record[..3].copy_from_slice(&cull[..3]);
                for (src, dst) in (3..11).zip([7, 8, 9, 10, 13, 16, 17, 18]) {
                    record[dst] = cull[src];
                }
                record[25] = cull[11];
                record
            })
            .collect::<Vec<_>>();
        let identity = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ];
        assert_eq!(
            buffers
                .upload_lighting_packets(queue, &identity, &lighting)
                .unwrap(),
            count.div_ceil(64) * 64
        );
        let packets = buffers
            .resource(render_resource::resource_name_key("lightToPutInTiles"))
            .unwrap();
        let counter = buffers
            .resource(render_resource::resource_name_key(
                "lightNodePoolNextFreeEntry",
            ))
            .unwrap();
        let pool = buffers
            .resource(render_resource::resource_name_key("lightNodePool"))
            .unwrap();
        assert!(
            buffers
                .resource(render_resource::resource_name_key("missingNativeResource"))
                .is_none()
        );
        let heads = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        });
        // Native fixed allocations persist across all frames below.
        let depth = device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba32Float,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            depth.as_image_copy(),
            &bytes(&[-0.5f32, -10.0, -4.0, f32::from_bits(u32::MAX)].map(f32::to_bits)),
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16),
                rows_per_image: Some(1),
            },
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let view = depth.create_view(&default());
        let layout = pipeline.get_bind_group_layout(0);
        let group = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: uniform.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: packets.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: heads.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: counter.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: pool.as_entire_binding(),
                    },
                ],
            });
        let prior_heads = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]),
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
        });
        params[41] = 0;
        let prior_uniform = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &bytes(&params),
            usage: BufferUsages::UNIFORM,
        });
        let prior_group = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &layout,
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: prior_uniform.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: packets.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: prior_heads.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: counter.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 5,
                        resource: pool.as_entire_binding(),
                    },
                ],
            });
        let pool_words = count * if prior_normal { 2 } else { 1 };
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 36 + pool_words as u64 * 4,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut previous = None;
        for frame in 0..3 {
            // Each original-HLSL fixture starts with empty heads. They are
            // caller-owned inputs; cull_lights only exchanges/appends them.
            queue.write_buffer(&heads, 0, &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]));
            queue.write_buffer(
                &prior_heads,
                0,
                &bytes(&[tiled_lighting::END_OF_LIGHT_LIST; 4]),
            );
            let mut encoder = device.create_command_encoder(&default());
            // Native351780 clears once BEFORE both passes, never between them.
            buffers.begin_frame(&mut encoder);
            if prior_normal {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &prior_group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            {
                let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&heads, 0, &readback, 0, 16);
            encoder.copy_buffer_to_buffer(&prior_heads, 0, &readback, 16, 16);
            encoder.copy_buffer_to_buffer(counter, 0, &readback, 32, 4);
            encoder.copy_buffer_to_buffer(pool, 0, &readback, 36, pool_words as u64 * 4);
            queue.submit([encoder.finish()]);
            let (tx, rx) = mpsc::channel();
            readback.slice(..).map_async(MapMode::Read, move |r| {
                tx.send(r).unwrap();
            });
            device
                .poll(PollType::Wait {
                    submission_index: None,
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
                .unwrap();
            rx.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
                .unwrap();
            let bytes = readback.slice(..).get_mapped_range();
            let words: Vec<u32> = bytes
                .chunks_exact(4)
                .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
                .collect();
            let used = words[8] as usize;
            assert!(used <= pool_words);
            let mut seen = HashSet::new();
            let mut result = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
            for chain in 0..if prior_normal { 8 } else { 4 } {
                let kind = chain % 4;
                let mut cursor = words[chain];
                while cursor != tiled_lighting::END_OF_LIGHT_LIST {
                    assert!((cursor as usize) < used);
                    assert!(
                        seen.insert(cursor),
                        "duplicate/cyclic TLF node or counter reset between passes"
                    );
                    let packed = tiled_lighting::PackedLightNode(words[9 + cursor as usize]);
                    let id = packed.light_index();
                    assert!((id as usize) < count);
                    assert_eq!((lights[id as usize * 12 + 11] >> 4) & 7, kind as u32);
                    if chain < 4 {
                        result[kind].push(id);
                    }
                    cursor = packed.next();
                }
                if chain < 4 {
                    result[kind].sort_unstable();
                }
            }
            if prior_normal {
                assert!(
                    seen.len() > result.iter().map(Vec::len).sum::<usize>(),
                    "normal prefix did not exercise shared allocation"
                );
            }
            assert_eq!(seen.len(), used);
            drop(bytes);
            readback.unmap();
            if let Some(before) = &previous {
                assert_eq!(
                    &result, before,
                    "native TLF buffers accumulated stale links on frame {frame}"
                );
            }
            previous = Some(result);
        }
        previous.unwrap()
    }

    fn depth_copy_shader_matches_fixture(
        device: &bevy::render::renderer::RenderDevice,
        queue: &bevy::render::renderer::RenderQueue,
        pipelines: &tiled_lighting_gpu::NativeTlfPipelines,
        source: &bevy::render::render_resource::Texture,
        expected: &[f32],
        entry: &str,
    ) {
        use bevy::render::render_resource::*;
        let array_source = entry == "fragment";
        let output_size = if entry == "downscale" { 16 } else { 32 };
        let module = &pipelines.depth_copy;
        // Caller-owned fixture layout; not the original 32-byte vertex ABI.
        let vertex_layout = [RawVertexBufferLayout {
            array_stride: 20,
            step_mode: VertexStepMode::Vertex,
            attributes: &[
                VertexAttribute {
                    format: VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                },
                VertexAttribute {
                    format: VertexFormat::Float32x2,
                    offset: 12,
                    shader_location: 1,
                },
            ],
        }];
        let pipeline = if array_source {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("native depthcopy fixture"),
                layout: None,
                vertex: RawVertexState {
                    module,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
                    buffers: &vertex_layout,
                },
                fragment: Some(RawFragmentState {
                    module,
                    entry_point: Some(entry),
                    compilation_options: default(),
                    targets: &[],
                }),
                primitive: default(),
                multisample: default(),
                multiview_mask: None,
                cache: None,
                depth_stencil: Some(DepthStencilState {
                    format: TextureFormat::Depth32Float,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(CompareFunction::Always),
                    stencil: default(),
                    bias: default(),
                }),
            })
        } else {
            pipelines.depth_copy_pipeline(
                device,
                TextureFormat::Depth32Float,
                entry == "downscale",
                &vertex_layout,
            )
        };
        let destination = device.create_texture(&TextureDescriptor {
            label: Some("copied depth fixture"),
            size: Extent3d {
                width: output_size,
                height: output_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Depth32Float,
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let source_view = source.create_view(&TextureViewDescriptor {
            dimension: Some(if array_source {
                TextureViewDimension::D2Array
            } else {
                TextureViewDimension::D2
            }),
            ..default()
        });
        let destination_view = destination.create_view(&default());
        let sampler = device.create_sampler(&SamplerDescriptor {
            min_filter: FilterMode::Nearest,
            mag_filter: FilterMode::Nearest,
            ..default()
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 32 * 256,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        for scale_bias in [
            [1_f32, 1., 0., 0.],
            [0.5, 0.5, 0.25, 0.25],
            [-1., 1., 1., 0.],
        ] {
            let mut vertex_data = [
                -1_f32, -1., 0., 0., 1., 3., -1., 0., 2., 1., -1., 3., 0., 0., -1.,
            ];
            if !array_source {
                for v in vertex_data.chunks_mut(5) {
                    v[3] = v[3] * scale_bias[0] + scale_bias[2];
                    v[4] = v[4] * scale_bias[1] + scale_bias[3];
                }
            }
            let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents: &vertex_data
                    .into_iter()
                    .flat_map(f32::to_le_bytes)
                    .collect::<Vec<_>>(),
                usage: BufferUsages::VERTEX,
            });
            let parameters = array_source.then(|| {
                let mut words = scale_bias.map(f32::to_bits).to_vec();
                words.extend([0; 4]);
                device.create_buffer_with_data(&BufferInitDescriptor {
                    label: None,
                    contents: &words
                        .into_iter()
                        .flat_map(u32::to_le_bytes)
                        .collect::<Vec<_>>(),
                    usage: BufferUsages::UNIFORM,
                })
            });
            let mut bindings = vec![
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&source_view),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&sampler),
                },
            ];
            if let Some(parameters) = &parameters {
                bindings.push(BindGroupEntry {
                    binding: 2,
                    resource: parameters.as_entire_binding(),
                });
            }
            let bind = device
                .wgpu_device()
                .create_bind_group(&BindGroupDescriptor {
                    label: None,
                    layout: &pipeline.get_bind_group_layout(0),
                    entries: &bindings,
                });
            let mut encoder = device.create_command_encoder(&default());
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: None,
                    color_attachments: &[],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: &destination_view,
                        depth_ops: Some(Operations {
                            load: LoadOp::Clear(0.333),
                            store: StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind, &[]);
                pass.set_vertex_buffer(0, *vertices.slice(..));
                pass.draw(0..3, 0..1);
            }
            encoder.copy_texture_to_buffer(
                TexelCopyTextureInfo {
                    texture: &destination,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::DepthOnly,
                },
                TexelCopyBufferInfo {
                    buffer: &readback,
                    layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(32),
                    },
                },
                Extent3d {
                    width: output_size,
                    height: output_size,
                    depth_or_array_layers: 1,
                },
            );
            let submission = queue.submit([encoder.finish()]);
            let (tx, rx) = std::sync::mpsc::channel();
            readback.slice(..).map_async(MapMode::Read, move |result| {
                tx.send(result).unwrap();
            });
            device
                .poll(PollType::Wait {
                    submission_index: Some(submission),
                    timeout: Some(std::time::Duration::from_secs(10)),
                })
                .unwrap();
            rx.recv_timeout(std::time::Duration::from_secs(10))
                .unwrap()
                .unwrap();
            let bytes = readback.slice(..).get_mapped_range();
            for y in 0..output_size as usize {
                for x in 0..output_size as usize {
                    let footprint = if entry == "downscale" { 0.5 } else { 0.0 };
                    let ratio = 32.0 / output_size as f32;
                    let sx = (((x as f32 + 0.5) * ratio * scale_bias[0] + 32. * scale_bias[2])
                        - footprint)
                        .floor() as i32;
                    let sy = (((y as f32 + 0.5) * ratio * scale_bias[1] + 32. * scale_bias[3])
                        - footprint)
                        .floor() as i32;
                    let sample = |dx: i32, dy: i32| {
                        expected
                            [(sy + dy).clamp(0, 31) as usize * 32 + (sx + dx).clamp(0, 31) as usize]
                    };
                    let predicted = if entry == "downscale" {
                        sample(0, 0)
                            .min(sample(1, 0))
                            .min(sample(0, 1).min(sample(1, 1)))
                    } else {
                        sample(0, 0)
                    };
                    let at = y * 256 + x * 4;
                    assert_eq!(
                        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()),
                        predicted.to_bits(),
                        "depthcopy {entry} {scale_bias:?} pixel{x},{y}"
                    );
                }
            }
            drop(bytes);
            readback.unmap();
        }
    }

    fn tiled_depth_shader_matches_case(app: &mut App, native_case: Option<usize>) {
        use bevy::render::{
            render_resource::*,
            renderer::{RenderDevice, RenderQueue},
        };
        let world = app.sub_app_mut(bevy::render::RenderApp).world_mut();
        world.init_resource::<tiled_lighting_gpu::NativeTlfPipelines>();
        let device = world.resource::<RenderDevice>();
        let queue = world.resource::<RenderQueue>();
        let pipeline = &world
            .resource::<tiled_lighting_gpu::NativeTlfPipelines>()
            .depth;
        let make_depth = || {
            device.create_texture(&TextureDescriptor {
                label: None,
                size: Extent3d {
                    width: 32,
                    height: 32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Depth32Float,
                usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        let opaque = make_depth();
        let alpha = make_depth();
        let output = device.create_texture(&TextureDescriptor {
            label: None,
            size: Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba32Float,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let original = native_case.map(tiled_lighting::original_depth_case);
        let unproject = original.as_ref().map_or([-0.1, 0.00001], |v| v.1);
        let mut depths = [Vec::<f32>::new(), Vec::<f32>::new()];
        for y in 0..32 {
            for x in 0..32 {
                let tile = x / 16 + 2 * (y / 16);
                if let Some((input, _, _)) = &original {
                    let pixel = (y % 16) * 16 + x % 16;
                    depths[0].push(input[pixel]);
                    depths[1].push(input[pixel + 256]);
                } else {
                    depths[0].push(if tile == 3 {
                        0.0
                    } else {
                        ((x + y) % 4 + 1) as f32 * 0.25
                    });
                    depths[1].push(if tile == 3 || (x + y) % 3 == 0 {
                        0.0
                    } else {
                        0.125 + ((x * 3 + y) % 5) as f32 * 0.125
                    });
                }
            }
        }
        // Depth32Float cannot be Queue::write_texture'd. Populate actual
        // depth attachments through the rasterizer, as the engine does.
        let fill_module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("synthetic depth fixture"),
            source: ShaderSource::Wgsl(
                r#"
                @group(0) @binding(0) var<storage,read> depths:array<f32>;
                @vertex fn vertex(@builtin(vertex_index) id:u32)->@builtin(position) vec4<f32> {
                    let p=array<vec2<f32>,3>(vec2<f32>(-1,-1),vec2<f32>(3,-1),vec2<f32>(-1,3));
                    return vec4<f32>(p[id],0,1);
                }
                @fragment fn fragment(@builtin(position) p:vec4<f32>)->@builtin(frag_depth) f32 {
                    return depths[u32(p.y)*32u+u32(p.x)];
                }
            "#
                .into(),
            ),
        });
        let fill_pipeline =
            device
                .wgpu_device()
                .create_render_pipeline(&RawRenderPipelineDescriptor {
                    label: None,
                    layout: None,
                    vertex: RawVertexState {
                        module: &fill_module,
                        entry_point: Some("vertex"),
                        compilation_options: default(),
                        buffers: &[],
                    },
                    fragment: Some(RawFragmentState {
                        module: &fill_module,
                        entry_point: Some("fragment"),
                        compilation_options: default(),
                        targets: &[],
                    }),
                    primitive: default(),
                    multisample: default(),
                    multiview_mask: None,
                    cache: None,
                    depth_stencil: Some(DepthStencilState {
                        format: TextureFormat::Depth32Float,
                        depth_write_enabled: Some(true),
                        depth_compare: Some(CompareFunction::Always),
                        stencil: default(),
                        bias: default(),
                    }),
                });
        let mut fill_encoder = device.create_command_encoder(&default());
        for (texture, values) in [(&opaque, &depths[0]), (&alpha, &depths[1])] {
            let bytes = values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>();
            let data = device.create_buffer_with_data(&BufferInitDescriptor {
                label: None,
                contents: &bytes,
                usage: BufferUsages::STORAGE,
            });
            let bind = device
                .wgpu_device()
                .create_bind_group(&BindGroupDescriptor {
                    label: None,
                    layout: &fill_pipeline.get_bind_group_layout(0),
                    entries: &[BindGroupEntry {
                        binding: 0,
                        resource: data.as_entire_binding(),
                    }],
                });
            let view = texture.create_view(&default());
            let mut pass = fill_encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                color_attachments: &[],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.0),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&fill_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
        }
        queue.submit([fill_encoder.finish()]);
        if native_case.is_none() {
            let pipelines = world.resource::<tiled_lighting_gpu::NativeTlfPipelines>();
            for entry in ["fragment", "identity", "downscale"] {
                depth_copy_shader_matches_fixture(
                    device, queue, pipelines, &opaque, &depths[0], entry,
                );
            }
        }
        let params = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &[
                unproject[0].to_bits(),
                unproject[1].to_bits(),
                0,
                0,
                16,
                0,
                0,
                0,
            ]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>(),
            usage: BufferUsages::UNIFORM,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            ..default()
        });
        let opaque_view = opaque.create_view(&default());
        let alpha_view = alpha.create_view(&default());
        let output_view = output.create_view(&default());
        let bind = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(&opaque_view),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(&alpha_view),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Sampler(&sampler),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: params.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::TextureView(&output_view),
                    },
                ],
            });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size: 512,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(2, 2, 1);
        }
        encoder.copy_texture_to_buffer(
            TexelCopyTextureInfo {
                texture: &output,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            TexelCopyBufferInfo {
                buffer: &readback,
                layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(256),
                    rows_per_image: Some(2),
                },
            },
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
        );
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        for ty in 0..2 {
            for tx in 0..2 {
                let mut tile = Vec::new();
                for source in &depths {
                    for y in ty * 16..ty * 16 + 16 {
                        for x in tx * 16..tx * 16 + 16 {
                            tile.push(source[y * 32 + x]);
                        }
                    }
                }
                let expected = original.as_ref().map(|v| v.2).unwrap_or_else(|| {
                    tiled_lighting::reduce_depth_tile(&tile, unproject).map(f32::to_bits)
                });
                for lane in 0..4 {
                    let at = ty * 256 + tx * 16 + lane * 4;
                    let actual = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
                    if lane == 3 {
                        assert_eq!(
                            actual, expected[lane],
                            "case{native_case:?} tile{tx},{ty} depth mask"
                        );
                    } else {
                        assert!(
                            actual.abs_diff(expected[lane]) <= 1,
                            "case{native_case:?} tile{tx},{ty} endpoint{lane}: {actual:08x} vs {:08x}",
                            expected[lane]
                        );
                    }
                }
            }
        }
        drop(bytes);
        readback.unmap();
    }

    fn native_probe_shader_matches_original(app: &mut App) {
        use bevy::render::{
            render_resource::*,
            renderer::{RenderDevice, RenderQueue},
        };
        let world = app.sub_app_mut(bevy::render::RenderApp).world_mut();
        let device = world.resource::<RenderDevice>();
        let queue = world.resource::<RenderQueue>();
        let mut source = include_str!("../../assets/shaders/dh2_indirect.wgsl")
            .replace("#define_import_path dh2_indirect", "")
            .replace("#{MATERIAL_BIND_GROUP}", "0");
        source.push_str(r#"
@group(0) @binding(120) var<storage, read_write> result: array<vec4<f32>>;
fn view_vector(v: vec3<f32>) -> vec3<f32> { return vec3(v.x,v.z,-v.y); }
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) invocation: vec3<u32>) {
    let id = invocation.x;
    let sh = native_vertex_sh(id, 0u, vec3(0.0));
    let d = vec3<f32>(vec3<i32>(i32(id%5u)-2,i32((id/5u)%5u)-2,i32((id/25u)%5u)-2))*0.25*select(1.0,3.0,id>=32u);
    var t = vec3(1.0,0.0,0.0); var b = vec3(0.0,1.0,0.0); var n = vec3(0.0,0.0,1.0);
    if (id & 1u) != 0u { t = vec3(0.0,1.0,0.0); b = vec3(-1.0,0.0,0.0); }
    if (id & 2u) != 0u { b = -b; n = -n; }
    let ao = f32(id%5u)*0.25;
    let one = pack_vertex_irradiance(sh,view_vector(t),view_vector(b),view_vector(n),false);
    let two = pack_vertex_irradiance(sh,view_vector(t),view_vector(b),view_vector(n),true);
    let at = id*9u;
    result[at] = vec4(irradiance(d,sh),0.0);
    result[at+1u] = vec4(one.luminance,0.0,0.0,0.0);
    result[at+2u] = one.chroma_a; result[at+3u] = one.chroma_b;
    result[at+4u] = vec4(unpack_diffuse(one,view_vector(d),view_vector(t),view_vector(b),view_vector(n),ao,false),0.0);
    result[at+5u] = vec4(two.luminance,0.0,0.0,0.0);
    result[at+6u] = two.chroma_a; result[at+7u] = two.chroma_b;
    result[at+8u] = vec4(unpack_diffuse(two,view_vector(d),view_vector(t),view_vector(b),view_vector(n),ao,true),0.0);
    let albedo=vec3(f32(id%17u),f32((id*3u)%17u),f32((id*7u)%17u))/16.0;
    result[64u*9u+4u*9u+id]=vec4(native_diffuse_response(albedo,f32(id%9u)/8.0,
        f32((id/3u)%5u)/4.0,vec3(.3,.5,.7),(id&1u)==0u,vec3(.04,.02,.08),vec3(.01,.02,.03)),0.0);
    if id < 4u {
        var p = vec3(0.25);
        if id == 1u { p = vec3(1.0,0.0,0.0); }
        if id == 2u { p = vec3(-0.25,1.25,-0.5); }
        if id == 3u { p = vec3(0.0); }
        let interpolated = native_vertex_sh(0x80000000u,0u,view_vector(p));
        for (var i = 0u; i < 9u; i++) { result[64u*9u+id*9u+i] = vec4(interpolated[i],0.0); }
    }
}
"#);
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("original PNW vertex SH numerical parity"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let pipeline =
            device
                .wgpu_device()
                .create_compute_pipeline(&RawComputePipelineDescriptor {
                    label: None,
                    layout: None,
                    module: &module,
                    entry_point: Some("main"),
                    compilation_options: default(),
                    cache: None,
                });
        let mut sh = Vec::new();
        for id in 0..64 {
            for i in 0..9 {
                for c in 0..4 {
                    sh.push(if c == 3 || id == 0 {
                        0.0f32
                    } else {
                        (((id * 17 + i * 5 + c * 3) % 19) as i32 - 9) as f32 / 16.0
                    });
                }
            }
        }
        let sh_bytes = sh.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<_>>();
        let coefficients = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &sh_bytes,
            usage: BufferUsages::STORAGE,
        });
        // One tetrahedron with four primary IDs and exact unit-axis positions.
        let mut words = vec![
            8u32, 9, 10, 16, 1, 1, 1, 4, 0, 0, 0, 1, 2, 3, 0x00010000, 0x00030002,
        ];
        for i in 0..4 {
            for axis in 0..3 {
                words.push((if i == axis { 1.0f32 } else { 0.0f32 }).to_bits());
            }
            words.push(255u32 << (8 * i));
        }
        let word_bytes = words
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect::<Vec<_>>();
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: None,
            contents: &word_bytes,
            usage: BufferUsages::STORAGE,
        });
        let size = (64 * 9 + 4 * 9 + 64) * 16;
        let output = device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&BufferDescriptor {
            label: None,
            size,
            usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let bind = device
            .wgpu_device()
            .create_bind_group(&BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    BindGroupEntry {
                        binding: 100,
                        resource: coefficients.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 119,
                        resource: vertices.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 120,
                        resource: output.as_entire_binding(),
                    },
                ],
            });
        let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor { label: None });
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(64, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, size);
        let submission = queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        readback.slice(..).map_async(MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });
        device
            .poll(PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(10)),
            })
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let bytes = readback.slice(..).get_mapped_range();
        let actual = bytes
            .chunks_exact(4)
            .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
            .collect::<Vec<_>>();
        drop(bytes);
        readback.unmap();
        for (id, line) in include_str!("../../analysis/native_indirect_lighting_cases.txt")
            .lines()
            .enumerate()
        {
            let expected = line
                .split_whitespace()
                .map(|v| f32::from_bits(u32::from_str_radix(v, 16).unwrap()))
                .collect::<Vec<_>>();
            for (row, native) in [2, 9, 10, 11, 16, 17, 18, 19, 20].into_iter().enumerate() {
                for c in 0..4 {
                    let a = actual[(id * 9 + row) * 4 + c];
                    let b = expected[native * 4 + c];
                    assert!(
                        (a - b).abs() <= 3e-5 * b.abs().max(1.0),
                        "WGSL/native HLSL case{id} row{row} channel{c}: {a} != {b}"
                    );
                }
            }
        }
        for (id, weights) in [
            [0.25f32; 4],
            [1.0, 0.0, 0.0, 0.0],
            [-0.25, 1.25, -0.5, 0.5],
            [0.0, 0.0, 0.0, 1.0],
        ]
        .into_iter()
        .enumerate()
        {
            for i in 0..9 {
                for c in 0..3 {
                    let expected: f32 = (0..4).map(|p| weights[p] * sh[(p * 9 + i) * 4 + c]).sum();
                    let a = actual[(64 * 9 + id * 9 + i) * 4 + c];
                    assert!(
                        (a - expected).abs() < 1e-5,
                        "PNW GPU case{id} coefficient{i} channel{c}: {a} != {expected}"
                    );
                }
            }
        }
        for(id,line) in include_str!("../../analysis/native_diffuse_response_cases.txt").lines().enumerate() {
            for(c,word) in line.split_whitespace().enumerate() {
                let expected=f32::from_bits(u32::from_str_radix(word,16).unwrap());
                let result=actual[(64*9+4*9+id)*4+c];
                assert!((result-expected).abs()<0.000003,"native diffuse response case{id} channel{c}: {result} != {expected}");
            }
        }
        eprintln!("GPU native diffuse radiance response: 64 installed-HLSL cases passed");
        eprintln!(
            "GPU native SH parity:64 original HLSL packed one/two-sided cases +4 PNW reference/interpolation cases passed"
        );
    }

    #[test]
    fn authored_mesh_retains_native_uv_color_and_independent_sheared_basis() {
        use bevy::mesh::VertexAttributeValues;
        let packed = [
            0x00, 0x38, 0x00, 0xbc, 127, 0, 0, 42, 0, 127, 0, 129, 0, 128, 255, 64,
        ];
        let linear = [
            1.0, 2.0, 3.0, 1000.0, 0.0, 1.0, 4.0, 2000.0, 0.0, 0.0, 1.0, 3000.0,
        ];
        let mesh = authored_mesh(&[Vec3::ZERO], &[[0.1, 0.2]], &[], &[packed], |v| {
            transformed_direction(&linear, v)
        });
        let f3 = |attribute| match mesh.attribute(attribute).unwrap() {
            VertexAttributeValues::Float32x3(v) => v[0],
            _ => panic!("wrong vertex type"),
        };
        assert_eq!(f3(Mesh::ATTRIBUTE_NORMAL), [1.0, 0.0, 0.0]);
        assert_eq!(f3(DH2_BITANGENT), [-3.0, -1.0, 4.0]);
        let VertexAttributeValues::Float32x4(t) = mesh.attribute(Mesh::ATTRIBUTE_TANGENT).unwrap()
        else {
            panic!()
        };
        assert_eq!(t[0], [2.0, 0.0, -1.0, -1.0]);
        let VertexAttributeValues::Float32x4(color) =
            mesh.attribute(Mesh::ATTRIBUTE_COLOR).unwrap()
        else {
            panic!()
        };
        assert_eq!(color[0], [0.0, 128.0 / 255.0, 1.0, 64.0 / 255.0]);
        let VertexAttributeValues::Float32x2(uv1) = mesh.attribute(Mesh::ATTRIBUTE_UV_1).unwrap()
        else {
            panic!()
        };
        assert_eq!(uv1[0], [0.5, -1.0]);
        // MD6 declarations omit color, not an inferred byte channel.
        let md6_packed: [u8; 12] = packed[..12].try_into().unwrap();
        let md6 = authored_mesh(&[Vec3::ZERO], &[[0.0; 2]], &[], &[md6_packed], |v| v);
        assert!(md6.attribute(Mesh::ATTRIBUTE_COLOR).is_none());
    }

    #[test]
    fn native_vegetation_channels_keep_local_inputs_on_baked_meshes() {
        use bevy::mesh::VertexAttributeValues;
        let packed = [
            0x00, 0x38, 0x00, 0xbc, 128, 64, 127, 0, 0, 127, 0, 127, 128, 64, 255, 0,
        ];
        let local = [[2.0, 3.0, 4.0]];
        let world = [Vec3::new(100.0, 200.0, 300.0)];
        let mut mesh = authored_mesh(&world, &[[0.1, 0.2]], &[], &[packed], |v| v * 2.0);
        retain_native_vegetation_channels(&mut mesh, &local, &[packed]).unwrap();
        let f3 = |attribute| match mesh.attribute(attribute).unwrap() {
            VertexAttributeValues::Float32x3(values) => values[0],
            _ => panic!("wrong vertex type"),
        };
        assert_eq!(f3(Mesh::ATTRIBUTE_POSITION), world[0].to_array());
        assert_eq!(f3(Mesh::ATTRIBUTE_NORMAL), [-2.0, 128.0 / 127.0, 2.0]);
        assert_eq!(
            f3(native_shadow_scene::NATIVE_VEGETATION_POSITION),
            local[0]
        );
        assert_eq!(
            f3(native_shadow_scene::NATIVE_VEGETATION_COLOR),
            [128.0 / 255.0, 64.0 / 255.0, 1.0]
        );
        let VertexAttributeValues::Float32x2(normal) = mesh
            .attribute(native_shadow_scene::NATIVE_VEGETATION_NORMAL)
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(normal[0], [-1.0, 64.0 / 127.0]);
        let VertexAttributeValues::Float32x2(uv) = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap()
        else {
            panic!()
        };
        assert_eq!(uv[0], [0.1, 0.2]);

        let short: [u8; 12] = packed[..12].try_into().unwrap();
        let mut no_color = authored_mesh(&world, &[[0.0; 2]], &[], &[short], |v| v);
        assert!(retain_native_vegetation_channels(&mut no_color, &local, &[short]).is_err());
        assert!(
            no_color
                .attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION)
                .is_none()
        );
        assert!(retain_native_vegetation_channels(&mut no_color, &[], &[packed]).is_err());
    }

    #[test]
    fn native_vegetation_mesh_channels_are_selective_and_shared_once() {
        use bevy::mesh::VertexAttributeValues;
        let packed = [[0u8; 16]];
        let local = [[2.0, 3.0, 4.0]];
        let mut meshes = Assets::<Mesh>::default();
        let handle = meshes.add(source_mesh(&local, &[[0.0; 2]], &[], &packed, false));
        let mut inputs = NativeVegetationMeshInputs::default();
        inputs.prepare(false, &handle, &mut meshes, &local, &packed);
        assert!(inputs.0.is_empty());
        assert!(
            meshes
                .get(&handle)
                .unwrap()
                .attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION)
                .is_none()
        );
        inputs.prepare(true, &handle, &mut meshes, &local, &packed);
        assert!(inputs.0[&handle.id()].is_ok());
        let raw_pointer = |mesh: &Mesh| match mesh
            .attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION)
            .unwrap()
        {
            VertexAttributeValues::Float32x3(values) => values.as_ptr(),
            _ => panic!("wrong vertex type"),
        };
        let first = raw_pointer(meshes.get(&handle).unwrap());
        inputs.prepare(true, &handle, &mut meshes, &local, &packed);
        assert_eq!(inputs.0.len(), 1);
        assert_eq!(raw_pointer(meshes.get(&handle).unwrap()), first);
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
        let overlay = world
            .spawn((Text::new("FPS: measuring"), FrameRateOverlay))
            .id();
        world.run_system_once(show_frame_rate).unwrap();
        assert_eq!(
            world.get::<Window>(window).unwrap().title,
            "Dishonored 2 Rust | 1 FPS | 0 loaded"
        );
        assert_eq!(world.get::<Text>(overlay).unwrap().0, "1 FPS | 1500.0 ms");
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
        assert!((spot - Vec3::new(0.0, expected, expected)).length() < 0.0001);
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
    #[ignore = "requires installed catalog and EOTW BWM; verifies the real world bank2, not rigid bank0"]
    fn native_world_shadow_sources_resolve_level_materials() {
        let base = Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base");
        let catalog = archive::Catalog::open(base).unwrap();
        let world = world_mesh::parse(&fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap()).unwrap();
        let mut world_mesh_assets = Assets::<Mesh>::default();
        let native_mesh_sources =
            NativeWorldShadowMeshSources::prepare(&world, &mut world_mesh_assets);
        assert_eq!(native_mesh_sources.objects.len(), world.objects.len());
        let mut checked_meshes = HashSet::new();
        for (object_id, object) in world.objects.iter().enumerate() {
            let source = native_mesh_sources.object(object_id).unwrap();
            assert_eq!(source.mesh_index, object.mesh_index);
            assert_eq!(source.matrix_start, object.matrix_start);
            assert_eq!(source.matrix_end, object.matrix_end);
            assert_eq!(source.lod, object.lod);
            assert_eq!(source.flipped, object.flipped);
            let decoded = &world.meshes[object.mesh_index];
            assert_eq!(
                source.raw_vertex_count.as_ref().unwrap(),
                &decoded.positions.len()
            );
            if !checked_meshes.insert((object.mesh_index, object.flipped)) {
                continue;
            }
            let mesh = world_mesh_assets.get(&source.mesh).unwrap();
            let bevy::mesh::VertexAttributeValues::Float32x3(forward) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap()
            else {
                panic!("forward BWM position attribute is missing")
            };
            let bevy::mesh::VertexAttributeValues::Float32x3(raw) = mesh
                .attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION)
                .unwrap()
            else {
                panic!("native local BWM position attribute is missing")
            };
            assert_eq!(raw, &decoded.positions);
            assert_eq!(forward.len(), decoded.positions.len());
            for (&local, converted) in decoded.positions.iter().zip(forward) {
                assert_eq!(*converted, game_to_view(local).to_array());
            }
            let bevy::mesh::VertexAttributeValues::Float32x2(raw_normal) = mesh
                .attribute(native_shadow_scene::NATIVE_VEGETATION_NORMAL)
                .unwrap()
            else {
                panic!("native local BWM normal attribute is missing")
            };
            assert_eq!(raw_normal.len(), decoded.packed_channels.len());
            for (actual, packed) in raw_normal.iter().zip(&decoded.packed_channels) {
                assert_eq!(
                    *actual,
                    [packed[4], packed[5]].map(|byte| ((byte as i8 as f32) / 127.0).max(-1.0))
                );
            }
            let bevy::mesh::VertexAttributeValues::Float32x3(raw_color) = mesh
                .attribute(native_shadow_scene::NATIVE_VEGETATION_COLOR)
                .unwrap()
            else {
                panic!("native local BWM color attribute is missing")
            };
            assert_eq!(raw_color.len(), decoded.packed_channels.len());
            for (actual, packed) in raw_color.iter().zip(&decoded.packed_channels) {
                assert_eq!(
                    *actual,
                    [packed[12], packed[13], packed[14]].map(|byte| byte as f32 / 255.0)
                );
            }
        }
        let mut library = shader_cache::NativeProgramLibrary::open(
            base,
            shader_cache::ShaderCacheBackend::Pc,
            &catalog,
        );
        let names: std::collections::BTreeSet<_> = world
            .objects
            .iter()
            .map(|object| &object.material_path)
            .collect();
        let mut counts = [0usize; 5];
        let mut base_slots = [0usize; 2];
        let mut failures = Vec::new();
        for name in names {
            let result: Result<Option<shader_cache::CachedWorldShadowClass>, String> = (|| {
                let bytes = catalog
                    .read(&format!("generated/decls/material/{name}.decl"))
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("missing {name}"))?;
                let surface = material_surface::Surface::parse(&bytes);
                let main = surface.main_program.as_deref().ok_or("missing MAIN")?;
                let program = catalog
                    .read(&format!(
                        "generated/decls/renderprog/{}.decl",
                        main.to_ascii_lowercase()
                    ))
                    .map_err(|e| e.to_string())?
                    .ok_or("missing MAIN definition")?;
                let inputs = resolve_native_shadow_source_inputs(
                    Some(&mut library),
                    &bytes,
                    &program,
                    Some(main),
                )?;
                let present = resolve_native_shadow_slot_presence(
                    Some(&mut library),
                    &bytes,
                    &inputs,
                    Some(main),
                )?;
                base_slots[usize::from(present)] += 1;
                let source = resolve_native_world_shadow_source(
                    Some(&mut library),
                    &bytes,
                    &inputs,
                    Some(main),
                )?;
                if let Some(source) = source {
                    let class = source.class.material_parameter_class();
                    resolve_native_shadow_material_parameters(
                        library.as_mut().map_err(|e| e.clone())?,
                        &catalog,
                        &bytes,
                        class,
                        false,
                    )?;
                    Ok(Some(source.class))
                } else {
                    Ok(None)
                }
            })(
            );
            match result {
                Ok(None) => counts[0] += 1,
                Ok(Some(shader_cache::CachedWorldShadowClass::Opaque)) => counts[1] += 1,
                Ok(Some(
                    shader_cache::CachedWorldShadowClass::AlphaTested
                    | shader_cache::CachedWorldShadowClass::VistaAlpha,
                )) => counts[2] += 1,
                Ok(Some(
                    shader_cache::CachedWorldShadowClass::Vegetation
                    | shader_cache::CachedWorldShadowClass::VegetationFixed,
                )) => counts[3] += 1,
                Err(error) => {
                    counts[4] += 1;
                    failures.push(format!("{name}: {error}"));
                }
            }
        }
        eprintln!(
            "actual world shadow bank2 disabled/opaque/alpha/vegetation/unsupported={counts:?}; base slot10 absent/present={base_slots:?}"
        );
        for failure in &failures {
            eprintln!("WORLD SOURCE UNSUPPORTED {failure}");
        }
        assert!(
            failures.is_empty(),
            "{} unresolved actual world shadow materials",
            failures.len()
        );
    }

    #[test]
    #[ignore = "requires installed catalog and actual EOTW BWM material fixture"]
    fn native_shadow_sources_resolve_actual_world_materials() {
        let catalog = archive::Catalog::open(Path::new(
            "D:/Programs/Steam/steamapps/common/Dishonored2/base",
        ))
        .unwrap();
        let world = world_mesh::parse(&fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap()).unwrap();
        let mut library = shader_cache::NativeProgramLibrary::open(
            Path::new("D:/Programs/Steam/steamapps/common/Dishonored2/base"),
            shader_cache::ShaderCacheBackend::Pc,
            &catalog,
        );
        let names: std::collections::BTreeSet<_> = world
            .objects
            .iter()
            .map(|object| &object.material_path)
            .collect();
        let mut counts = [0usize; 5]; // disabled, opaque, cutout, vegetation, unsupported
        let mut vegetation_materials = std::collections::BTreeSet::new();
        let mut failures = Vec::new();
        // Explicit synthetic model inputs validate the real material encoder;
        // they are not a claim that every actual entity owns group 3/offset 0.
        let mut clock = game_clock::NativeGameClock::from_bytes([0; game_clock::STATE_SIZE]);
        clock.initialize();
        let clocks = game_clock::GameplayViewClocks::from_clock(&clock);
        let mvp = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ];
        for name in names {
            let result = (|| {
                let bytes = catalog
                    .read(&format!("generated/decls/material/{name}.decl"))
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| format!("missing material {name}"))?;
                let surface = material_surface::Surface::parse(&bytes);
                let program = match surface.main_program.as_deref() {
                    Some(name) => catalog
                        .read(&format!(
                            "generated/decls/renderprog/{}.decl",
                            name.to_ascii_lowercase()
                        ))
                        .map_err(|error| error.to_string())?
                        .ok_or_else(|| format!("missing main program {name}"))?,
                    None => return Err("material has no main program".into()),
                };
                let source = resolve_native_shadow_source_inputs(
                    Some(&mut library),
                    &bytes,
                    &program,
                    surface.main_program.as_deref(),
                )?;
                // Read-only native material snapshot, not a path exception:
                // final class 2 / hairType 0 clears all three rigid banks.
                if name == "models/environment/props/generic/clocks_01/clocks_glass.material" {
                    assert_eq!(source.source, material::ShadowProgramSource::Disabled);
                }
                let resolved = resolve_native_rigid_shadow_source(
                    Some(&mut library),
                    &bytes,
                    &source,
                    surface.main_program.as_deref(),
                );
                if let Err(error) = &resolved {
                    let library = library.as_mut().map_err(|error| error.to_string())?;
                    let main = surface.main_program.as_deref().unwrap();
                    let selected = source.override_program.as_deref().unwrap_or(main);
                    let parameters = native_shadow_variant_overrides(
                        library,
                        &bytes,
                        &source,
                        Some(main),
                        selected,
                    )?;
                    let values: Vec<_> = [
                        "noShadow",
                        "invisible",
                        "hairType",
                        "skyType",
                        "useDirectInstancing",
                        "useMultiInstancing",
                        "noScreenShadows",
                        "translucency_hq/enable",
                        "translucency/enable",
                    ]
                    .into_iter()
                    .map(|name| {
                        (
                            name,
                            library.option_value(main.as_bytes(), &parameters, name.as_bytes()),
                        )
                    })
                    .collect();
                    let requested_hash = library
                        .specialization_hash(selected.as_bytes(), &parameters)?
                        .map(|hash| format!("{hash:08x}"));
                    let stage_ids = |data: render_program::D3dVariantData| {
                        data.stages
                            .into_iter()
                            .map(|stage| {
                                (
                                    stage.kind,
                                    match stage.bytecode {
                                        render_program::StageBytecode::Absent => None,
                                        render_program::StageBytecode::Cached(id) => Some(id),
                                        render_program::StageBytecode::Inline(_) => Some(u32::MAX),
                                    },
                                )
                            })
                            .collect::<Vec<_>>()
                    };
                    let base = library
                        .serialized_variant(selected.as_bytes(), &parameters)?
                        .map(stage_ids);
                    let mut banks = Vec::new();
                    for (enabled, excluded) in [
                        ("useDirectInstancing", "useMultiInstancing"),
                        ("useMultiInstancing", "useDirectInstancing"),
                    ] {
                        if library.option_value(
                            main.as_bytes(),
                            &parameters,
                            excluded.as_bytes(),
                        )? != 0
                        {
                            continue;
                        }
                        let mut instanced = parameters.clone();
                        instanced
                            .parameters
                            .push(render_program::SerializedParameter {
                                name: enabled.as_bytes().to_vec(),
                                kind: render_program::RenderParameterKind::Option,
                                value: render_program::SerializedParameterValue::Words([
                                    1, 0, 0, 0,
                                ]),
                            });
                        let stages = library
                            .serialized_variant(selected.as_bytes(), &instanced)?
                            .map(stage_ids);
                        let hash = library
                            .specialization_hash(selected.as_bytes(), &instanced)?
                            .map(|hash| format!("{hash:08x}"));
                        banks.push((enabled, hash, stages));
                    }
                    let main_state = library
                        .definition(main.as_bytes())?
                        .unwrap()
                        .native_render_state();
                    return Err(format!(
                        "{error}; requested native signature {requested_hash:?}; base {base:?}; blend parsed={:08x} mainRpd={:08x}; effective main options {values:?}; native instance banks {banks:?}",
                        source.main_blend_state, main_state[0]
                    ));
                }
                resolved
            })();
            match result {
                Ok(None) => counts[0] += 1,
                Ok(Some(source)) => {
                    let material = catalog
                        .read(&format!("generated/decls/material/{name}.decl"))
                        .unwrap()
                        .unwrap();
                    let library = library.as_mut().unwrap();
                    let parameters = resolve_native_shadow_material_parameters(
                        library,
                        &catalog,
                        &material,
                        source.class,
                        true,
                    )
                    .unwrap_or_else(|error| {
                        panic!("{name}: native shadow material parameters: {error}")
                    });
                    for (parameter, default) in &parameters.defaults {
                        let expected = library
                            .registered_float_vector(
                                &catalog,
                                &parameters.overrides,
                                parameter,
                                None,
                            )
                            .unwrap();
                        assert_eq!(
                            parameters
                                .vector(parameter, None)
                                .unwrap()
                                .map(f32::to_bits),
                            expected
                        );
                        let fallback = render_program::SerializedParameterBlock {
                            flags: 0,
                            parameters: vec![render_program::SerializedParameter {
                                name: parameter.clone(),
                                kind: render_program::RenderParameterKind::Uniform,
                                value: render_program::SerializedParameterValue::Words(
                                    [11.0f32.to_bits(); 4],
                                ),
                            }],
                        };
                        let expected = parameters
                            .overrides
                            .vector_words(parameter, 0x30, *default, Some(&fallback))
                            .unwrap();
                        assert_eq!(
                            parameters
                                .vector(parameter, Some(&fallback))
                                .unwrap()
                                .map(f32::to_bits),
                            expected
                        );
                    }
                    if source.class == shader_cache::CachedRigidShadowClass::Vegetation {
                        let wind = parameters.vegetation(None).unwrap();
                        let uv = parameters.vector(b"layer0_scalebias", None).unwrap();
                        for group in 0..game_clock::GROUP_COUNT {
                            let flags = (group as u8) << 4;
                            let actual = parameters
                                .vegetation_instance_cb(None, &clocks, 0.5, flags, &mvp, 1.0)
                                .unwrap();
                            let local_time = render_model::model_shader_local_time(
                                0.5,
                                clock.time(group).unwrap(),
                                clock.delta(group).unwrap(),
                            );
                            let expected =
                                native_shadow_scene::native_vegetation_shadow_instance_cb(
                                    uv, &wind, &mvp, local_time, 1.0,
                                );
                            assert_eq!(actual, expected, "{name}: model clock group {group}");
                        }
                        assert!(
                            parameters
                                .vegetation_instance_cb(None, &clocks, 0.5, 0x50, &mvp, 1.0)
                                .is_err()
                        );
                    }
                    match source.class {
                        shader_cache::CachedRigidShadowClass::Opaque => counts[1] += 1,
                        shader_cache::CachedRigidShadowClass::AlphaTested | shader_cache::CachedRigidShadowClass::VistaAlpha => counts[2] += 1,
                        shader_cache::CachedRigidShadowClass::Vegetation => {
                            counts[3] += 1;
                            if vegetation_materials.is_empty() {
                                let definition = library
                                    .definition(&source.program)
                                    .unwrap()
                                    .unwrap()
                                    .clone();
                                let material = catalog
                                    .read(&format!("generated/decls/material/{name}.decl"))
                                    .unwrap()
                                    .unwrap();
                                let mut evidence = format!(
                                    "material={name}\nprogram={}\ndefaults={:?}\nmaterialSource:\n{}\n",
                                    String::from_utf8_lossy(&source.program),
                                    definition.defaults,
                                    String::from_utf8_lossy(&material)
                                );
                                let mut parameters = std::collections::BTreeSet::new();
                                for stage in &source.variant.stages {
                                    let names: Vec<_> = stage
                                        .bindings
                                        .parameters
                                        .iter()
                                        .filter_map(|entry| {
                                            let name =
                                                entry.name.bytes(&library.cache.names).unwrap();
                                            let key: Vec<_> = name
                                                .split(|byte| *byte == 0)
                                                .next()
                                                .unwrap()
                                                .iter()
                                                .map(u8::to_ascii_lowercase)
                                                .collect();
                                            (library.parameter_kinds.get(&key)
                                                == Some(
                                                    &render_program::RenderParameterKind::Uniform,
                                                ))
                                            .then(|| String::from_utf8_lossy(&key).into_owned())
                                        })
                                        .collect();
                                    evidence.push_str(&format!(
                                        "stage={} layout={:?} uniforms={names:?}\n",
                                        stage.kind, stage.layout
                                    ));
                                    parameters.extend(names);
                                }
                                for parameter in parameters {
                                    let path = format!(
                                        "generated/decls/renderparm/{}.decl",
                                        parameter.to_ascii_lowercase()
                                    );
                                    let bytes = catalog.read(&path).unwrap().unwrap();
                                    evidence.push_str(&format!(
                                        "\n{parameter}:\n{}\n",
                                        String::from_utf8_lossy(&bytes)
                                    ));
                                }
                                fs::write(
                                    "artifacts/native-vegetation-uniform-evidence.txt",
                                    evidence,
                                )
                                .unwrap();
                            }
                            vegetation_materials.insert(name.to_string());
                        }
                    }
                }
                Err(error) => {
                    counts[4] += 1;
                    failures.push(format!("{name}: {error}"));
                }
            }
        }
        // Exercise the production mesh/input preparation against every actual
        // world mesh using a verified wind program, including shared LOD data.
        let mut geometry = Assets::<Mesh>::default();
        let mut handles = HashMap::new();
        let mut inputs = NativeVegetationMeshInputs::default();
        for object in &world.objects {
            if !vegetation_materials.contains(&object.material_path) {
                continue;
            }
            let source = &world.meshes[object.mesh_index];
            let handle = handles.entry(object.mesh_index).or_insert_with(|| {
                geometry.add(source_mesh(
                    &source.positions,
                    &source.uvs,
                    &source.indices,
                    &source.packed_channels,
                    false,
                ))
            });
            inputs.prepare(
                true,
                handle,
                &mut geometry,
                &source.positions,
                &source.packed_channels,
            );
            assert!(
                inputs.0[&handle.id()].is_ok(),
                "{}: {:?}",
                object.material_path,
                inputs.0[&handle.id()]
            );
            let mesh = geometry.get(&*handle).unwrap();
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(raw)) =
                mesh.attribute(native_shadow_scene::NATIVE_VEGETATION_POSITION)
            else {
                panic!("missing native local positions")
            };
            assert_eq!(raw, &source.positions);
        }
        assert!(vegetation_materials.is_empty() || !inputs.0.is_empty());
        let vertices: usize = inputs
            .0
            .values()
            .map(|result| *result.as_ref().unwrap())
            .sum();
        let geometry_report = format!(
            "native vegetation geometry: {} shared meshes, {vertices} vertices, {} decoded attribute bytes",
            inputs.0.len(),
            vertices * std::mem::size_of::<[f32; 8]>()
        );
        eprintln!("{geometry_report}");
        let report = format!(
            "actual BWM shadow material sources disabled/opaque/cutout/vegetation/unsupported={counts:?}\n{geometry_report}\n{}\n",
            failures.join("\n")
        );
        fs::write("artifacts/native-world-shadow-source-coverage.txt", &report).unwrap();
        eprintln!(
            "actual BWM shadow material sources disabled/opaque/cutout/vegetation/unsupported={counts:?}"
        );
        for error in failures.iter().take(25) {
            eprintln!("unsupported shadow source: {error}");
        }
        assert!(
            counts[1] + counts[2] > 0,
            "no actual map materials resolved; this is NOT a complete-coverage assertion"
        );
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
    fn area_rectangle_preserves_native_corners_and_emitting_side() {
        // Real balcony light plus identity: compare native +/- half-extent
        // corners with the backend rectangle, including its one-sided normal.
        for rotation in [
            [[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]],
            [[-0.55556643,0.8314722,0.],[-0.8314722,-0.55556643,0.],[0.,0.,1.]],
        ] {
            let center=Vec3::new(-41.088326,23.700006,-83.30427);
            let transform=authored_area_light_transform(center,rotation).unwrap();
            let z=game_to_view(rotation[0]).normalize();
            let x=game_to_view(rotation[1]);let y=z.cross(x);
            let half=[0.7587613,1.5461122];
            let native=[-1.,1.].into_iter().flat_map(|sx|[-1.,1.].map(|sy|center+sx*half[0]*x+sy*half[1]*y)).collect::<Vec<_>>();
            for sx in [-1.,1.] {for sy in [-1.,1.] {
                let corner=center+sx*half[0]*transform.right()+sy*half[1]*transform.up();
                assert!(native.iter().any(|p|p.distance(corner)<0.00001));
            }}
            assert!(transform.up().cross(*transform.right()).dot(z)>0.99999);
        }
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
                    let full_size=size.map(|half|2.0*half);
                    let render_entity = render_world.spawn_empty().id();
                    main_world.spawn((
                        RectLight {
                            color: Color::WHITE,
                            intensity: light_backend_intensity(
                                output.normalized_intensity,
                                entities::LightType::Area,
                                full_size,
                            )
                            .unwrap(),
                            width: full_size[0],
                            height: full_size[1],
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
            let backend=light_backend_intensity(5.0, kind, [1.0;2]).unwrap();
            let world=if kind==LightType::Parallel {backend}else{backend/(4.0*std::f32::consts::PI)};
            assert!((world-696.3).abs()<0.0001,"native preset intensity5 in world units: {world}");
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
            static_world_model_present: false,
            instances: Vec::new(),
            active: 0,
            tome_render_instances: TomeRenderInstanceMap::default(),
            visible_authored_ids: vec![7],
            visible_cell_indices: vec![0],
            visible_cell_mask: vec![1],
            native_shadow_diffuse_slots: Vec::new(),
            light_query_state: None,
            shadow_ordinary_query: None,
            visibility_suppression_groups: Vec::new(),
            trace_bwm_lod: false,
            world_lods_by_render_instance: Vec::new(),
            last_bwm_lod_visibility_report: None,
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
            trace_texture_streaming: false,
            texture_events_pending: VecDeque::new(),
            texture_event_history: VecDeque::new(),
            texture_events_dropped: 0,
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
        // Keep Local cache and change ticks across actual system invocations.
        let stream = world.register_system(stream_visible_models);
        world.run_system(stream).unwrap();
        assert_eq!(world.resource::<StreamingDiagnostics>().status, "ok");
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=false")
        );
        let initial_ids = world
            .resource::<SceneRenderQueue>()
            .visible_authored_ids
            .clone();
        let initial_cells = world
            .resource::<SceneRenderQueue>()
            .visible_cell_indices
            .clone();
        let initial_depth = world
            .resource::<SceneRenderQueue>()
            .light_query_state
            .as_ref()
            .unwrap()
            .depth_grid
            .clone();
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=true")
        );
        assert_eq!(
            world.resource::<SceneRenderQueue>().visible_authored_ids,
            initial_ids
        );
        assert_eq!(
            world.resource::<SceneRenderQueue>().visible_cell_indices,
            initial_cells
        );
        assert_eq!(
            world
                .resource::<SceneRenderQueue>()
                .light_query_state
                .as_ref()
                .unwrap()
                .depth_grid,
            initial_depth
        );

        // Replacing the resource must invalidate even identical query inputs.
        let unchanged_world = world.remove_resource::<TomeWorld>().unwrap();
        world.insert_resource(unchanged_world);
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=false")
        );
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=true")
        );

        let projection = world
            .entity_mut(camera_entity)
            .take::<Projection>()
            .unwrap();
        world.run_system(stream).unwrap();
        assert_eq!(
            world.resource::<StreamingDiagnostics>().status,
            "camera_not_ready"
        );
        assert!(
            world
                .resource::<SceneRenderQueue>()
                .visible_authored_ids
                .is_empty()
        );
        assert!(
            world
                .resource::<SceneRenderQueue>()
                .light_query_state
                .is_none()
        );
        world.entity_mut(camera_entity).insert(projection);
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=true")
        );
        assert_eq!(
            world.resource::<SceneRenderQueue>().visible_authored_ids,
            initial_ids
        );

        let viewport = world
            .get_mut::<Camera>(camera_entity)
            .unwrap()
            .viewport
            .take();
        world.run_system(stream).unwrap();
        assert_eq!(
            world.resource::<StreamingDiagnostics>().status,
            "camera_not_ready"
        );
        assert!(
            world
                .resource::<SceneRenderQueue>()
                .light_query_state
                .is_none()
        );
        world.get_mut::<Camera>(camera_entity).unwrap().viewport = viewport.clone();
        world
            .get_mut::<Camera>(camera_entity)
            .unwrap()
            .viewport
            .as_mut()
            .unwrap()
            .physical_size = UVec2::ZERO;
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .status
                .starts_with("query_error:")
        );
        assert!(
            world
                .resource::<SceneRenderQueue>()
                .light_query_state
                .is_none()
        );
        world.get_mut::<Camera>(camera_entity).unwrap().viewport = viewport;
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=true")
        );

        let other_camera = world.spawn(Camera3d::default()).id();
        world.run_system(stream).unwrap();
        assert_eq!(
            world.resource::<StreamingDiagnostics>().status,
            "camera_unavailable"
        );
        assert!(
            world
                .resource::<SceneRenderQueue>()
                .visible_authored_ids
                .is_empty()
        );
        assert!(
            world
                .resource::<SceneRenderQueue>()
                .light_query_state
                .is_none()
        );
        world.despawn(other_camera);
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=true")
        );
        assert_eq!(
            world.resource::<SceneRenderQueue>().visible_authored_ids,
            initial_ids
        );
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
                object_distance_scale: 1.0,
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
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=false")
        );
        world.run_system(stream).unwrap();
        assert!(
            world
                .resource::<StreamingDiagnostics>()
                .pending
                .contains("query_cached=false")
        );
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
        world.run_system(stream).unwrap();
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
        let finer = read_streamed_diffuse_at_target(
            &catalog,
            stem,
            TextureReadTarget::Exact(2, texture::CompressedFormat::Bc7Srgb),
        )
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
                vertex_probes: Handle::default(),
                native_shadow: Handle::default(),
                native_local_shadow: Handle::default(),
                native_local_shadow_map: None,
                native_shadow_map: None,
                native_screen_map: None,
                    native_cloud_map: None,
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
                    specular0:None,specular1:None,specular2:None,
            },
        });
        app.insert_resource(materials);
        app.insert_resource(Assets::<DecalMaterial>::default());
        app.insert_resource(TextureReadWorker::new(catalog, 1));
        app.insert_resource(SceneRenderQueue {
            static_world_model_present: false,
            instances: vec![RenderInstance {
                source: None,
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
            trace_bwm_lod: false,
            world_lods_by_render_instance: Vec::new(),
            last_bwm_lod_visibility_report: None,
            diffuse_image_demands: vec![texture::ProjectedAreaDemand::default()],
            diffuse_image_paths: vec![stem],
            light_query_state: None,
            shadow_ordinary_query: None,
            diffuse_image_materials: vec![vec![(
                MaterialHandle::Probe(material.clone()),
                surface_gpu::ImageRole(0),
            )]],
            native_material_images: HashMap::new(),
            native_shadow_diffuse_slots: Vec::new(),
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
            static_world_model_present: false,
            light_query_state: None,
            shadow_ordinary_query: None,
            native_shadow_diffuse_slots: Vec::new(),
            instances: Vec::new(),
            active: 0,
            tome_render_instances: TomeRenderInstanceMap::default(),
            visible_authored_ids: Vec::new(),
            visible_cell_indices: Vec::new(),
            visible_cell_mask: Vec::new(),
            visibility_suppression_groups: Vec::new(),
            trace_bwm_lod: false,
            world_lods_by_render_instance: Vec::new(),
            last_bwm_lod_visibility_report: None,
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
                object_distance_scale: 1.0,
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
            let diagnostics = parallel
                .into_iter()
                .map(|job| job.diagnostics)
                .collect::<Vec<_>>();
            let mut cache = TomeRenderQueryCache::default();
            cache.store(
                TomeRenderQueryKey::new(&input, count),
                &Ok(Some(merged.clone())),
                &diagnostics,
            );
            assert!(cache.matches(&TomeRenderQueryKey::new(&input, count), false));
            let cached = cache.output.as_ref().unwrap();
            assert_eq!(cached.indices(), merged.indices());
            assert_eq!(
                cached
                    .area_metrics()
                    .iter()
                    .map(|value| value.map(f32::to_bits))
                    .collect::<Vec<_>>(),
                merged
                    .area_metrics()
                    .iter()
                    .map(|value| value.map(f32::to_bits))
                    .collect::<Vec<_>>()
            );
            assert_eq!(cached.cell_indices(), merged.cell_indices());
            assert_eq!(cached.state, merged.state);
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
            object_distance_scale: 1.0,
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
                    object_distance_scale: 1.0,
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
                // Installed FUN_140a82710 Tome-query captures include 22108
                // and 17564 but reject 22109 for this camera. This is a
                // visibility-set fixture, not evidence of an LOD-tier rule.
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
    #[ignore = "requires local original-game EOTW BWM/Tome fixtures and installed executable"]
    fn dense_depth_grid_matches_original_bounds_after_camera_shifts() {
        bevy::tasks::ComputeTaskPool::get_or_init(bevy::tasks::TaskPool::new);
        let bytes = fs::read("artifacts/eotw_p_lowchaos.tome").unwrap();
        let tree = tome::SpatialTree::parse(&bytes).unwrap();
        let query_data = tome::DirectTomeQueryData::parse(&bytes, &tree).unwrap();
        let world = TomeWorld {
            sphere_graph: tome::SphereCellGraph::parse(&bytes),
            gate_states: tome::initial_gate_states(&bytes).unwrap(),
            bytes,
            tree,
            query_data,
            query_job_count: tome::ORIGINAL_DEFAULT_QUERY_JOB_COUNT,
        };
        let pose = [
            [-0.72593063, 0.01375436, -0.6876305, -36.467716],
            [-0.68776804, -0.014517557, 0.7257855, 192.6837],
            [4.656613e-10, 0.9998, 0.019998547, 5.6344576],
            [0.0, 0.0, 0.0, 1.0],
        ];
        let base = Mat4::from_cols_array_2d(&std::array::from_fn(|column| {
            std::array::from_fn(|row| pose[row][column])
        }));
        let bwm = world_mesh::parse(&fs::read("artifacts/eotw_p_lowchaos.bwm").unwrap()).unwrap();
        let mut bwm_instances = vec![Vec::new(); bwm.visibility.len()];
        let mut triangles = Vec::new();
        for object in &bwm.objects {
            for instances in &mut bwm_instances[object.matrix_start..object.matrix_end] {
                instances.push(triangles.len());
                triangles.push(bwm.meshes[object.mesh_index].indices.len() / 3);
            }
        }
        let mapping = TomeRenderInstanceMap::from_world_mapping(&bwm, &bwm_instances);
        let groups: Vec<_> = bwm
            .visibility_groups
            .iter()
            .map(|group| RenderSuppressionGroup {
                parent: bwm_instances[group.parent].clone(),
                children: group
                    .children
                    .iter()
                    .flat_map(|&index| bwm_instances[index].iter().copied())
                    .collect(),
            })
            .collect();
        for yaw in [-0.0005, 0.0, 0.0005] {
            let columns = (base * Mat4::from_rotation_y(yaw)).to_cols_array_2d();
            let pose =
                std::array::from_fn(|row| std::array::from_fn(|column| columns[column][row]));
            let rows = tome::original_world_visibility_query_matrix(
                80.0, 1920, 1017, 1.0, pose, 0.01, 7000.0,
            )
            .unwrap();
            let input = tome::OrdinaryDirectQueryInput {
                camera_point: [pose[0][3], pose[1][3], pose[2][3]],
                render_rows: rows,
                cropped_rows: rows,
                initial_tile_rect: [0, 0, 8, 16],
                camera_imprecision: [0.6; 3],
                object_imprecision: 0.6,
                minimum_area_metric: 25.0 / (1920.0 * 1017.0),
                global_distance_threshold: tome::ORIGINAL_DEFAULT_GLOBAL_DISTANCE_THRESHOLD,
                object_distance_scale: 1.0,
                winding_flip: tome::query_matrix_winding_flip(rows),
                gate_states: Some(&world.gate_states),
                output_capacity: tome::ORIGINAL_WORLD_QUERY_OUTPUT_CAPACITY,
            };
            let started = std::time::Instant::now();
            let jobs = run_tome_render_jobs(&world, &input);
            let output = tome::merge_render_query_outputs(&jobs).unwrap().unwrap();
            let query_ms = started.elapsed().as_secs_f64() * 1000.0;
            let state = output.state.as_ref().unwrap();
            let query = tome::RenderAabbQuery::from_state(state).unwrap();
            let mut old_state = state.clone();
            old_state.depth_grid = None;
            let old_query = tome::RenderAabbQuery::from_state(&old_state).unwrap();
            let mut workloads = Vec::new();
            for candidate in [&old_state, state] {
                let mut visible = vec![false; triangles.len()];
                mapping
                    .mark_visible_instances(output.indices(), &mut visible)
                    .unwrap();
                let accepted = mapping
                    .apply_spatial_queries(Some(candidate), &mut visible)
                    .unwrap();
                RenderSuppressionGroup::apply_all(&groups, &mut visible);
                let surfaces = visible.iter().filter(|&&value| value).count();
                let geometry: usize = visible
                    .iter()
                    .zip(&triangles)
                    .filter_map(|(&value, &count)| value.then_some(count))
                    .sum();
                workloads.push((accepted, surfaces, geometry));
            }
            println!(
                "dense yaw={yaw} query_ms={query_ms:.3} spatial/mapped_surfaces/mapped_triangles before={:?} after={:?}",
                workloads[0], workloads[1]
            );
            let mut old_count = 0;
            let mut depth_count = 0;
            // World object output and later spatial AABB output are different
            // native lists. Original764c0 also rejects world object7268 here;
            // compare every bound with native rather than forcing a superset.
            let mut raw = b"DH2DQ001".to_vec();
            for value in state.viewport {
                raw.extend_from_slice(&value.to_le_bytes());
            }
            let unused_bounds = [0.0_f32; 6];
            for value in state
                .render_rows
                .iter()
                .flatten()
                .chain(state.cropped_rows.iter().flatten())
                .chain(state.camera_imprecision.iter())
                .chain(std::iter::once(&state.maximum_depth))
                .chain(unused_bounds.iter())
                .chain(state.depth_grid.as_ref().unwrap().iter())
            {
                raw.extend_from_slice(&value.to_le_bytes());
            }
            raw.extend_from_slice(&123.0_f32.to_le_bytes());
            raw.extend_from_slice(&(world.query_data.object_count() as u32).to_le_bytes());
            for index in 0..world.query_data.object_count() as u32 {
                let bounds = tome::object_bounds(&world.bytes, index).unwrap();
                let old = old_query.visible(bounds).unwrap();
                let (depth, area) = query.visibility_and_area(bounds).unwrap();
                assert!(
                    !depth || old,
                    "depth admitted outside-frustum object{index} yaw={yaw}"
                );
                old_count += usize::from(old);
                depth_count += usize::from(depth);
                raw.extend_from_slice(&index.to_le_bytes());
                for value in bounds.iter().flatten() {
                    raw.extend_from_slice(&value.to_le_bytes());
                }
                raw.extend_from_slice(&u32::from(depth).to_le_bytes());
                raw.extend_from_slice(&area.to_le_bytes());
            }
            let fixture = "artifacts/dense-depth-query-state.bin";
            fs::write(fixture, raw).unwrap();
            let native = std::process::Command::new("python")
                .args(["analysis/probe_original_depth_aabb.py", "--state", fixture])
                .output()
                .unwrap();
            assert!(
                native.status.success(),
                "native AABB parity yaw={yaw}: {}{}",
                String::from_utf8_lossy(&native.stdout),
                String::from_utf8_lossy(&native.stderr)
            );
            print!("{}", String::from_utf8_lossy(&native.stdout));
            assert!(
                depth_count < old_count,
                "dense camera did not reject any extra bounds yaw={yaw}"
            );
            println!(
                "dense yaw={yaw} frustum_bounds={old_count} depth_bounds={depth_count} accepted_objects={} grid_max={}",
                output.indices().len(),
                state.maximum_depth
            );
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
            static_world_model_present: false,
            instances: vec![RenderInstance {
                source: None,
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
            trace_bwm_lod: false,
            world_lods_by_render_instance: Vec::new(),
            last_bwm_lod_visibility_report: None,
            diffuse_image_demands: Vec::new(),
            diffuse_image_paths: Vec::new(),
            diffuse_image_materials: Vec::new(),
            native_material_images: HashMap::new(),
            native_shadow_diffuse_slots: Vec::new(),
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
            shadow_ordinary_query: None,
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
        let catalog = archive::Catalog::open(std::path::Path::new(
            "D:/Programs/Steam/steamapps/common/Dishonored2/base",
        ))
        .unwrap();
        let cases = [
            (
                "models/environment/vs_map/nature/roots_01/roots_lod2_green_d$nomips",
                "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_green_d$nomips.tga.bimage7",
            ),
            (
                "models/environment/vs_map/nature/roots_01/roots_lod2_n.tga$nomips",
                "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_n.tga$nomips.bimage7",
            ),
            (
                "models/environment/vs_map/nature/roots_01/roots_lod2_g.tga$nomips",
                "generated/image/models/environment/vs_map/nature/roots_01/roots_lod2_g.tga$nomips.bimage7",
            ),
            (
                "ipr_constantcolor(0.0745098, 0.0980392, 0.12549, 1)",
                "generated/image/ipr_constantcolor__0.0745098__0.0980392__0.12549__1_.bimage7",
            ),
            (
                "ipr_constantcolor(0.501961, 0.501961, 0, 1)",
                "generated/image/ipr_constantcolor__0.501961__0.501961__0__1_.bimage7",
            ),
            (
                "ipr_constantcolor(0, 0, 0, 1)",
                "generated/image/ipr_constantcolor__0__0__0__1_.bimage7",
            ),
            (
                "ipr_constantcolor(0.6, 0.6, 0.6, 1)",
                "generated/image/ipr_constantcolor__0.6__0.6__0.6__1_.bimage7",
            ),
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
            source: None,
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
            source: None,
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
                flipped: false,
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
                object_distance_scale: 1.0,
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
        assert!(render_model::model_eligible(
            default_input.frame_flags,
            u8::MAX
        ));
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
            position: [0.0; 3],
            scale: [1.5, -0.015, 1.0],
            rotation: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            custom_materials: Vec::new(),
            max_visible_range: None,
        };
        assert_eq!(
            model_havok_path(&placement, "hkt", false).as_deref(),
            Some("generated/models/fence.blueprint.hkt._150_-1_100.10.hkx")
        );
        assert_eq!(
            model_havok_path(&placement, "box", false).as_deref(),
            Some("generated/models/fence.blueprint.box._150_-1_100.10.hkx")
        );
        assert_eq!(
            model_havok_path(&placement, "cvd", false).as_deref(),
            Some("generated/models/fence.blueprint.cvd._150_-1_100.10.hkx")
        );
        assert_eq!(
            model_havok_path(&placement, "hkt", true).as_deref(),
            Some("generated/models/fence_hd.blueprint.hkt._150_-1_100.10.hkx")
        );
        assert_eq!(model_havok_path(&placement, "unknown", false), None);
    }

    #[test]
    #[ignore = "requires installed catalog and artifacts/eotw_p_lowchaos.entities"]
    fn blueprint_root_collision_uses_component_not_render_children() {
        let document =
            entities::parse(&fs::read_to_string("artifacts/eotw_p_lowchaos.entities").unwrap())
                .unwrap();
        let bodies = entities::rigid_body_settings(&document);
        let parent = entities::render_placements(&document)
            .into_iter()
            .find(|placement| placement.id == "env_bp_fence_metal_200cm_02")
            .unwrap();
        let catalog = archive::Catalog::open(&PathBuf::from(
            r"D:\Programs\Steam\steamapps\common\Dishonored2\base",
        ))
        .unwrap();
        assert_eq!(
            model_havok_path(&parent, "hkt", false).as_deref(),
            Some(
                "generated/models/environment/props/karnaca/fence_metal_set_01/bp_fence_metal_200cm_02.blueprint.hkt._100_100_100.10.hkx"
            )
        );
        let placements = expanded_model_placements(&[parent], &catalog);
        assert_eq!(placements.len(), 10);
        let (triangles, owners, matched, failed) =
            static_model_collision(&catalog, &placements, &bodies);
        assert_eq!((matched, failed, triangles.len()), (1, 0, 312));
        assert_eq!(owners, ["env_bp_fence_metal_200cm_02"]);
        assert!(
            triangles
                .iter()
                .all(|(vertices, owner)| *owner == 0
                    && vertices.iter().all(|point| point.is_finite()))
        );
        assert_eq!(
            static_model_collision(&catalog, &placements, &[]).0.len(),
            0
        );
        let mut unbound_defaults = bodies.clone();
        for body in &mut unbound_defaults {
            body.physics_behavior = None;
        }
        assert_eq!(
            static_model_collision(&catalog, &placements, &unbound_defaults)
                .0
                .len(),
            0
        );
        let world = navigation::CollisionWorld::with_owned(triangles, owners);
        let eye = game_to_view([-33.679825, 83.83279, 18.319]);
        let direction = game_to_view([0.5484812, 0.29665253, -0.78177094]).normalize();
        let (distance, owner, _) = world
            .first_hit_details(eye, eye + direction * 40.0)
            .unwrap();
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
