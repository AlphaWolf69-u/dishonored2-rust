use std::io::Write;
// State.
use super::{
    archive, entities, env, fs, game_settings, navigation, probe_lighting, runtime, shadow_pass,
    tome, world_mesh, Aabb, DecalExtension, ExtendedMaterial, Instant, PathBuf, ProbeExtension,
    Shader, SystemTime, VecDeque, UNIX_EPOCH,
};
use bevy::prelude::*;

#[derive(Resource)]
pub(super) struct SceneData {
    pub(super) source_path: PathBuf,
    pub(super) world: world_mesh::WorldMesh,
    pub(super) player_start: Option<entities::PlayerStart>,
    pub(super) placed_models: Vec<entities::PlacedModel>,
    pub(super) placed_lights: Vec<entities::PlacedLight>,
    pub(super) environment_lighting: Option<entities::EnvironmentLighting>,
    #[allow(dead_code)] // Preserved for the not-yet-ported fog, water, sky, and postfx passes.
    pub(super) environment_render_parameters: Option<entities::EnvironmentRenderParameters>,
    pub(super) environment_probes: Option<probe_lighting::ProbeLightingSet>,
    #[allow(dead_code)]
    // Actual startup inputs for the native CPU frame publisher, not captured defaults.
    pub(super) shadow_quality_preset: Option<u32>,
    #[allow(dead_code)]
    pub(super) shadow_quality: Option<entities::ShadowQualitySettings>,
    #[allow(dead_code)] // Actual startup target specification; not a fabricated Ready frame.
    pub(super) shadow_target_dimensions: Option<shadow_pass::NativeParallelShadowTargetDimensions>,
    #[allow(dead_code)] // Original r_depthBiasClamp CVar used by native hardware shadow bias.
    pub(super) shadow_depth_bias_clamp: Option<f32>,
    pub(super) shadow_sort_ccw: Option<bool>,
    pub(super) shadow_cascade_controls: Option<game_settings::ShadowCascadeControls>,
    #[allow(dead_code)] // Parsed runtime CVar; used by the pending live cascade publisher.
    pub(super) shadow_cascade_fars: Option<[f32; 4]>,
    #[allow(dead_code)] // Authored whole-view receiver bounds, not visible-mesh bounds.
    pub(super) far_shadow_bounds: Option<[[f32; 3]; 2]>,
    pub(super) pickups: Vec<entities::Pickup>,
    pub(super) catalog: Option<archive::Catalog>,
}

#[derive(Component)]
pub(super) struct PickupVisual(pub(super) String);

pub(super) struct PickupInstance {
    pub(super) entity_id: String,
    pub(super) position: Vec3,
}

#[derive(Resource)]
pub(super) struct PickupState {
    pub(super) items: Vec<PickupInstance>,
    pub(super) world: runtime::WorldState,
}

#[derive(Resource)]
pub(super) struct NavigationWorld(pub(super) navigation::CollisionWorld);

#[derive(Resource)]
pub(super) struct TomeWorld {
    pub(super) bytes: Vec<u8>,
    pub(super) tree: tome::SpatialTree,
    pub(super) gate_states: Vec<u32>,
    pub(super) query_data: tome::DirectTomeQueryData,
    pub(super) sphere_graph: Result<tome::SphereCellGraph, String>,
    pub(super) query_job_count: u32,
}

#[derive(Resource)]
pub(super) struct NativeShadowCasterWorld(pub(super) tome::shadow_caster::ShadowCasterTome);

#[derive(Resource)]
pub(super) struct NativeViewSettings(pub(super) Result<game_settings::ViewSettings, String>);

#[derive(Resource)]
pub(super) struct NavigationState {
    pub(super) walking: bool,
    pub(super) walker: navigation::Walker,
}

#[derive(Component)]
pub(super) struct CoordinateOverlay;

#[derive(Component)]
pub(super) struct FrameRateOverlay;

#[derive(Clone)]
pub(super) struct FocusTarget {
    pub(super) center: Vec3,
    pub(super) distance: f32,
    pub(super) label: String,
    pub(super) spawn: Option<entities::PlayerStart>,
}

#[derive(Resource)]
pub(super) struct FocusTargets {
    pub(super) targets: Vec<FocusTarget>,
    pub(super) active: usize,
}

#[derive(Resource)]
pub(super) struct SnapshotPath(pub(super) PathBuf);

#[derive(Resource)]
pub(super) struct BenchmarkSeconds(pub(super) f32, pub(super) f32);

#[derive(Resource)]
pub(super) struct OriginalLocalLightCullEnabled(pub(super) bool);

#[derive(Resource, Default)]
pub(super) struct OriginalLocalLightCullStats {
    pub(super) total: usize,
    pub(super) zero_preset_rgb: usize,
    pub(super) proximity_culled: usize,
    pub(super) proximity_unverified: usize,
    pub(super) projection_bounds_unverified: usize,
    pub(super) source_class_counts: [usize; 2],
    pub(super) static_name_hash_matches: usize,
    pub(super) static_id_culled: usize,
    pub(super) static_id_binding_active: bool,
    pub(super) world_query_bound: bool,
    pub(super) world_query_depth_grid_bound: bool,
    pub(super) world_query_unverified: usize,
    pub(super) world_query_error: Option<String>,
    pub(super) sphere_candidates: usize,
    pub(super) sphere_culled: usize,
    pub(super) sphere_unverified: usize,
    pub(super) sphere_query_ns: u128,
    pub(super) sphere_error: Option<String>,
    pub(super) clip_culled: usize,
    pub(super) area_culled: usize,
    pub(super) total_by_kind: [usize; 4],
    pub(super) visible_by_kind: [usize; 4],
}

#[derive(Component, Clone, Copy)]
pub(super) struct OriginalLocalLightCull {
    pub(super) range: f32,
    pub(super) kind: entities::LightType,
    pub(super) culling: entities::LightCulling,
    pub(super) definition_name_hash: u32,
    pub(super) linear_color: [f32; 3],
    pub(super) projection_bounds: Option<(Vec3, Vec3)>,
    pub(super) proximity_bounds: Option<(Vec3, f32)>,
    pub(super) initial_sphere: Option<[f32; 4]>,
    pub(super) clip_to_world: Option<Mat4>,
}

// Native gather retains candidate storage between calls too. Keep the point
// lookup and traversal in one batch, rather than allocating per-light masks.
#[derive(Default)]
pub(super) struct LocalLightSphereBatch {
    pub(super) worker: tome::SphereCellWorker,
    pub(super) spheres: Vec<[f32; 4]>,
    pub(super) point_cells: Vec<Option<u32>>,
    pub(super) entities: Vec<Entity>,
}

// The non-Umbra branch uses 128 / viewport area (32 physical pixels).
// The world-query branch instead consumes the native normalized area metric.
pub(super) const ORIGINAL_LOCAL_LIGHT_MIN_AREA_PIXELS: f32 = 32.0;
pub(super) const ORIGINAL_LIGHT_MIN_RANGE: f32 = 0.1;
pub(super) const ORIGINAL_LIGHT_FALLBACK_CULL_RADIUS: f32 = 32_768.0;

#[derive(Resource, Default)]
pub(super) struct VisibilityElapsedTiming {
    pub(super) samples: u64,
    pub(super) total_ns: u128,
    pub(super) max_ns: u128,
    pub(super) query_ns: u128,
    pub(super) mapping_ns: u128,
    pub(super) update_ns: u128,
}

// Keep exact per-frame inputs in memory, but flush ordinary disk samples only
// once per second or on failure/large visibility changes. No rendering fallback.
#[derive(Resource)]
pub(super) struct StreamingDiagnostics {
    pub(super) frame: u64,
    pub(super) pending: String,
    pub(super) history: VecDeque<String>,
    pub(super) log: Option<fs::File>,
    pub(super) log_path: PathBuf,
    pub(super) status: String,
    pub(super) previous_status: String,
    pub(super) previous_active: usize,
    pub(super) last_logged: Instant,
    pub(super) trace_texture_streaming: bool,
    pub(super) texture_events_pending: VecDeque<String>,
    pub(super) texture_event_history: VecDeque<String>,
    pub(super) texture_events_dropped: usize,
}

/// Resource names inside archives are not filesystem output directories.
/// Locate a checkout beside the executable, or keep outputs beside a packaged client.
pub(super) fn diagnostic_directory(kind: &str) -> PathBuf {
    let executable = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    let parent = executable.parent().unwrap_or(std::path::Path::new("."));
    let root = parent
        .ancestors()
        .find(|path| path.join("Cargo.toml").is_file())
        .unwrap_or(parent);
    root.join("artifacts").join(kind)
}

impl StreamingDiagnostics {
    pub(super) fn new(_source_path: &std::path::Path) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let directory = diagnostic_directory("streaming-reports");
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

    pub(super) fn texture_event(&mut self, message: impl Into<String>) {
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

    pub(super) fn record(&mut self, sample: String, active: usize, capture: bool) {
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
    pub(super) fn opt_in_texture_events_reach_session_log_and_recent_history() {
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
pub(super) enum RenderModelSurface {
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
pub(super) struct RenderSourcePlacements(pub(super) Vec<entities::PlacedModel>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RenderInstanceSource {
    pub(super) surface: RenderModelSurface,
    pub(super) world_baked: bool,
}

pub(super) struct RenderInstance {
    pub(super) source: Option<RenderInstanceSource>,
    pub(super) mesh: Handle<Mesh>,
    pub(super) material: MaterialHandle,
    pub(super) diffuse_image_slot: Option<usize>,
    pub(super) probe_index: u32,
    pub(super) transform: Transform,
    pub(super) bounds: Aabb,
    pub(super) pickup_id: Option<String>,
    pub(super) entity: Option<Entity>,
    pub(super) visible: bool,
}

pub(super) type DecalMaterial = ExtendedMaterial<StandardMaterial, DecalExtension>;
pub(super) type ProbeMaterial = ExtendedMaterial<StandardMaterial, ProbeExtension>;
pub(super) const PROBE_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("169be3e1-273e-4607-aadf-e0d572cbf41d");
pub(super) const PROBE_PREPASS_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("f78172ec-62f2-4bb4-9e45-c6e99fdf6c36");
pub(super) const VISTA_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("61fa0479-ac93-413b-bdf0-cb04c11a6e6f");
pub(super) const SURFACE_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("b7018b4d-b81d-44c0-9dd0-0b64cc879316");
pub(super) const DECAL_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("9165b294-a09d-46cb-af56-27edbe334a1b");
pub(super) const INDIRECT_SHADER_HANDLE: Handle<Shader> =
    bevy::asset::uuid_handle!("8257a42a-79cb-431e-862a-d5c8380347b5");
pub(super) const ORIGINAL_DEFAULT_HORIZONTAL_FOV_DEGREES: f32 = 80.0;
pub(super) const ORIGINAL_DEFAULT_NEAR: f32 = 0.01;
pub(super) const ORIGINAL_DEFAULT_FAR: f32 = 7000.0;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum MaterialHandle {
    Probe(Handle<ProbeMaterial>),
    Decal(Handle<DecalMaterial>),
}

#[derive(Clone)]
pub(super) struct ResolvedMaterial {
    pub(super) handle: MaterialHandle,
    pub(super) diffuse_image_slot: Option<usize>,
    pub(super) probe_index: u32,
    pub(super) native_vertex_lighting: bool,
}
