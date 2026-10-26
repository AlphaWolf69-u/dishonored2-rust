// GDraw draws on UNORM targets with UNORM textures (formats 0x1c, 0x47,
// 0x62 in FUN_1402e2c00): its colours and blending are in gamma space. The
// view target holds linear colour that is sRGB-encoded on output, so the
// scene moves into a gamma-space canvas for the UI and back afterwards.
@group(0) @binding(0) var source: texture_2d<f32>;

@vertex
fn transfer_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One triangle over the target.
    let corner = vec2(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn encode(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

fn decode(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn copy_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(source, vec2<i32>(position.xy), 0);
}

// The view target holds linear colour; the canvas keeps the encoding.
@fragment
fn encode_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(source, vec2<i32>(position.xy), 0);
    return vec4(encode(c.rgb), c.a);
}

// Encoded on output, the canvas colour is shown unchanged.
@fragment
fn decode_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let c = textureLoad(source, vec2<i32>(position.xy), 0);
    return vec4(decode(c.rgb), c.a);
}
