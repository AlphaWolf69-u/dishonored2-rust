// Installed arkvegetation world-shadow pair dfe9d6cb (63-slot VS).
// Unlike rigid VS14200, the instance's current wind and normalized model
// rows supply the bending direction/strength. No hardcoded wind substitute.
struct WorldVegetationShadowCB {
    w: vec4<f32>,
    z: vec4<f32>,
    y: vec4<f32>,
    x: vec4<f32>,
    uv_scale_bias: vec4<f32>,
    parameters: VegetationAnimation,
    local_time: vec2<f32>,
    instance_buffer_offset: u32,
    wind_buffer_offset: u32,
}
struct MatrixInstance {
    x: vec4<f32>,
    y: vec4<f32>,
    z: vec4<f32>,
}
struct WindInstance {
    previous: vec4<f32>,
    current: vec4<f32>,
}
@group(0) @binding(0) var<uniform> state: WorldVegetationShadowCB;
@group(1) @binding(0) var<storage, read> visible_indices: array<u32>;
@group(1) @binding(1) var<storage, read> matrices: array<MatrixInstance>;
@group(1) @binding(2) var diffuse: texture_2d<f32>;
@group(1) @binding(3) var bilinear_sampler: sampler;
@group(1) @binding(4) var<storage, read> winds: array<WindInstance>;

fn world_vegetation_shadow_position(local_position: vec3<f32>, normal_xy: vec2<f32>,
    color: vec3<f32>, instance_id: u32) -> vec4<f32>
{
    let index = visible_indices[state.instance_buffer_offset + instance_id] & 0x00ffffffu;
    let model = matrices[index];
    let wind = winds[state.wind_buffer_offset + instance_id].current;
    let row_x = inverseSqrt(dot(model.x.xyz, model.x.xyz)) * model.x.xyz;
    let row_y = inverseSqrt(dot(model.y.xyz, model.y.xyz)) * model.y.xyz;
    let local_wind = vec2<f32>(dot(row_x, wind.xyz), dot(row_y, wind.xyz)) * wind.w;
    let bent = vec4<f32>(animate_vegetation(state.parameters, state.local_time.x,
        local_wind, max(wind.w, state.parameters.leaf_minimum.x),
        local_position, normal_xy, color), 1.0);
    let world = vec4<f32>(dot(bent, model.x), dot(bent, model.y), dot(bent, model.z), 1.0);
    return shadow_world_project(world, state.x, state.y, state.z, state.w);
}
// The other installed world variant (51-slot VS 11d8d0a6...) really has
// fixed local wind and NO wind-buffer binding. It is not a fallback for the
// 63-slot variant: selection is by the exact resident bytecode pair.
fn world_fixed_vegetation_shadow_position(position: vec3<f32>, normal_xy: vec2<f32>,
    color: vec3<f32>, instance_id: u32) -> vec4<f32>
{
    let index = visible_indices[state.instance_buffer_offset + instance_id] & 0x00ffffffu;
    let model = matrices[index];
    let bent = vec4<f32>(animate_vegetation(state.parameters, state.local_time.x,
        vec2<f32>(0.0, 0.01), max(state.parameters.leaf_minimum.x, 0.0),
        position, normal_xy, color), 1.0);
    let world = vec4<f32>(dot(bent, model.x), dot(bent, model.y), dot(bent, model.z), 1.0);
    return shadow_world_project(world, state.x, state.y, state.z, state.w);
}
struct VertexOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex
fn vertex(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) color: vec3<f32>, @location(3) normal_xy: vec2<f32>,
    @builtin(instance_index) instance: u32) -> VertexOutput
{
    var output: VertexOutput;
    output.clip = world_vegetation_shadow_position(position, normal_xy, color, instance);
    output.uv = uv;
    return output;
}
@vertex
fn vertex_fixed(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) color: vec3<f32>, @location(3) normal_xy: vec2<f32>,
    @builtin(instance_index) instance: u32) -> VertexOutput
{
    var output: VertexOutput;
    output.clip = world_fixed_vegetation_shadow_position(position, normal_xy, color, instance);
    output.uv = uv;
    return output;
}
@fragment
fn fragment(input: VertexOutput) {
    let uv = fma(input.uv, state.uv_scale_bias.xy, state.uv_scale_bias.zw);
    if textureSample(diffuse, bilinear_sampler, uv).a < 0.5 {
        discard;
    }
}
