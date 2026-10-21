// FFTWaves/UpdateSpectrum from the installed render-program declaration.
// The backend must decode typed half-buffer inputs to float storage.
struct SpectrumUniform {
    // ActualDim, InWidth, OutSize.x, OutSize.y.
    grid: vec4<u32>,
    displacement_offsets: vec4<u32>,
    // localTime.x, ocean/TimeScale.
    clock: vec4<f32>,
}

@group(0) @binding(0) var<uniform> parameters: SpectrumUniform;
@group(0) @binding(1) var<storage, read> initial_spectrum: array<vec2<f32>>;
@group(0) @binding(2) var<storage, read> angular_frequency: array<f32>;
@group(0) @binding(3) var<storage, read_write> current_spectrum: array<vec2<f32>>;

@compute @workgroup_size(16, 16, 1)
fn update_spectrum(@builtin(global_invocation_id) id: vec3<u32>) {
    // The source suppresses writes outside OutSize after evaluating the math.
    // None of those reads has side effects; suppress them before buffer access.
    if id.x >= parameters.grid.z || id.y >= parameters.grid.w {
        return;
    }
    let input_index = id.y * parameters.grid.y + id.x;
    let mirrored_index = (parameters.grid.x - id.y) * parameters.grid.y
        + parameters.grid.x - id.x;
    let output_index = id.y * parameters.grid.z + id.x;
    let positive = initial_spectrum[input_index];
    let negative = initial_spectrum[mirrored_index];
    let phase = angular_frequency[input_index] * parameters.clock.x * parameters.clock.y;
    let sine = sin(phase);
    let cosine = cos(phase);
    let height = vec2<f32>(
        (positive.x + negative.x) * cosine - (positive.y + negative.y) * sine,
        (positive.x - negative.x) * sine + (positive.y - negative.y) * cosine,
    );
    var kx = f32(id.x) - f32(parameters.grid.x) * 0.5;
    var ky = f32(id.y) - f32(parameters.grid.x) * 0.5;
    let squared = kx * kx + ky * ky;
    var inverse_length = 0.0;
    if squared > 1e-12 {
        inverse_length = 1.0 / sqrt(squared);
    }
    kx *= inverse_length;
    ky *= inverse_length;
    current_spectrum[output_index] = height;
    current_spectrum[output_index + parameters.displacement_offsets.x]
        = vec2<f32>(height.y * kx, -height.x * kx);
    current_spectrum[output_index + parameters.displacement_offsets.y]
        = vec2<f32>(height.y * ky, -height.x * ky);
}
