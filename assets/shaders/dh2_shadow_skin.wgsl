// Rewrite transport of the installed 4/8-weight shadow VS inputs. Matrix
// offset is in joints; each joint owns three float4 rows. Native UNORM8
// weights are used without normalization. The multiply/MAD order is DXBC's.
struct Instance {
    offset: vec4<u32>,
    z: vec4<f32>, y: vec4<f32>, x: vec4<f32>, w: vec4<f32>,
    depth: vec4<f32>, uv: vec4<f32>,
}
@group(0) @binding(0) var<uniform> instance: Instance;
@group(1) @binding(0) var<storage, read> palette: array<vec4<f32>>;
@group(1) @binding(1) var diffuse: texture_2d<f32>;
@group(1) @binding(2) var alpha_sampler: sampler;
struct Input {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) indices0: vec4<u32>, @location(3) weights0: vec4<f32>,
    @location(4) indices1: vec4<u32>, @location(5) weights1: vec4<f32>,
}
struct Output {
    @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32>,
}
// D3D typed-buffer loads return zero outside the actual palette. Express
// that rule explicitly instead of depending on a backend's robust access.
fn skin_load(index:u32)->vec4<f32> {
    if index < arrayLength(&palette) {return palette[index];}
    return vec4<f32>(0.0);
}
fn skin_row(input: Input, row: u32) -> vec4<f32> {
    let i = (input.indices0 + vec4<u32>(instance.offset.x)) * 3u + vec4<u32>(row);
    var value = skin_load(i.y) * input.weights0.y;
    value = fma(skin_load(i.x), vec4<f32>(input.weights0.x), value);
    value = fma(skin_load(i.z), vec4<f32>(input.weights0.z), value);
    value = fma(skin_load(i.w), vec4<f32>(input.weights0.w), value);
    if SKIN_INFLUENCES == 8u {
        let j = (input.indices1 + vec4<u32>(instance.offset.x)) * 3u + vec4<u32>(row);
        value = fma(skin_load(j.x), vec4<f32>(input.weights1.x), value);
        value = fma(skin_load(j.y), vec4<f32>(input.weights1.y), value);
        value = fma(skin_load(j.z), vec4<f32>(input.weights1.z), value);
        value = fma(skin_load(j.w), vec4<f32>(input.weights1.w), value);
    }
    return value;
}
fn skin_position(input: Input) -> vec4<f32> {
    let p = vec4<f32>(input.position, 1.0);
    let q = vec4<f32>(dot(skin_row(input, 0u), p), dot(skin_row(input, 1u), p), dot(skin_row(input, 2u), p), 1.0);
    return vec4<f32>(dot(instance.x, q), dot(instance.y, q), dot(instance.z, q) * instance.depth.x, dot(instance.w, q));
}
@vertex fn vertex(input: Input) -> Output {
    var output: Output;
    output.clip = skin_position(input);
    output.uv = input.uv;
    return output;
}
fn bayer2(p: vec2<u32>) -> u32 {
    return (2u * p.y + p.x + 1u) & 3u;
}
fn skin_dither_survives(alpha:f32,pixel:vec2<u32>)->bool {
        let p = pixel & vec2<u32>(7u);
        let b = 16u * bayer2(p & vec2<u32>(1u)) + 4u * bayer2((p >> vec2<u32>(1u)) & vec2<u32>(1u)) + bayer2(p >> vec2<u32>(2u));
        let threshold = (1.0 / 17.0) * fma(f32(b), 1.0 / 64.0, 1.0);
        let a = clamp(alpha * (1.0/3.0), 0.0, 1.0);
        return !(fma(fma(a, -2.0, 3.0), a * a, -threshold) < 0.0);
}
@fragment fn fragment(input: Output) {
    if SKIN_DITHER {
        if !skin_dither_survives(textureSample(diffuse,alpha_sampler,input.uv).a,vec2<u32>(input.clip.xy)) {discard;}
    } else {
        let uv = fma(input.uv, instance.uv.xy, instance.uv.zw);
        if textureSample(diffuse, alpha_sampler, uv).a < 0.5 { discard; }
    }
}
