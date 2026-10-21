// arkFFT/Radix008A_CS, Radix008A_CS2 and Utility from installed declarations.
struct FftUniform {
    // thread_count, ostride, istride, pstride.
    strides: vec4<u32>,
    phase_base: vec4<f32>,
}
@group(0) @binding(0) var<uniform> parameters: FftUniform;
@group(0) @binding(1) var<storage, read> source: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read_write> destination: array<vec2<f32>>;

fn butterfly(a: vec2<f32>, b: vec2<f32>) -> mat2x2<f32> {
    return mat2x2<f32>(a + b, a - b);
}
fn update_forward(a: vec2<f32>, b: vec2<f32>) -> mat2x2<f32> {
    return mat2x2<f32>(vec2<f32>(a.x + b.y, a.y - b.x),
        vec2<f32>(a.x - b.y, a.y + b.x));
}
fn multiply_complex(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}
fn forward_eight(input_values: array<vec2<f32>, 8>) -> array<vec2<f32>, 8> {
    var d = input_values;
    for (var i = 0u; i < 4u; i++) {
        let pair = butterfly(d[i], d[i + 4u]);
        d[i] = pair[0];
        d[i + 4u] = pair[1];
    }
    var pair = update_forward(d[4], d[6]);
    d[4] = pair[0]; d[6] = pair[1];
    pair = update_forward(d[5], d[7]);
    d[5] = pair[0]; d[7] = pair[1];
    let cosine_pi_four = 0.70710678118654752440084436210485;
    d[5] = multiply_complex(d[5], vec2<f32>(cosine_pi_four, -cosine_pi_four));
    d[7] = multiply_complex(d[7], vec2<f32>(-cosine_pi_four, -cosine_pi_four));
    pair = butterfly(d[0], d[2]); d[0] = pair[0]; d[2] = pair[1];
    pair = butterfly(d[1], d[3]); d[1] = pair[0]; d[3] = pair[1];
    pair = butterfly(d[0], d[1]); d[0] = pair[0]; d[1] = pair[1];
    pair = update_forward(d[2], d[3]); d[2] = pair[0]; d[3] = pair[1];
    pair = butterfly(d[4], d[5]); d[4] = pair[0]; d[5] = pair[1];
    pair = butterfly(d[6], d[7]); d[6] = pair[0]; d[7] = pair[1];
    return d;
}
fn transform(thread_id: u32, final_radix: bool) {
    if thread_id >= parameters.strides.x { return; }
    var d: array<vec2<f32>, 8>;
    if final_radix {
        let address = thread_id << 3u;
        for (var i = 0u; i < 8u; i++) { d[i] = source[address + i]; }
    } else {
        let remainder = thread_id & (parameters.strides.z - 1u);
        let address = ((thread_id - remainder) << 3u) + remainder;
        for (var i = 0u; i < 8u; i++) { d[i] = source[address + i * parameters.strides.z]; }
    }
    d = forward_eight(d);
    if !final_radix {
        let p = thread_id & (parameters.strides.z - parameters.strides.w);
        let phase = parameters.phase_base.x * f32(p);
        let order = array<u32, 7>(4u, 2u, 6u, 1u, 5u, 3u, 7u);
        for (var i = 0u; i < 7u; i++) {
            let angle = f32(i + 1u) * phase;
            d[order[i]] = multiply_complex(d[order[i]], vec2<f32>(cos(angle), sin(angle)));
        }
    }
    let remainder = thread_id & (parameters.strides.y - 1u);
    let address = ((thread_id - remainder) << 3u) + remainder;
    let order = array<u32, 8>(0u, 4u, 2u, 6u, 1u, 5u, 3u, 7u);
    for (var i = 0u; i < 8u; i++) {
        destination[address + i * parameters.strides.y] = d[order[i]];
    }
}
@compute @workgroup_size(128, 1, 1)
fn radix_eight(@builtin(global_invocation_id) id: vec3<u32>) {
    transform(id.x, false);
}
@compute @workgroup_size(128, 1, 1)
fn radix_eight_final(@builtin(global_invocation_id) id: vec3<u32>) {
    transform(id.x, true);
}
