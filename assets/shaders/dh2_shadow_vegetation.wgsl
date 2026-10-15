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
    let phase = dot(position, vec3<f32>(color.y + 1.0));
    let time = p.local_time_depth_hack.x;
    let wave0 = fract((vec4<f32>(phase, phase, color.y, color.y) + vec4<f32>(time))
        * vec4<f32>(1.975, 0.793, 0.375, 0.193));
    let wave1 = fma(wave0, vec4<f32>(2.0), vec4<f32>(-1.0));
    let wave2 = fract(fma(wave1, vec4<f32>(p.leaf_frequency.x), vec4<f32>(0.5)));
    let wave3 = abs(fma(wave2, vec4<f32>(2.0), vec4<f32>(-1.0)));
    let smooth_wave = fma(-wave3, vec4<f32>(2.0), vec4<f32>(3.0)) * (wave3 * wave3);
    let summed_wave = vec2<f32>(smooth_wave.x + smooth_wave.y, smooth_wave.z + smooth_wave.w);
    let leaf = (max(p.leaf_min_wind_strength.x, 0.0) * p.leaf_amplitude.x) * color.z;
    let amplitudes = vec3<f32>(leaf * normal_xy.x,
        color.x * p.branch_amplitude.x, leaf * normal_xy.y);
    let bent = fma(vec3<f32>(summed_wave.x - 1.0, summed_wave.y, summed_wave.x - 1.0),
        amplitudes, position);
    let main_wave = sin(p.main_bending_frequency.x * time);
    let main_amplitude = fma(main_wave, p.main_bending_variance.x, p.main_bending_amplitude.x);
    let main_weight = fma(bent.z, main_amplitude, 1.0);
    let squared_weight = main_weight * main_weight;
    let main_bend = fma(squared_weight, squared_weight, -squared_weight);
    let displaced = vec3<f32>(fma(vec2<f32>(main_bend), vec2<f32>(0.0, 0.01), bent.xy), bent.z);
    // The native VS preserves the post-leaf-bending length, with no zero guard.
    return sqrt(dot(bent, bent)) * (inverseSqrt(dot(displaced, displaced)) * displaced);
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
