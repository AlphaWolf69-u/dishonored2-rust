#[cfg(feature = "game")]
pub mod auto_exposure;
#[cfg(feature = "game")]
pub mod camera_projection;
#[cfg(feature = "game")]
pub mod coarse_depth;
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
#[cfg(all(test, feature = "game"))]
mod submission_tests;
#[cfg(feature = "game")]
pub mod surface_gpu;
#[cfg(all(test, feature = "game"))]
mod texture_query_tests;
#[cfg(feature = "game")]
pub mod tiled_lighting_gpu;
