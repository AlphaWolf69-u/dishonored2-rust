#define_import_path dh2_parallel_receiver
#import dh2_shadow::{native_parallel_shadow_slice, native_shadow_offset, native_sample_directional_shadow, native_fast_cloud_shadowing}

// Matches native_shadow_sampling::NativeParallelSamplingUniform. This is the
// rewrite's stable transport, not a copy of an original constant-buffer ABI.
struct NativeParallelSamplingUniform {
    world_to_shadow: array<mat4x4<f32>, 5>,
    view_depth_row: vec4<f32>,
    cascade_fars: vec4<f32>,
    sun_direction: vec4<f32>,
    shadow_size: vec4<f32>,
    bias: vec4<f32>,
    config: vec4<u32>,
    quality: vec4<i32>,
    cloud_motion: vec4<f32>,
    cloud_density: vec4<f32>,
    camera_time: vec4<f32>,
}

@group(#{MATERIAL_BIND_GROUP}) @binding(120) var<storage, read> native_parallel: NativeParallelSamplingUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var native_parallel_depth: texture_depth_2d_array;
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var native_parallel_comparison: sampler_comparison;
@group(#{MATERIAL_BIND_GROUP}) @binding(123) var native_screen_visibility: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(124) var native_cloud: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(125) var native_cloud_sampler: sampler;

fn native_parallel_directional_index() -> u32 {
    return select(0xffffffffu, native_parallel.config.y, native_parallel.config.x != 0u);
}

// The original DIRECT_VISIBILITY compile branch calls ComputeParallelShadow
// with _Surface.wsPositionHacked and _Surface.wsVertexNormal, not the normal-map N.
// Ordinary opaque ScreenSpace consumers instead read the produced visibility
// texture; mode 2 must not fall through into this direct-atlas branch.
// Geometry has already been converted gameXYZ -> viewerX,Z,-Y; undo that
// exactly before using the original rows, ray and bias parameters.
fn native_parallel_depth_visibility(viewer_position: vec3<f32>, viewer_vertex_normal: vec3<f32>, pixel: vec2<f32>, route: vec4<u32>) -> f32 {
    if native_parallel.config.x == 0u || (native_parallel.config.x == 2u && route.x == 3u) { return 1.0; }
    if native_parallel.config.x == 2u && route.x == 0u {
        return textureLoad(native_screen_visibility, vec2<i32>(pixel), 0).y;
    }
    let position = vec3(viewer_position.x, -viewer_position.z, viewer_position.y);
    let normal = vec3(viewer_vertex_normal.x, -viewer_vertex_normal.z, viewer_vertex_normal.y);
    let slice = native_parallel_shadow_slice(position, native_parallel.view_depth_row, native_parallel.cascade_fars);
    let options = select(native_parallel.config.z, route.y, native_parallel.config.x == 2u);
    let coordinates = native_shadow_offset(native_parallel.world_to_shadow[slice], position, normal,
        native_parallel.sun_direction.xyz, true,
        (options & 1u) != 0u, (options & 2u) != 0u, (options & 4u) != 0u, (options & 8u) != 0u,
        native_parallel.bias.x, native_parallel.bias.y, native_parallel.bias.z);
    // shadowCommon's QUALITY is the original shadowFilteringQuality minus 1.
    var visibility = native_sample_directional_shadow(native_parallel_depth, native_parallel_comparison,
        coordinates, i32(slice), native_parallel.shadow_size.xy,
        select(native_parallel.quality.x, i32(route.w), native_parallel.config.x == 2u),
        select(native_parallel.quality.y, i32(route.z), native_parallel.config.x == 2u) - 1);
    if native_parallel.config.x == 2u && (route.y & 16u) != 0u && native_parallel.cloud_density.w != 0.0 {
        visibility *= native_fast_cloud_shadowing(position, native_parallel.sun_direction.xyz,
            native_parallel.camera_time.w, native_parallel.cloud_motion, native_parallel.cloud_density,
            native_cloud, native_cloud_sampler);
    }
    return visibility;
}
