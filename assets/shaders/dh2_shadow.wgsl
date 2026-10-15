// Original shadow/shadowCommon/shadow_paraboloid renderprog algorithms.
// Matrices contain the ORIGINAL row vectors as WGSL columns; no matrix guessing.
// Geometry/atlas allocation and the original bias/settings producer remain external.
// Cloud attenuation additionally requires the original atmosphere/intersection path.
struct NativeShadowInfo {
    world_to_shadow: mat4x4<f32>,
    shadow_to_world: mat4x4<f32>,
    info: u32,
    unused_tail_a: u32,
    unused_tail_b: u32,
    unused_tail_c: u32,
}

fn native_shadow_slice(info: u32) -> i32 { return i32(info & 31u); }

fn native_omni_shadow_side(direction: vec3<f32>) -> u32 {
    let a = abs(direction);
    if a.x > a.y {
        if a.x > a.z { return select(4u, 5u, direction.x > 0.0); }
    } else if a.y > a.z { return select(0u, 1u, direction.y > 0.0); }
    return select(2u, 3u, direction.z > 0.0);
}

fn native_spot_shadow_side(to_light: vec3<f32>, left: vec3<f32>) -> u32 {
    return select(1u, 0u, dot(to_light, left) > 0.0);
}

fn native_parallel_shadow_slice(position: vec3<f32>, view_row2: vec4<f32>, fars: vec4<f32>) -> u32 {
    let z = -dot(vec4(position, 1.0), view_row2);
    let mask = step(fars, vec4(z));
    return u32(mask.x + mask.y + mask.z + mask.w);
}

fn native_shadow_position(matrix: mat4x4<f32>, position: vec3<f32>, directional: bool) -> vec4<f32> {
    let p = vec4(position, 1.0);
    if directional { return vec4(dot(matrix[0], p), dot(matrix[1], p), dot(matrix[2], p), 1.0); }
    return matrix * p;
}

fn native_uv_normal_offset(matrix: mat4x4<f32>, position: vec3<f32>, offset: vec4<f32>, directional: bool) -> vec4<f32> {
    var p = native_shadow_position(matrix, position, directional);
    p.x = offset.x;
    p.y = offset.y;
    return p;
}

fn native_shadow_normal_scale(normal: vec3<f32>, light_dir: vec3<f32>, slope_scale: bool) -> vec2<f32> {
    if slope_scale {
        let cosine = dot(normalize(light_dir), normal);
        return vec2(cosine, clamp(1.0 - cosine, 0.0, 1.0));
    }
    return vec2(0.0, 1.0);
}

// Arguments represent original compile-time options, not invented defaults.
// shadowCommon's uvNormalOffset call has an omitted directional argument; its
// compiled caller still needs verification. It is deliberately NOT enabled here.
fn native_shadow_offset(matrix: mat4x4<f32>, position: vec3<f32>, normal: vec3<f32>, light_dir: vec3<f32>, directional: bool,
    use_normal_offset: bool, use_slope_scale: bool, use_slope_bias: bool, post_projection: bool,
    normal_offset: f32, bias: f32, slope_bias: f32) -> vec4<f32> {
    let scale = native_shadow_normal_scale(normal, light_dir, use_slope_scale);
    var world_position = position;
    if use_normal_offset { world_position += normal * normal_offset * scale.y; }
    var p = native_shadow_position(matrix, world_position, directional);
    var z_bias = bias;
    if use_slope_bias {
        var slope = 0.0;
        if post_projection {
            var tangent = light_dir - normal * dot(normal, light_dir);
            tangent = normalize(tangent);
            let q = native_shadow_position(matrix, position + tangent, directional);
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

fn native_shadow_tap(map: texture_depth_2d_array, comparison: sampler_comparison, base: vec2<f32>, offset: vec2<f32>, inv_size: vec2<f32>, slice: i32, depth: f32) -> f32 {
    return textureSampleCompareLevel(map, comparison, base + offset * inv_size, slice, depth);
}

// Original optimized 2x2/3x3/5x5 PCF uses 1/4/9 hardware-comparison samples.
// Keep accumulation order and normalizations; don't replace with a generic grid.
fn native_shadow_pcf(map: texture_depth_2d_array, comparison: sampler_comparison, position: vec3<f32>, slice: i32, size: vec2<f32>, quality: i32) -> f32 {
    if quality == 0 { return clamp(textureSampleCompareLevel(map, comparison, position.xy, slice, position.z), 0.0, 1.0); }
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
        sum += uw0 * vw0 * native_shadow_tap(map, comparison, base_uv, vec2(u0, v0), inv_size, slice, position.z);
        sum += uw1 * vw0 * native_shadow_tap(map, comparison, base_uv, vec2(u1, v0), inv_size, slice, position.z);
        sum += uw0 * vw1 * native_shadow_tap(map, comparison, base_uv, vec2(u0, v1), inv_size, slice, position.z);
        sum += uw1 * vw1 * native_shadow_tap(map, comparison, base_uv, vec2(u1, v1), inv_size, slice, position.z);
        return clamp(sum * (1.0 / 16.0), 0.0, 1.0);
    }
    let uw0 = 4.0 - 3.0 * s; let uw1 = 7.0; let uw2 = 1.0 + 3.0 * s;
    let u0 = (3.0 - 2.0 * s) / uw0 - 2.0; let u1 = (3.0 + s) / uw1; let u2 = s / uw2 + 2.0;
    let vw0 = 4.0 - 3.0 * t; let vw1 = 7.0; let vw2 = 1.0 + 3.0 * t;
    let v0 = (3.0 - 2.0 * t) / vw0 - 2.0; let v1 = (3.0 + t) / vw1; let v2 = t / vw2 + 2.0;
    sum += uw0 * vw0 * native_shadow_tap(map, comparison, base_uv, vec2(u0, v0), inv_size, slice, position.z);
    sum += uw1 * vw0 * native_shadow_tap(map, comparison, base_uv, vec2(u1, v0), inv_size, slice, position.z);
    sum += uw2 * vw0 * native_shadow_tap(map, comparison, base_uv, vec2(u2, v0), inv_size, slice, position.z);
    sum += uw0 * vw1 * native_shadow_tap(map, comparison, base_uv, vec2(u0, v1), inv_size, slice, position.z);
    sum += uw1 * vw1 * native_shadow_tap(map, comparison, base_uv, vec2(u1, v1), inv_size, slice, position.z);
    sum += uw2 * vw1 * native_shadow_tap(map, comparison, base_uv, vec2(u2, v1), inv_size, slice, position.z);
    sum += uw0 * vw2 * native_shadow_tap(map, comparison, base_uv, vec2(u0, v2), inv_size, slice, position.z);
    sum += uw1 * vw2 * native_shadow_tap(map, comparison, base_uv, vec2(u1, v2), inv_size, slice, position.z);
    sum += uw2 * vw2 * native_shadow_tap(map, comparison, base_uv, vec2(u2, v2), inv_size, slice, position.z);
    return clamp(sum * (1.0 / 144.0), 0.0, 1.0);
}

// global: HIGH_QUALITY=2, AVERAGE_QUALITY=1, LOW_QUALITY=0.
// shadowCommon: QUALITY=(shadowFilteringQuality-1), so average uses
// one comparison for filtering<=1 and four for filtering>1. This is not
// the menu's independent 0..5 shadow-resolution quality setting.
fn native_shadow_selected_filter(light_quality: i32, filtering_quality: i32) -> i32 {
    if light_quality == 2 { return filtering_quality; }
    if light_quality == 1 { return select(0, 1, filtering_quality > 1); }
    return 0;
}

fn native_sample_directional_shadow(map: texture_depth_2d_array, comparison: sampler_comparison,
    coordinates: vec4<f32>, slice: i32, size: vec2<f32>, light_quality: i32, filtering_quality: i32) -> f32 {
    // Original directional wrapper's unknown-quality result is ZERO, unlike
    // spot/omni. Do not merge the two defaults or reinterpret the quality.
    if light_quality < 0 || light_quality > 2 { return 0.0; }
    return native_shadow_pcf(map, comparison, coordinates.xyz, slice, size,
        native_shadow_selected_filter(light_quality, filtering_quality));
}

fn native_sample_local_shadow(map: texture_depth_2d_array, comparison: sampler_comparison,
    coordinates: vec4<f32>, slice: i32, size: vec2<f32>, light_quality: i32, filtering_quality: i32) -> f32 {
    if light_quality < 0 || light_quality > 2 { return 1.0; }
    return native_shadow_pcf(map, comparison, coordinates.xyz, slice, size,
        native_shadow_selected_filter(light_quality, filtering_quality));
}

fn native_paraboloid_direction(matrix: mat4x4<f32>, world_position: vec3<f32>) -> vec4<f32> {
    let delta = world_position - matrix[3].xyz;
    let light_space = vec3(dot(delta, matrix[0].xyz), dot(delta, matrix[1].xyz), dot(delta, matrix[2].xyz));
    let distance = length(light_space);
    var direction = vec3(0.0, 0.0, 0.01);
    if distance > 1e-8 { direction = light_space / distance; }
    return vec4(direction, distance);
}

fn native_paraboloid_projection(matrix: mat4x4<f32>, inverse_matrix: mat4x4<f32>, position: vec3<f32>) -> vec4<f32> {
    let d = native_paraboloid_direction(matrix, position);
    return vec4(d.xy / max(1e-4, 1.0 + d.z), d.w / inverse_matrix[0].z, sign(d.z));
}

fn native_paraboloid_shadow(map: texture_2d_array<f32>, bilinear: sampler, shadow: NativeShadowInfo, position: vec3<f32>, normal: vec3<f32>, use_normal_offset: bool, use_slope_scale: bool, normal_offset: f32) -> f32 {
    var p = position;
    if use_normal_offset {
        let light_dir = normalize(shadow.world_to_shadow[3].xyz - p);
        let scale = native_shadow_normal_scale(normal, light_dir, use_slope_scale);
        p += (normal_offset * scale.y) * normal;
    }
    let d = native_paraboloid_direction(shadow.world_to_shadow, p);
    let distance = d.w + shadow.shadow_to_world[2].z;
    let pb = d.xy / max(1e-4, 1.0 + d.z);
    let rect = shadow.shadow_to_world[1];
    let uv = rect.xy + clamp(0.5 * (1.0 + pb), vec2(0.0), vec2(1.0)) * rect.zw;
    let blocker = textureSampleLevel(map, bilinear, uv, native_shadow_slice(shadow.info), 0.0).x;
    let receiver = exp(shadow.shadow_to_world[0].x * (distance / shadow.shadow_to_world[0].z));
    return 1.0 - clamp(receiver * blocker, 0.0, 1.0);
}
