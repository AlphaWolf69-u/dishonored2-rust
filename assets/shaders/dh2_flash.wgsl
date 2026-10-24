// Iggy display-list triangles (GDraw equivalent) over the display output.
struct Stage {
    // Stage pixels to NDC: ndc = position * scale + offset (the display
    // target, or a filter layer's render target).
    scale: vec2<f32>,
    offset: vec2<f32>,
};
@group(0) @binding(0) var<uniform> stage: Stage;
@group(1) @binding(0) var fill_texture: texture_2d<f32>;
@group(1) @binding(1) var fill_sampler: sampler;

struct VertexIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) mul: vec4<f32>,
    @location(3) add: vec4<f32>,
    @location(4) params: vec2<f32>,
    // Perspective divisor of 3D content (1 in 2D).
    @location(5) w: f32,
};
struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) mul: vec4<f32>,
    @location(2) add: vec4<f32>,
    @location(3) params: vec2<f32>,
};

@vertex
fn flash_vertex(v: VertexIn) -> VertexOut {
    var out: VertexOut;
    let ndc = v.position * stage.scale + stage.offset;
    // Homogeneous w gives perspective-correct interpolation like GDraw's
    // o1 = uv / r.z.
    out.clip = vec4(ndc * v.w, 0.0, v.w);
    out.uv = v.uv;
    out.mul = v.mul;
    out.add = v.add;
    out.params = v.params;
    return out;
}

// Sampling modes: 0 texture coordinates, 1 radial gradient, 2 focal
// gradient (params.y = focal point); 3 texture coordinates into a ramp.
@fragment
fn flash_fragment(in: VertexOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    let mode = u32(in.params.x + 0.5);
    if mode == 1u {
        uv = vec2(length(in.uv), 0.5);
    } else if mode == 2u {
        let f = in.params.y;
        let d = in.uv - vec2(f, 0.0);
        let a = dot(d, d);
        let b = 2.0 * f * d.x;
        let c = f * f - 1.0;
        let t = (-b + sqrt(max(b * b - 4.0 * a * c, 0.0))) / max(2.0 * a, 1e-9);
        uv = vec2(1.0 / max(t, 1e-9), 0.5);
    }
    // Bitmaps, ramps (FUN_1410947c0) and layer targets are premultiplied.
    // GDraw's textured shader (exe 0x229ce80): the multiplier with its rgb
    // scaled by its alpha, the offset scaled by the result's alpha, rgb no
    // more than alpha; blended ONE, INV_SRC_ALPHA.
    let texel = textureSample(fill_texture, fill_sampler, uv);
    let r = texel * vec4(in.mul.rgb * in.mul.a, in.mul.a);
    return vec4(min(vec3(r.a), r.rgb + in.add.rgb * r.a), r.a);
}
