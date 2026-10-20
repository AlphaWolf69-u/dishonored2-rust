// Original shadow/shadowCommon/shadow_paraboloid renderprog algorithms.
// Matrices contain the ORIGINAL row vectors as WGSL columns; no matrix guessing.
// Geometry/atlas allocation and the original bias/settings producer remain external.
// Cloud attenuation additionally requires the original atmosphere/intersection path.
struct ShadowInfo {
    world_to_shadow: mat4x4<f32>,
    shadow_to_world: mat4x4<f32>,
    info: u32,
    unused_tail_a: u32,
    unused_tail_b: u32,
    unused_tail_c: u32,
}

// HLSL saturate's _sat modifier returns zero for NaN. WGSL clamp alone
// does not retain that contract, which matters at perspective W=0 planes.
fn shadow_saturate(value:f32)->f32 {
    if (bitcast<u32>(value)&0x7fffffffu)>0x7f800000u {return 0.0;}
    return clamp(value,0.0,1.0);
}

// The original D3D11 normalized-depth comparison treats a NaN reference as
// zero. The WGPU comparison call does not; native non-finite PCF cases verify
// this transport conversion independently of the filter's final saturate.
fn shadow_depth_reference(value:f32)->f32 {
    if (bitcast<u32>(value)&0x7fffffffu)>0x7f800000u {return 0.0;}
    return value;
}

fn shadow_slice(info: u32) -> i32 { return i32(info & 31u); }

fn omni_shadow_side(direction: vec3<f32>) -> u32 {
    let a = abs(direction);
    if a.x > a.y {
        if a.x > a.z { return select(4u, 5u, direction.x > 0.0); }
    } else if a.y > a.z { return select(0u, 1u, direction.y > 0.0); }
    return select(2u, 3u, direction.z > 0.0);
}

fn spot_shadow_side(to_light: vec3<f32>, left: vec3<f32>) -> u32 {
    return select(1u, 0u, dot(to_light, left) > 0.0);
}

fn parallel_shadow_slice(position: vec3<f32>, view_row2: vec4<f32>, fars: vec4<f32>) -> u32 {
    let z = -dot(vec4(position, 1.0), view_row2);
    let mask = step(fars, vec4(z));
    return u32(mask.x + mask.y + mask.z + mask.w);
}

// shadowCommon::GetFastCloudShadowing and the float-output ray/sphere overload.
fn fast_cloud_shadowing(position: vec3<f32>, direction: vec3<f32>, world_time: f32,
    motion: vec4<f32>, density_parameters: vec4<f32>, cloud: texture_2d<f32>, cloud_sampler: sampler) -> f32 {
    let center = vec3(0.0, 0.0, -motion.x);
    let radius = motion.x + 2.0;
    let origin = 0.001 * position;
    let diff = origin - center;
    let a0 = dot(diff, diff) - radius * radius;
    let a1 = dot(direction, diff);
    let root = sqrt(max(0.0, a1 * a1 - a0));
    let near_hit = max(-a1 - root, 0.0);
    let far_hit = -a1 + root;
    let hit = origin + direction * select(near_hit, far_hit, far_hit > 0.0);
    let angle = world_time * motion.y;
    let sine = sin(angle);
    let cosine = cos(angle);
    var uv = vec2(cosine * hit.x - sine * hit.y, sine * hit.x + cosine * hit.y);
    uv += vec2(world_time) * motion.zw;
    uv *= density_parameters.xy;
    let density = textureSampleLevel(cloud, cloud_sampler, uv, 0.0).x * density_parameters.z;
    return clamp(exp(-density), 0.0, 1.0);
}

fn shadow_position(matrix: mat4x4<f32>, position: vec3<f32>, directional: bool) -> vec4<f32> {
    let p = vec4(position, 1.0);
    if directional { return vec4(dot(matrix[0], p), dot(matrix[1], p), dot(matrix[2], p), 1.0); }
    // shadowCommon uses mul(position, mWorldToShadowMat). Native structured
    // records store the CPU rows as HLSL's column-major float4 columns; the
    // row-vector product dots those columns. Our transport retains those
    // same four vectors as WGSL columns.
    return vec4(dot(matrix[0], p), dot(matrix[1], p), dot(matrix[2], p), dot(matrix[3], p));
}

fn uv_normal_offset(matrix: mat4x4<f32>, position: vec3<f32>, offset: vec4<f32>, directional: bool) -> vec4<f32> {
    var p = shadow_position(matrix, position, directional);
    p.x = offset.x;
    p.y = offset.y;
    return p;
}

fn shadow_normal_scale(normal: vec3<f32>, light_dir: vec3<f32>, slope_scale: bool) -> vec2<f32> {
    if slope_scale {
        let cosine = dot(normalize(light_dir), normal);
        return vec2(cosine, shadow_saturate(1.0 - cosine));
    }
    return vec2(0.0, 1.0);
}

// Arguments represent original compile-time options, not invented defaults.
// shadowCommon's uvNormalOffset call has an omitted directional argument; its
// compiled caller still needs verification. It is deliberately NOT enabled here.
fn shadow_offset(matrix: mat4x4<f32>, position: vec3<f32>, normal: vec3<f32>, light_dir: vec3<f32>, directional: bool,
    use_normal_offset: bool, use_slope_scale: bool, use_slope_bias: bool, post_projection: bool,
    normal_offset: f32, bias: f32, slope_bias: f32) -> vec4<f32> {
    let scale = shadow_normal_scale(normal, light_dir, use_slope_scale);
    var world_position = position;
    if use_normal_offset { world_position += normal * normal_offset * scale.y; }
    var p = shadow_position(matrix, world_position, directional);
    var z_bias = bias;
    if use_slope_bias {
        var slope = 0.0;
        if post_projection {
            var tangent = light_dir - normal * dot(normal, light_dir);
            tangent = normalize(tangent);
            let q = shadow_position(matrix, position + tangent, directional);
            let delta = q / q.w - p / p.w;
            slope = clamp(max(abs(delta.z) / (abs(delta.x) + 1e-10), abs(delta.z) / (abs(delta.y) + 1e-10)), 0.0, 10.0);
        } else if use_slope_scale {
            slope = sqrt(1.0 - scale.x * scale.x) / max(scale.x, 0.00001);
        }
        z_bias += slope * slope_bias;
    }
    p.z -= z_bias;
    p.x /= p.w; p.y /= p.w; p.z /= p.w;
    return p;
}

fn shadow_tap(map: texture_depth_2d_array, comparison: sampler_comparison, base: vec2<f32>, offset: vec2<f32>, inv_size: vec2<f32>, slice: i32, depth: f32) -> f32 {
    return textureSampleCompareLevel(map, comparison, base + offset * inv_size, slice, shadow_depth_reference(depth));
}

// Original optimized 2x2/3x3/5x5 PCF uses 1/4/9 hardware-comparison samples.
// Keep accumulation order and normalizations; don't replace with a generic grid.
fn shadow_pcf(map: texture_depth_2d_array, comparison: sampler_comparison, position: vec3<f32>, slice: i32, size: vec2<f32>, quality: i32) -> f32 {
    if quality == 0 { return shadow_saturate(textureSampleCompareLevel(map, comparison, position.xy, slice, shadow_depth_reference(position.z))); }
    let uv = position.xy * size;
    let inv_size = 1.0 / size;
    let base = floor(uv + 0.5);
    let st = uv + 0.5 - base;
    let base_uv = (base - 0.5) * inv_size;
    let s = st.x; let t = st.y;
    var sum = 0.0;
    if quality == 1 {
        let uw0 = 3.0 - 2.0 * s; let uw1 = 1.0 + 2.0 * s;
        let u0 = (2.0 - s) / uw0 - 1.0; let u1 = s / uw1 + 1.0;
        let vw0 = 3.0 - 2.0 * t; let vw1 = 1.0 + 2.0 * t;
        let v0 = (2.0 - t) / vw0 - 1.0; let v1 = t / vw1 + 1.0;
        sum += uw0 * vw0 * shadow_tap(map, comparison, base_uv, vec2(u0, v0), inv_size, slice, position.z);
        sum += uw1 * vw0 * shadow_tap(map, comparison, base_uv, vec2(u1, v0), inv_size, slice, position.z);
        sum += uw0 * vw1 * shadow_tap(map, comparison, base_uv, vec2(u0, v1), inv_size, slice, position.z);
        sum += uw1 * vw1 * shadow_tap(map, comparison, base_uv, vec2(u1, v1), inv_size, slice, position.z);
        return shadow_saturate(sum * (1.0 / 16.0));
    }
    let uw0 = 4.0 - 3.0 * s; let uw1 = 7.0; let uw2 = 1.0 + 3.0 * s;
    let u0 = (3.0 - 2.0 * s) / uw0 - 2.0; let u1 = (3.0 + s) / uw1; let u2 = s / uw2 + 2.0;
    let vw0 = 4.0 - 3.0 * t; let vw1 = 7.0; let vw2 = 1.0 + 3.0 * t;
    let v0 = (3.0 - 2.0 * t) / vw0 - 2.0; let v1 = (3.0 + t) / vw1; let v2 = t / vw2 + 2.0;
    sum += uw0 * vw0 * shadow_tap(map, comparison, base_uv, vec2(u0, v0), inv_size, slice, position.z);
    sum += uw1 * vw0 * shadow_tap(map, comparison, base_uv, vec2(u1, v0), inv_size, slice, position.z);
    sum += uw2 * vw0 * shadow_tap(map, comparison, base_uv, vec2(u2, v0), inv_size, slice, position.z);
    sum += uw0 * vw1 * shadow_tap(map, comparison, base_uv, vec2(u0, v1), inv_size, slice, position.z);
    sum += uw1 * vw1 * shadow_tap(map, comparison, base_uv, vec2(u1, v1), inv_size, slice, position.z);
    sum += uw2 * vw1 * shadow_tap(map, comparison, base_uv, vec2(u2, v1), inv_size, slice, position.z);
    sum += uw0 * vw2 * shadow_tap(map, comparison, base_uv, vec2(u0, v2), inv_size, slice, position.z);
    sum += uw1 * vw2 * shadow_tap(map, comparison, base_uv, vec2(u1, v2), inv_size, slice, position.z);
    sum += uw2 * vw2 * shadow_tap(map, comparison, base_uv, vec2(u2, v2), inv_size, slice, position.z);
    return shadow_saturate(sum * (1.0 / 144.0));
}

// global: HIGH_QUALITY=2, AVERAGE_QUALITY=1, LOW_QUALITY=0.
// shadowCommon: QUALITY=(shadowFilteringQuality-1), so average uses
// one comparison for filtering<=1 and four for filtering>1. This is not
// the menu's independent 0..5 shadow-resolution quality setting.
fn shadow_selected_filter(light_quality: i32, filtering_quality: i32) -> i32 {
    if light_quality == 2 { return filtering_quality; }
    if light_quality == 1 { return select(0, 1, filtering_quality > 1); }
    return 0;
}

fn sample_directional_shadow(map: texture_depth_2d_array, comparison: sampler_comparison,
    coordinates: vec4<f32>, slice: i32, size: vec2<f32>, light_quality: i32, filtering_quality: i32) -> f32 {
    // Original directional wrapper's unknown-quality result is ZERO, unlike
    // spot/omni. Do not merge the two defaults or reinterpret the quality.
    if light_quality < 0 || light_quality > 2 { return 0.0; }
    return shadow_pcf(map, comparison, coordinates.xyz, slice, size,
        shadow_selected_filter(light_quality, filtering_quality));
}

fn sample_local_shadow(map: texture_depth_2d_array, comparison: sampler_comparison,
    coordinates: vec4<f32>, slice: i32, size: vec2<f32>, light_quality: i32, filtering_quality: i32) -> f32 {
    if light_quality < 0 || light_quality > 2 { return 1.0; }
    return shadow_pcf(map, comparison, coordinates.xyz, slice, size,
        shadow_selected_filter(light_quality, filtering_quality));
}

fn paraboloid_direction(matrix: mat4x4<f32>, world_position: vec3<f32>) -> vec4<f32> {
    let delta = world_position - matrix[3].xyz;
    let light_space = vec3(dot(delta, matrix[0].xyz), dot(delta, matrix[1].xyz), dot(delta, matrix[2].xyz));
    let distance = length(light_space);
    var direction = vec3(0.0, 0.0, 0.01);
    if distance > 1e-8 { direction = light_space / distance; }
    return vec4(direction, distance);
}

fn paraboloid_projection(matrix: mat4x4<f32>, inverse_matrix: mat4x4<f32>, position: vec3<f32>) -> vec4<f32> {
    let d = paraboloid_direction(matrix, position);
    return vec4(d.xy / max(1e-4, 1.0 + d.z), d.w / inverse_matrix[0].z, sign(d.z));
}

fn paraboloid_shadow(map: texture_2d_array<f32>, bilinear: sampler, shadow: ShadowInfo, position: vec3<f32>, normal: vec3<f32>, use_normal_offset: bool, use_slope_scale: bool, normal_offset: f32) -> f32 {
    var p = position;
    if use_normal_offset {
        let light_dir = normalize(shadow.world_to_shadow[3].xyz - p);
        let scale = shadow_normal_scale(normal, light_dir, use_slope_scale);
        p += (normal_offset * scale.y) * normal;
    }
    let d = paraboloid_direction(shadow.world_to_shadow, p);
    let distance = d.w + shadow.shadow_to_world[2].z;
    let pb = d.xy / max(1e-4, 1.0 + d.z);
    let rect = shadow.shadow_to_world[1];
    let uv = rect.xy + clamp(0.5 * (1.0 + pb), vec2(0.0), vec2(1.0)) * rect.zw;
    let blocker = textureSampleLevel(map, bilinear, uv, shadow_slice(shadow.info), 0.0).x;
    let receiver = exp(shadow.shadow_to_world[0].x * (distance / shadow.shadow_to_world[0].z));
    return 1.0 - clamp(receiver * blocker, 0.0, 1.0);
}
