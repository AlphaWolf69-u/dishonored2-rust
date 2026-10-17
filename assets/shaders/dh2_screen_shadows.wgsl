// Original screenShadows + bilateralBlur/blurX/blurY render programs.
// Concatenate after dh2_shadow.wgsl. All projection rows, environment values,
// compile choices and target sizes are resolved by the original CPU owner.
// Depth is in the ORIGINAL projection convention, not guessed reverse-Z.
struct ScreenParallelData {
    world_to_shadow: array<mat4x4<f32>, 5>,
    view_depth_row: vec4<f32>,
    cascade_fars: vec4<f32>,
    sun_direction: vec4<f32>,
    shadow_size: vec4<f32>,
    bias: vec4<f32>,
    config: vec4<u32>,
    quality: vec4<i32>,
}

struct NativeScreenShadowUniform {
    // Logical original inverse VP rows; explicit dots avoid storage transposes.
    inverse_view_projection: array<vec4<f32>, 4>,
    // -inverseProjection[2].w, inverseProjection[3].z, [3].w, unused.
    unproject: vec4<f32>,
    camera_time: vec4<f32>,
    // resolutionScale.xy, screenShadows/stepSize, modelDepthHack.x.
    depth_scale_step_hack: vec4<f32>,
    // width, height, original blurRadius, axis (0=X, 1=Y).
    extent_radius_axis: vec4<u32>,
    // falloff, sharpness, unused, unused.
    blur: vec4<f32>,
    // cloudAltitude, cloudRotation, cloudSpeedX, cloudSpeedY.
    cloud_motion: vec4<f32>,
    // cloudTilingX, cloudTilingY, cloudDensity, unused.
    cloud_density: vec4<f32>,
    // useJitter, parallelLightHasShadow, original cloud branch enabled, unused.
    options: vec4<u32>,
}

@group(0) @binding(0) var<storage, read> screen_parallel: ScreenParallelData;
@group(0) @binding(1) var<uniform> screen: NativeScreenShadowUniform;
@group(0) @binding(2) var screen_depth: texture_depth_2d;
@group(0) @binding(3) var screen_point: sampler;
@group(0) @binding(4) var screen_atlas: texture_depth_2d_array;
@group(0) @binding(5) var screen_comparison: sampler_comparison;
@group(0) @binding(6) var screen_cloud: texture_2d<f32>;
@group(0) @binding(7) var screen_cloud_sampler: sampler;
@group(0) @binding(8) var screen_source: texture_2d<f32>;

struct ScreenVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex fn screen_vertex(@builtin(vertex_index) index: u32) -> ScreenVertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: ScreenVertex;
    out.position = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv;
    return out;
}

// helper::B2/B8, not a conventional substituted Bayer ordering.
fn screen_b2(p: vec2<u32>) -> u32 { return ((p.y << 1u) + p.x + 1u) & 3u; }
fn screen_b8(p: vec2<u32>) -> u32 {
    return (((screen_b2(p & vec2(1u)) << 2u)
        + screen_b2((p & vec2(3u)) >> vec2(1u))) << 2u)
        + screen_b2((p & vec2(7u)) >> vec2(2u));
}

// shadowCommon::GetFastCloudShadowing uses the FLOAT-output overload of
// intersection::IntersectRaySphere. Its discriminant clamps to zero and
// near hit clamps to zero; the other overload's SolveQuadratic is different.
fn screen_cloud_visibility(position: vec3<f32>) -> f32 {
    return native_fast_cloud_shadowing(position, screen_parallel.sun_direction.xyz,
        screen.camera_time.w, screen.cloud_motion, screen.cloud_density, screen_cloud, screen_cloud_sampler);
}

@fragment fn screen_visibility(input: ScreenVertex) -> @location(0) vec4<f32> {
    let depth = textureSampleLevel(screen_depth, screen_point,
        input.uv * screen.depth_scale_step_hack.xy, 0) * screen.depth_scale_step_hack.w;
    let ndc = vec4(input.uv.x * 2.0 - 1.0, 1.0 - input.uv.y * 2.0, depth, 1.0);
    let homogeneous = vec4(dot(screen.inverse_view_projection[0], ndc),
        dot(screen.inverse_view_projection[1], ndc),
        dot(screen.inverse_view_projection[2], ndc),
        dot(screen.inverse_view_projection[3], ndc));
    var position = homogeneous.xyz / homogeneous.w;
    if screen.options.x != 0u {
        let direction = normalize(screen.camera_time.xyz - position);
        let pixel = vec2<u32>(input.position.xy) % vec2(8u);
        let offset = f32(screen_b8(pixel)) * screen.depth_scale_step_hack.z * (1.0 / 64.0);
        position += direction * offset;
    }
    var visibility = 1.0;
    if screen.options.y != 0u {
        let slice = native_parallel_shadow_slice(position,
            screen_parallel.view_depth_row, screen_parallel.cascade_fars);
        let options = screen_parallel.config.z;
        // Original screenShadows has no vertex normal and DISABLES normal
        // offset. Other resolved slope options/bias remain the owner's values.
        let coordinates = native_shadow_offset(screen_parallel.world_to_shadow[slice],
            position, vec3(0.0), screen_parallel.sun_direction.xyz, true, false,
            (options & 2u) != 0u, (options & 4u) != 0u, (options & 8u) != 0u,
            screen_parallel.bias.x, screen_parallel.bias.y, screen_parallel.bias.z);
        visibility = native_sample_directional_shadow(screen_atlas, screen_comparison,
            coordinates, i32(slice), screen_parallel.shadow_size.xy, 2,
            screen_parallel.quality.y - 1);
    }
    if screen.options.z != 0u { visibility *= screen_cloud_visibility(position); }
    // Original scalar output replicates into the channels enabled by the
    // actual pass's colour-write mask; never choose a channel here by guess.
    return vec4(visibility);
}

fn screen_linear_z(raw_depth: f32) -> f32 {
    return screen.unproject.x / (screen.unproject.y * raw_depth + screen.unproject.z);
}

// DWAO's depth transport is independent of its AO calculation. Only G/BA
// are enabled here; no AO value is fabricated for the lighting consumer.
@fragment fn screen_pack_depth(input: ScreenVertex) -> @location(0) vec4<f32> {
    let z = screen_linear_z(textureLoad(screen_depth, vec2<i32>(input.position.xy), 0));
    let value = z * 257.0;
    let high = trunc(value / 256.0);
    return vec4(0.0, 1.0, high / 255.0, (value - high * 256.0) / 255.0);
}

@fragment fn screen_bilateral(input: ScreenVertex) -> @location(0) vec4<f32> {
    let center = vec2<i32>(floor(input.position.xy));
    let center_depth = screen_linear_z(textureLoad(screen_depth, center, 0));
    let axis = select(vec2<i32>(1, 0), vec2<i32>(0, 1), screen.extent_radius_axis.w != 0u);
    let coordinate = select(center.x, center.y, screen.extent_radius_axis.w != 0u);
    let extent = i32(select(screen.extent_radius_axis.x, screen.extent_radius_axis.y, screen.extent_radius_axis.w != 0u));
    let radius = i32(screen.extent_radius_axis.z);
    let first = max(0, coordinate - radius) - coordinate;
    let last = min(coordinate + radius + 1, extent) - coordinate;
    var weighted = 0.0;
    var weights = 0.0;
    for (var i = first; i < last; i += 1) {
        let pixel = center + axis * i;
        let depth = screen_linear_z(textureLoad(screen_depth, pixel, 0));
        let value = textureLoad(screen_source, pixel, 0).y;
        let difference = depth - center_depth;
        let weight = exp(f32(i * i) * screen.blur.x
            + difference * difference * screen.blur.y) + 0.001;
        weighted += weight * value;
        weights += weight;
    }
    return vec4(weighted / weights);
}
