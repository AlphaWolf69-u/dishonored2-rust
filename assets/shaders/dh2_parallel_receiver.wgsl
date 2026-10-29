#define_import_path dh2_parallel_receiver
#import dh2_shadow::{parallel_shadow_slice, shadow_offset, shadow_position, sample_directional_shadow, fast_cloud_shadowing}

// Matches shadow_sampling::ParallelSamplingUniform. This is the
// rewrite's stable transport, not a copy of an original constant-buffer ABI.
struct ParallelSamplingUniform {
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

@group(#{MATERIAL_BIND_GROUP}) @binding(120) var<storage, read> parallel: ParallelSamplingUniform;
@group(#{MATERIAL_BIND_GROUP}) @binding(121) var parallel_depth: texture_depth_2d_array;
@group(#{MATERIAL_BIND_GROUP}) @binding(122) var parallel_comparison: sampler_comparison;
@group(#{MATERIAL_BIND_GROUP}) @binding(123) var screen_visibility: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(124) var cloud: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(125) var cloud_sampler: sampler;

fn parallel_directional_index() -> u32 {
    return select(0xffffffffu, parallel.config.y, parallel.config.x != 0u);
}

// lightContext::ComputeAmbientOcclusion: screen R also applies to opaque
// noScreenShadows materials. An unavailable current view is neutral AO.
fn ambient_occlusion(pixel: vec2<f32>, material_ao: f32, is_alpha: bool) -> f32 {
    if is_alpha { return 1.0; }
    if parallel.quality.z == 0 { return material_ao; }
    return min(material_ao, textureLoad(screen_visibility, vec2<i32>(pixel), 0).x);
}

// The original DIRECT_VISIBILITY compile branch calls ComputeParallelShadow
// with _Surface.wsPositionHacked and _Surface.wsVertexNormal, not the normal-map N.
// Ordinary opaque ScreenSpace consumers instead read the produced visibility
// texture; mode 2 must not fall through into this direct-atlas branch.
// Geometry has already been converted gameXYZ -> viewerX,Z,-Y; undo that
// exactly before using the original rows, ray and bias parameters.
fn parallel_depth_visibility(viewer_position: vec3<f32>, viewer_vertex_normal: vec3<f32>, pixel: vec2<f32>, route: vec4<u32>) -> f32 {
    if parallel.config.x == 0u || (parallel.config.x == 2u && route.x == 3u) { return 1.0; }
    if parallel.config.x == 2u && route.x == 0u {
        return textureLoad(screen_visibility, vec2<i32>(pixel), 0).y;
    }
    let position = vec3(viewer_position.x, -viewer_position.z, viewer_position.y);
    let normal = vec3(viewer_vertex_normal.x, -viewer_vertex_normal.z, viewer_vertex_normal.y);
    let slice = parallel_shadow_slice(position, parallel.view_depth_row, parallel.cascade_fars);
    let options = select(parallel.config.z, route.y, parallel.config.x == 2u);
    let coordinates = shadow_offset(parallel.world_to_shadow[slice], position, normal,
        parallel.sun_direction.xyz, true,
        (options & 1u) != 0u, (options & 2u) != 0u, (options & 4u) != 0u, (options & 8u) != 0u,
        parallel.bias.x, parallel.bias.y, parallel.bias.z);
    // shadowCommon's QUALITY is the original shadowFilteringQuality minus 1.
    var visibility = sample_directional_shadow(parallel_depth, parallel_comparison,
        coordinates, i32(slice), parallel.shadow_size.xy,
        select(parallel.quality.x, i32(route.w), parallel.config.x == 2u),
        select(parallel.quality.y, i32(route.z), parallel.config.x == 2u) - 1);
    if parallel.config.x == 2u && (route.y & 16u) != 0u && parallel.cloud_density.w != 0.0 {
        visibility *= fast_cloud_shadowing(position, parallel.sun_direction.xyz,
            parallel.camera_time.w, parallel.cloud_motion, parallel.cloud_density,
            cloud, cloud_sampler);
    }
    return visibility;
}

// BRDF_vegetation's shadowed irradiance for the parallel light, as the eotw
// leaf shader compiles it (capture event 54031): lighting.renderprog sets
// m_ShadowSetup = 0 and never assigns m_shadowInfo, so GetShadowMapZandPos
// uses the cascade's cb_shadowprojections row set for BOTH transforms; the
// "caster" is that matrix applied to (uv, mapZ). The cascade comes from the
// depth-reconstructed position, the fetch and distance from the interpolated
// world position; the depth is filtered bilinearly with no bias.
// Returns enableShadow * exp(-w * isBehindCaster * distance).
fn parallel_thickness_visibility(viewer_position: vec3<f32>, viewer_hacked: vec3<f32>, w: f32) -> f32 {
    if parallel.config.x == 0u { return 1.0; }
    let position = vec3(viewer_position.x, -viewer_position.z, viewer_position.y);
    let hacked = vec3(viewer_hacked.x, -viewer_hacked.z, viewer_hacked.y);
    let slice = parallel_shadow_slice(hacked, parallel.view_depth_row, parallel.cascade_fars);
    let matrix = parallel.world_to_shadow[slice];
    let coordinate = shadow_position(matrix, position, true).xyz;
    // SampleLevel(bilinearSampler): four texels, standard texel-centre weights.
    let size = vec2<f32>(textureDimensions(parallel_depth));
    let texel = coordinate.xy * size - 0.5;
    let base = floor(texel);
    let f = texel - base;
    let last = vec2<i32>(size) - 1;
    let a = clamp(vec2<i32>(base), vec2(0), last);
    let b = clamp(vec2<i32>(base) + 1, vec2(0), last);
    let d00 = textureLoad(parallel_depth, a, i32(slice), 0);
    let d10 = textureLoad(parallel_depth, vec2(b.x, a.y), i32(slice), 0);
    let d01 = textureLoad(parallel_depth, vec2(a.x, b.y), i32(slice), 0);
    let d11 = textureLoad(parallel_depth, b, i32(slice), 0);
    let map_z = mix(mix(d00, d10, f.x), mix(d01, d11, f.x), f.y);
    let caster = shadow_position(matrix, vec3(coordinate.xy, map_z), true).xyz;
    let enable_shadow = clamp(1e5 * map_z, 0.0, 1.0);
    let behind = clamp(1e4 * (coordinate.z - map_z), 0.0, 1.0);
    return enable_shadow * exp(-w * behind * distance(position, caster));
}

// BRDF_skin's directional `GetFastCloudShadowing( _Surface.wsPositionHacked )`,
// applied in the BRDF on top of the light's shadow term.
fn parallel_cloud_shadowing(viewer_position: vec3<f32>) -> f32 {
    if parallel.config.x == 0u || parallel.cloud_density.w == 0.0 { return 1.0; }
    let position = vec3(viewer_position.x, -viewer_position.z, viewer_position.y);
    return fast_cloud_shadowing(position, parallel.sun_direction.xyz,
        parallel.camera_time.w, parallel.cloud_motion, parallel.cloud_density,
        cloud, cloud_sampler);
}
