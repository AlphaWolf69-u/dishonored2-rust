// Iggy display-list triangles (GDraw equivalent) over the display output.
struct Stage {
    // Stage size in movie pixels, then viewport scale and offset in NDC.
    size: vec2<f32>,
    scale: vec2<f32>,
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
    let ndc = vec2(v.position.x / stage.size.x * 2.0 - 1.0, 1.0 - v.position.y / stage.size.y * 2.0);
    out.clip = vec4(ndc * stage.scale, 0.0, 1.0);
    out.uv = v.uv;
    out.mul = v.mul;
    out.add = v.add;
    out.params = v.params;
    return out;
}

// Sampling modes: 0 texture coordinates, 1 radial gradient, 2 focal
// gradient (params.y = focal point); 3 marks a premultiplied ramp texture.
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
    var texel = textureSample(fill_texture, fill_sampler, uv);
    if mode != 0u && texel.a > 0.0 {
        // Gradient ramps are premultiplied (FUN_1410947c0).
        texel = vec4(texel.rgb / texel.a, texel.a);
    }
    return clamp(texel * in.mul + in.add, vec4(0.0), vec4(1.0));
}
