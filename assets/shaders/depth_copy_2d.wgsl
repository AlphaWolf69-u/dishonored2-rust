// Original RenderDoc dh2_frame82270: VS 6b16fa07-b6bbd755-2aab9dc8-05c7ae0b,
// identity PS 94d6f085-904e7f2d-f54010ff-cb5cf381 (event25931),
// downscale PS e15952ed-f0b97971-27dff39c-045f282b (event25946).
// These are 2D input shaders, not the array-input depthcopy.decl variant.
@group(0) @binding(0) var source: texture_depth_2d;
@group(0) @binding(1) var point_clamp: sampler;
// unhackdepthbuffer.decl: modelDepthHack.x comes from the actual callback
// (reciprocal of its positive depth-hack value, otherwise 1).
@group(0) @binding(2) var<uniform> model_depth_hack: vec4<f32>;
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}
@vertex fn vertex(@location(0) position: vec3<f32>, @location(1) uv: vec2<f32>) -> VertexOutput {
    var output: VertexOutput;
    output.position = vec4<f32>(position, 1.0);
    output.uv = uv;
    return output;
}
@fragment fn identity(input: VertexOutput) -> @builtin(frag_depth) f32 {
    return textureSampleLevel(source, point_clamp, input.uv, 0);
}
@fragment fn unhack(input: VertexOutput) -> @builtin(frag_depth) f32 {
    return textureSampleLevel(source, point_clamp, input.uv, 0) * model_depth_hack.x;
}
@fragment fn downscale(input: VertexOutput) -> @builtin(frag_depth) f32 {
    let four = textureGather(source, point_clamp, input.uv);
    let pair = min(four.zw, four.xy);
    return min(pair.y, pair.x);
}
