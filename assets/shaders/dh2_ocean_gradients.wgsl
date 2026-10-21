// FFTWaves/GenGradientsAndJacobian. Its installed shader fixes FFT size and
// patch length at 512 and 50; the wave owner must supply a 512-square texture.
struct GradientUniform { choppiness: vec4<f32>, }
@group(0) @binding(0) var<uniform> parameters: GradientUniform;
@group(0) @binding(1) var displacement: texture_2d<f32>;

@vertex fn fullscreen_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn gradients_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(position.xy);
    // PointWrap at pixel-centre UV position/512, with the source's texel offsets.
    let left = textureLoad(displacement, vec2<i32>((pixel.x + 511) % 512, pixel.y), 0).xyz;
    let right = textureLoad(displacement, vec2<i32>((pixel.x + 1) % 512, pixel.y), 0).xyz;
    let back = textureLoad(displacement, vec2<i32>(pixel.x, (pixel.y + 511) % 512), 0).xyz;
    let front = textureLoad(displacement, vec2<i32>(pixel.x, (pixel.y + 1) % 512), 0).xyz;
    let gradient = vec2<f32>(-(right.z - left.z), -(front.z - back.z));
    let scale = parameters.choppiness.x * (512.0 / 50.0);
    let dx = (right.xy - left.xy) * scale;
    let dy = (front.xy - back.xy) * scale;
    let jacobian = (1.0 + dx.x) * (1.0 + dy.y) - dx.y * dy.x;
    return vec4<f32>(gradient, 0.0, max(1.0 - jacobian, 0.0));
}
