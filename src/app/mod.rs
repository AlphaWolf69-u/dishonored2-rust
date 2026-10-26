//! Desktop client composition and application systems.
use crate::{
    archive, auto_exposure, bmd6, bmodel, camera_projection, coarse_depth, edge_skeleton, entities,
    frame_profile, game_clock, game_clock_runtime, game_settings, local_caster_query,
    local_shadow_cache, material, material_surface, navigation, physworld, probe_lighting,
    render_light, render_model, render_program, render_shadow, render_state, runtime,
    screen_shadow_pass, screen_shadow_pipeline, shader_cache, shadow_pass, shadow_pipeline,
    shadow_receiver, shadow_sampling, surface_gpu, texture, tiled_lighting, tome, worker_topology,
    world_mesh,
};
use bevy::app::AppExit;
use bevy::asset::{AssetId, RenderAssetUsages};
use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::VisibilitySystems;
use bevy::camera::Projection;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::image::{ImageAddressMode, ImageSampler, ImageSamplerDescriptor};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::light::{
    CascadeShadowConfigBuilder, DirectionalLightShadowMap, PointLightShadowMap, RectLight,
};
use bevy::mesh::{Indices, MeshTag, MeshVertexAttribute, MeshVertexBufferLayoutRef};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline,
    MaterialPlugin,
};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, PrimitiveTopology, RenderPipelineDescriptor,
    SpecializedMeshPipelineError, TexelCopyBufferLayout, TextureDimension, TextureFormat,
    VertexFormat,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::storage::ShaderBuffer;
use bevy::render::texture::GpuImage;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::render::{Render, RenderApp, RenderSystems};
use bevy::shader::{Shader, ShaderRef};
use bevy::window::{CursorGrabMode, CursorOptions};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

mod local_shadow_updates;
mod fog_lights;
mod environment_reflection;
mod model_motion;
mod particle_view;
mod machinery;
mod physics_skinning;
mod shadow_updates;
mod shadow_diagnostics;
mod world_instances;
mod water_objects;
mod water_scene;
mod physics_scene;
mod state;
use state::{
    BenchmarkSeconds, CoordinateOverlay, DecalMaterial, FocusTarget, FocusTargets,
    FrameRateOverlay, LocalLightSphereBatch, MaterialHandle, ShadowCasterWorld,
    ViewSettings, NavigationWorld, LocalLightCull,
    LocalLightCullEnabled, LocalLightCullStats, PickupInstance, PickupState,
    PickupVisual, ProbeMaterial, RenderInstance, RenderInstanceSource, RenderModelSurface,
    RenderSourcePlacements, ResolvedMaterial, SceneData, SnapshotPath, StreamingDiagnostics,
    TomeWorld, VisibilityElapsedTiming, DECAL_SHADER_HANDLE, FILLING_SHADER_HANDLE,
    INDIRECT_SHADER_HANDLE, DEFAULT_FAR, DEFAULT_HORIZONTAL_FOV_DEGREES,
    DEFAULT_NEAR, LOCAL_LIGHT_MIN_AREA_PIXELS, PROBE_PREPASS_SHADER_HANDLE,
    PROBE_SHADER_HANDLE, SKY_SHADER_HANDLE, SURFACE_SHADER_HANDLE, VISTA_SHADER_HANDLE,
};
mod shadow_preparation;
use shadow_preparation::{
    apply_rest_pose, build_world_layer, initialize_parallel_shadow_owner,
    rest_skinning, shadow_target_dimensions, publish_initial_whole_view_depth,
    retain_model_shadow_position, retain_skin_inputs, sync_shadow_target,
    CachedMaterial, ParallelShadowOwner, ProgramCacheSource,
    ShadowMaterialInputs, ShadowMaterialParameters, ShadowMaterials,
    VegetationMeshInputs, WorldShadowDrawSettings, WorldShadowMeshSources,
};
mod materials;
// Standalone historical sprite comparisons; gameplay uses fog_scene's sun pass.
#[cfg(test)]
mod sky_sprite;
mod fog_scene;
mod display_scene;
mod camera_effects;
mod alpha_depth_pass;
mod lens_scene;
mod fog_volumes;
mod fog_objects;
mod vegetation_wind;
use materials::{
    DecalExtension, ImageAccess, ShadowImageAccess, ProbeExtension, DH2_BITANGENT, DH2_NORMAL_W,
};
mod scene_queue;
mod decal_pass;
use scene_queue::{
    bwm_lod_visibility_counts, ShadowModelBoundsSource, ShadowModelBoundsSources,
    PlacedModelQueries, PlacedModelQueryGroup, RenderSuppressionGroup, SceneRenderQueue,
};
mod texture_streaming;
use texture_streaming::{
    surface_sampler_image, read_streamed_diffuse_at_target, report_prepared_diffuse_images,
    stream_visible_textures, DiffuseMipState, PreparedImageMailbox, TextureReadTarget,
    TextureReadWorker,
};
mod visibility_instances;
use visibility_instances::TomeRenderInstanceMap;
mod view_geometry;
use view_geometry::{
    apply_placed_model_queries, authored_area_light_transform, authored_light_cosines,
    authored_mesh, authored_world_bounds, environment_sun_ray, expanded_model_placements,
    f8_camera_pose, f8_reference_fov, game_affine_to_view, game_to_view,
    initial_placed_model_query, light_backend_intensity, mean_game_position,
    view_distance_scale, needs_exact_world_mesh, light_distance,
    light_preset, light_volume_intersects_camera,
    local_light_clip_to_world, placed_direction, placed_transform, placed_vertex,
    preserved_half_point_angular_scale, reference_fov_from_projection, render_bounds,
    retain_vegetation_channels, retain_world_shadow_channels, source_mesh,
    static_model_collision, transformed, transformed_direction, view_of, view_to_game, Bounds,
};
mod material_loading;
use material_loading::{material_option_overrides, MaterialResolver};
mod setup;
use setup::setup;
mod controls;
use controls::{camera_controls, interact_pickup, update_coordinate_overlay};
mod diagnostics;
use diagnostics::{collision_report, game_pose_from_view_transform, snapshot_once};
mod visibility_queries;
use visibility_queries::stream_visible_models;
mod light_culling;
#[cfg(test)]
use light_culling::light_proximity_bounds;
use light_culling::{
    cull_small_local_lights, local_light_kind_index, light_projection_bounds,
    light_proximity_from_cached_bounds,
};
mod performance;
use performance::{benchmark_frames, finish_streaming_diagnostics, show_frame_rate};
mod launch;
mod audio;
mod engine_keys;
mod player;
mod player_arms;
mod pause_menu;
mod pause_fx;
mod display_settings;

/// Run the desktop application with installed game resources.
pub fn main() {
    launch::main();
}

#[cfg(test)]
mod tests;
