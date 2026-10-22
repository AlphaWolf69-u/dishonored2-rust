#[cfg(feature = "game")]
pub mod auto_exposure;
#[cfg(feature = "game")]
pub mod alpha_depth;
#[cfg(feature = "game")]
pub mod water_depth;
#[cfg(feature = "game")]
pub mod underwater_stencil;
#[cfg(feature = "game")]
pub mod ocean_water_line;
#[cfg(feature = "game")]
pub mod water_depth_program;
#[cfg(feature = "game")]
pub mod water_scene;
#[cfg(feature = "game")]
pub mod water_hdr;
#[cfg(feature = "game")]
pub mod position_meshes;
#[cfg(feature = "game")]
pub mod position_depth;
#[cfg(feature = "game")]
pub mod camera_projection;
#[cfg(feature = "game")]
pub mod coarse_depth;
#[cfg(feature = "game")]
pub mod fog;
#[cfg(feature = "game")]
pub mod fog_lookup;
#[cfg(feature = "game")]
pub mod fog_capture;
#[cfg(feature = "game")]
pub mod display_capture;
#[cfg(feature = "game")]
pub mod fog_shadow;
#[cfg(feature = "game")]
pub mod ocean_waves;
#[cfg(feature = "game")]
pub mod ocean_surface;
#[cfg(feature = "game")]
pub mod ocean_vertex;
#[cfg(feature = "game")]
pub mod ocean_depth;
#[cfg(feature = "game")]
pub mod ocean_mesh;
#[cfg(feature = "game")]
pub mod ocean_colour;
#[cfg(feature = "game")]
pub mod water_environment;
#[cfg(feature = "game")]
pub mod image_formats;
#[cfg(feature = "game")]
pub mod layered_images;
#[cfg(feature = "game")]
pub mod water_refraction;
#[cfg(feature = "game")]
pub mod water_reflection;
#[cfg(feature = "game")]
pub mod water_surface;
#[cfg(feature = "game")]
pub mod colour_output;
#[cfg(all(test, feature = "game"))]
mod test_depth;
#[cfg(all(test, feature = "game"))]
mod test_water;
#[cfg(all(test, feature = "game"))]
mod test_water_inputs;
#[cfg(feature = "game")]
pub mod screen_ray;
#[cfg(feature = "game")]
pub mod ocean_height_queries;
#[cfg(feature = "game")]
pub mod water_foam_depth;
#[cfg(feature = "game")]
pub mod water_normals;
#[cfg(feature = "game")]
pub mod water_colour;
#[cfg(feature="game")]
pub mod water_probes;
#[cfg(feature="game")]
pub mod water_world_colour;
#[cfg(feature="game")]
pub mod probe_weights;
#[cfg(feature = "game")]
pub mod noise_textures;
#[cfg(all(test, feature = "game"))]
mod fog_alpha_tests;
#[cfg(feature = "game")]
pub mod fog_filter;
#[cfg(feature = "game")]
pub mod fog_light_scattering;
#[cfg(feature = "game")]
pub mod tiled_lighting_pass;
#[cfg(feature = "game")]
pub mod water_depth_scene;
#[cfg(all(test, feature = "game"))]
mod fog_light_scattering_tests;
#[cfg(all(test, feature = "game"))]
mod fog_filter_tests;
#[cfg(feature = "game")]
pub mod fog_material;
#[cfg(feature = "game")]
pub mod fog_objects;
#[cfg(feature = "game")]
mod fog_material_pass;
#[cfg(feature = "game")]
pub mod fog_meshes;
#[cfg(feature = "game")]
pub mod fog_pass;
#[cfg(all(test, feature = "game"))]
mod fog_pass_tests;
#[cfg(feature = "game")]
pub mod fog_pipeline;
#[cfg(feature = "game")]
pub mod fog_rays;
#[cfg(all(test, feature = "game"))]
mod fog_rays_tests;
#[cfg(feature = "game")]
mod fog_scene;
#[cfg(all(test, feature = "game"))]
mod fog_scene_tests;
#[cfg(feature = "game")]
pub mod fog_sun_mask;
#[cfg(feature = "game")]
pub mod fog_sun_sprite;
#[cfg(feature = "game")]
pub mod fog_targets;
#[cfg(feature = "game")]
pub mod fog_volumes;
#[cfg(all(test, feature = "game"))]
mod fog_tests;
#[cfg(feature = "game")]
pub mod pipeline_state;
#[cfg(feature = "game")]
pub mod screen_shadow_pass;
#[cfg(feature = "game")]
pub mod screen_shadow_pipeline;
#[cfg(feature = "game")]
pub mod shadow_depth_copy;
#[cfg(feature = "game")]
pub mod shadow_pass;
#[cfg(feature = "game")]
pub mod shadow_pipeline;
#[cfg(feature = "game")]
pub mod shadow_sampling;
#[cfg(feature = "game")]
pub mod sky_mask;
#[cfg(all(test, feature = "game"))]
mod sky_mask_tests;
#[cfg(feature = "game")]
pub mod sky_pass;
#[cfg(all(test, feature = "game"))]
mod submission_tests;
#[cfg(feature = "game")]
pub mod surface_gpu;
#[cfg(all(test, feature = "game"))]
mod texture_query_tests;
#[cfg(feature = "game")]
pub mod tiled_lighting_gpu;
#[cfg(all(test, feature = "game"))]
mod depth_restore_tests;

#[cfg(all(test, feature = "game", target_os = "windows"))]
mod command_pool_tests;
#[cfg(all(test, feature = "game"))]
mod storage_upload_tests;
#[cfg(feature = "game")]
pub mod colour_cube;
#[cfg(feature = "game")]
pub mod tone_mapping;
#[cfg(feature = "game")]
pub mod display_scene;

#[cfg(feature = "game")]
pub mod bloom;
#[cfg(feature = "game")]
pub mod luminance;
#[cfg(feature = "game")]
pub mod lens_streak;
#[cfg(feature = "game")]
pub mod lens_composite;
#[cfg(feature = "game")]
pub mod lens_flare;
#[cfg(feature = "game")]
pub mod colour_downscale;
#[cfg(feature = "game")]
pub mod lens_scene;
