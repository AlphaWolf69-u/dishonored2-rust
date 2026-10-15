// Original PostFX_DWAOBlur, selected by FUN_14032c7f0's two blur draws.
// Concatenate after dh2_screen_shadows.wgsl. Unlike the other bilateralBlur
// program, this rejects depth-discontinuous taps, clamps point samples at
// edges, and preserves packed depth in Z/W while blurring BOTH AO and shadow.
struct NativeDwaoBlurUniform {
    offset0: vec4<f32>,
    offset1: vec4<f32>,
    // Original DWAO/BlurParams.x; centre weight is unused by this shader.
    parameters: vec4<f32>,
}
@group(1) @binding(0) var<uniform> dwao_blur: NativeDwaoBlurUniform;

fn native_unpack_depth_2byte(value: vec2<f32>) -> f32 {
    // global::unpack_2byte, not an IEEE half/UNORM16 substitute.
    return dot(value, vec2(255.0 * 256.0, 255.0) / 257.0);
}

struct DwaoVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) offset0: vec4<f32>,
    @location(2) offset1: vec4<f32>,
}
@vertex fn screen_dwao_vertex(@builtin(vertex_index) index: u32) -> DwaoVertex {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: DwaoVertex;
    out.position = vec4(uv * vec2(2.0, -2.0) + vec2(-1.0, 1.0), 0.0, 1.0);
    out.uv = uv * screen.depth_scale_step_hack.xy;
    // Preserve the original vertex-stage offset/interpolation ordering.
    out.offset0 = out.uv.xyxy + dwao_blur.offset0;
    out.offset1 = out.uv.xyxy + dwao_blur.offset1;
    return out;
}

@fragment fn screen_dwao_blur(input: DwaoVertex) -> @location(0) vec4<f32> {
    let a = textureSampleLevel(screen_source, screen_point, input.offset0.xy, 0.0);
    let b = textureSampleLevel(screen_source, screen_point, input.offset0.zw, 0.0);
    let center = textureSampleLevel(screen_source, screen_point, input.uv, 0.0);
    let c = textureSampleLevel(screen_source, screen_point, input.offset1.xy, 0.0);
    let d = textureSampleLevel(screen_source, screen_point, input.offset1.zw, 0.0);
    let depth = native_unpack_depth_2byte(center.zw);
    let neighbor_depth = vec4(native_unpack_depth_2byte(a.zw), native_unpack_depth_2byte(b.zw),
        native_unpack_depth_2byte(c.zw), native_unpack_depth_2byte(d.zw));
    let rejected = abs(vec4(depth) - neighbor_depth) > vec4(dwao_blur.parameters.x * depth);
    let ao = select(vec4(a.x, b.x, c.x, d.x), vec4(center.x), rejected);
    let shadow = select(vec4(a.y, b.y, c.y, d.y), vec4(center.y), rejected);
    let weights = vec4(0.152469158, 0.221841291, 0.221841291, 0.152469158);
    return vec4(dot(ao, weights) + center.x * 0.251379132,
        dot(shadow, weights) + center.y * 0.251379132, center.zw);
}
