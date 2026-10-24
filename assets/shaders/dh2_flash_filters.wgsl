// GDraw D3D11 filter passes over a filter layer's render target.
// Tap blur (pixel shaders at exe 0x22a8eb0 and siblings): the sum of
// weight * sample(clamp(uv + offset, clamp.xy, clamp.zw)).
// Colour matrix (exe 0x22a95d0): on the premultiplied texel c,
// rgb_i = dot(row_i, c) + offset_i * c.a, then rgb and a scaled by row_3.w.
struct Pass {
    // Destination rectangle in NDC (x0, y0, x1, y1) and the source
    // coordinates of its corners.
    destination: vec4<f32>,
    uv: vec4<f32>,
    clamp_rect: vec4<f32>,
    // x: tap count.
    params: vec4<u32>,
    taps: array<vec4<f32>, 18>,
    rows: array<vec4<f32>, 5>,
};
@group(0) @binding(0) var<uniform> pass_data: Pass;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var linear_clamp: sampler;

struct VertexOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn filter_vertex(@builtin(vertex_index) index: u32) -> VertexOut {
    // Two triangles over the destination rectangle.
    let corners = array<vec2<f32>, 6>(vec2(0.0, 0.0), vec2(1.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 0.0), vec2(1.0, 1.0), vec2(0.0, 1.0));
    let t = corners[index];
    var out: VertexOut;
    out.clip = vec4(mix(pass_data.destination.xy, pass_data.destination.zw, t), 0.0, 1.0);
    out.uv = mix(pass_data.uv.xy, pass_data.uv.zw, t);
    return out;
}

@fragment
fn blur_fragment(in: VertexOut) -> @location(0) vec4<f32> {
    var sum = vec4(0.0);
    for (var i = 0u; i < pass_data.params.x; i = i + 1u) {
        let tap = pass_data.taps[i];
        let uv = min(max(in.uv + tap.xy, pass_data.clamp_rect.xy), pass_data.clamp_rect.zw);
        sum = sum + textureSampleLevel(source, linear_clamp, uv, 0.0) * tap.z;
    }
    return sum;
}

@fragment
fn matrix_fragment(in: VertexOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(source, linear_clamp, in.uv, 0.0);
    let r = dot(pass_data.rows[0], c) + pass_data.rows[4].x * c.w;
    let g = dot(pass_data.rows[1], c) + pass_data.rows[4].y * c.w;
    let b = dot(pass_data.rows[2], c) + pass_data.rows[4].z * c.w;
    return vec4(vec3(r, g, b) * pass_data.rows[3].w, c.w * pass_data.rows[3].w);
}
