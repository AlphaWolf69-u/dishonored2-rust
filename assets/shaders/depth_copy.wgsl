// Original generated/decls/renderprog/depthcopy.decl: point-sampled depth,
// with vertex UV scale/bias. Array slice is supplied as an integer image layer.
// This shader alone does not establish the depthAlpha frame-graph binding.
struct CopyParameters {
    scale_bias: vec4<f32>,
    layer: i32,
    _padding0: u32,
    _padding1: u32,
    _padding2: u32,
}
@group(0) @binding(0) var source: texture_depth_2d_array;
@group(0) @binding(1) var point_sampler: sampler;
@group(0) @binding(2) var<uniform> parameters: CopyParameters;
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 1.0);
    output.uv = uv * parameters.scale_bias.xy + parameters.scale_bias.zw;
    return output;
}
@fragment fn fragment(input: VertexOutput) -> @builtin(frag_depth) f32 {
    return textureSampleLevel(source, point_sampler, input.uv, parameters.layer, 0);
}
