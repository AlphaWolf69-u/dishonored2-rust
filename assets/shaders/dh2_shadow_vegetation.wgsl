// Installed VS14200/PS14201 (arkvegetation shadow role), not forward wind.
// CB2 is 208 bytes: actual per-material wind parameters, model MVP Z/Y/X/W,
// local time and depth hack. The owner supplies original LOCAL positions,
// original vertex color.xyz and original packed-normal.xy after decoding.
struct VegetationShadowCB {
    layer0_scale_bias: vec4<f32>,
    main_bending_variance: vec4<f32>,
    main_bending_frequency: vec4<f32>,
    main_bending_amplitude: vec4<f32>,
    leaf_min_wind_strength: vec4<f32>,
    leaf_frequency: vec4<f32>,
    leaf_amplitude: vec4<f32>,
    branch_amplitude: vec4<f32>,
    mvp_z: vec4<f32>,
    mvp_y: vec4<f32>,
    mvp_x: vec4<f32>,
    mvp_w: vec4<f32>,
    local_time_depth_hack: vec4<f32>,
}
@group(0) @binding(0) var<uniform> per_instance: VegetationShadowCB;
@group(1) @binding(0) var diffuse: texture_2d<f32>;
@group(1) @binding(1) var bilinear_sampler: sampler;

fn vegetation_shadow_position(p: VegetationShadowCB, position: vec3<f32>,
    normal_xy: vec2<f32>, color: vec3<f32>) -> vec3<f32>
{
    let parameters = NativeShadowWindParameters(p.main_bending_variance,
        p.main_bending_frequency, p.main_bending_amplitude, p.leaf_min_wind_strength,
        p.leaf_frequency, p.leaf_amplitude, p.branch_amplitude);
    // This rigid ABI's actual VS has fixed local wind, unlike world instances.
    return native_shadow_wind_position(parameters, p.local_time_depth_hack.x,
        vec2<f32>(0.0, 0.01), max(p.leaf_min_wind_strength.x, 0.0),
        position, normal_xy, color);
}

fn vegetation_shadow_clip(p: VegetationShadowCB, position: vec3<f32>,
    normal_xy: vec2<f32>, color: vec3<f32>) -> vec4<f32>
{
    let bent = vec4<f32>(vegetation_shadow_position(p, position, normal_xy, color), 1.0);
    return vec4<f32>(dot(bent, p.mvp_x), dot(bent, p.mvp_y),
        dot(bent, p.mvp_z) * p.local_time_depth_hack.z, dot(bent, p.mvp_w));
}

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec3<f32>,
    @location(3) normal_xy: vec2<f32>,
}
struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vertex(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.clip = vegetation_shadow_clip(per_instance, input.position, input.normal_xy, input.color);
    output.uv = input.uv;
    return output;
}
@fragment
fn fragment(input: VertexOutput) {
    let uv = fma(input.uv, per_instance.layer0_scale_bias.xy, per_instance.layer0_scale_bias.zw);
    if textureSample(diffuse, bilinear_sampler, uv).a < 0.5 {
        discard;
    }
}
