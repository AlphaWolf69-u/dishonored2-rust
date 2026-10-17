// Original vertex.decl TransformVertex, useMultiInstancing=1, isShadow=1:
// remap (instanceBufferOffset + SV_InstanceID) through the low 24 bits,
// read MatrixInstanceType's three float4 rows, then use viewProjectionMatrix.
// No rigid-model depth hack belongs to this branch.
// The host Mesh::ATTRIBUTE_POSITION has game_to_view applied; undo only that
// exact signed-axis permutation before the original local-to-world transform.
struct ViewState {
    w: vec4<f32>,
    z: vec4<f32>,
    y: vec4<f32>,
    x: vec4<f32>,
    instance_buffer_offset: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    uv_scale_bias: vec4<f32>,
};
struct MatrixInstance {
    x: vec4<f32>,
    y: vec4<f32>,
    z: vec4<f32>,
};
@group(0) @binding(0) var<uniform> state: ViewState;
@group(1) @binding(0) var<storage, read> visible_indices: array<u32>;
@group(1) @binding(1) var<storage, read> matrices: array<MatrixInstance>;

fn world_shadow_position(host_position: vec3<f32>, instance_id: u32) -> vec4<f32> {
    let local = vec4<f32>(host_position.x, -host_position.z, host_position.y, 1.0);
    let index = visible_indices[state.instance_buffer_offset + instance_id] & 0x00ffffffu;
    let model = matrices[index];
    let world = vec4<f32>(dot(local, model.x), dot(local, model.y), dot(local, model.z), 1.0);
    return native_shadow_world_project(world, state.x, state.y, state.z, state.w);
}

@vertex
fn vertex(@location(0) position: vec3<f32>, @builtin(instance_index) instance: u32)
    -> @builtin(position) vec4<f32> {
    return world_shadow_position(position, instance);
}

// Actual installed cutout pair: VS63aba4ead9c2c3aab263a36ae8ed362c
// passes raw UV; PS2cdcb121514df898807997ea08c1d694 applies layer0 MAD
// after interpolation and discards strictly below 0.5. No forward alpha tint.
@group(1) @binding(2) var diffuse: texture_2d<f32>;
@group(1) @binding(3) var bilinear_sampler: sampler;
struct AlphaOutput {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};
@vertex
fn vertex_alpha(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @builtin(instance_index) instance: u32) -> AlphaOutput {
    var output: AlphaOutput;
    output.clip = world_shadow_position(position, instance);
    output.uv = uv;
    return output;
}
// Installed Vista VSdff3e1d3ecb4d96f7ce676a352631c44 /
// PS1a8eaf2045c70c568c1edb697b3dd21b: only raw UV.xy is consumed.
// Its declared anisotropicsampler is bound here, not the bilinear sampler.
@fragment
fn fragment_vista(input: AlphaOutput) {
    if textureSample(diffuse, bilinear_sampler, input.uv).a < 0.5 {
        discard;
    }
}

@fragment
fn fragment(input: AlphaOutput) {
    let uv = fma(input.uv, state.uv_scale_bias.xy, state.uv_scale_bias.zw);
    if textureSample(diffuse, bilinear_sampler, uv).a < 0.5 {
        discard;
    }
}
