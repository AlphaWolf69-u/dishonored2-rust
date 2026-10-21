// FFTWaves/UpdateDisplacement: real FFT channels with checkerboard correction.
struct DisplacementUniform {
    // DtAddressOffset.x/y, OutSize.x.
    addresses: vec4<u32>,
    choppiness: vec4<f32>,
}
@group(0) @binding(0) var<uniform> parameters: DisplacementUniform;
@group(0) @binding(1) var<storage, read> transformed: array<vec2<f32>>;

@vertex fn fullscreen_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn displacement_fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let x = u32(position.x);
    let y = u32(position.y);
    let address = parameters.addresses.z * y + x;
    let correction = select(1.0, -1.0, ((x + y) & 1u) != 0u);
    let dx = transformed[address + parameters.addresses.x].x * correction * parameters.choppiness.x;
    let dy = transformed[address + parameters.addresses.y].x * correction * parameters.choppiness.x;
    let dz = transformed[address].x * correction;
    return vec4<f32>(dx, dy, dz, 1.0);
}
