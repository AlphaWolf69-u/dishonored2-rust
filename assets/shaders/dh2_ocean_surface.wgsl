// arkOceanHelpers active Windows paths. Compose after dh2_ocean_noise.wgsl.
// Parameter values come from the current environment/view; no shader defaults.
struct OceanSurfaceParameters {
    // ScaleX, ScaleY, ScaleZ, MasksExtent.
    spatial: vec4<f32>,
    // localTime.x, TimeScale, globalViewInfos.z, globalViewInfos.w.
    clock_view: vec4<f32>,
    // wsCamPos.xyz, NormalFadeOutStart.
    camera: vec4<f32>,
    // PerlinBlendStart, PerlinFrequency, PerlinSpeed, PerlinAmplitude.
    perlin: vec4<f32>,
    // PerlinPersistence, PerlinFoamThreshold, PerlinFoamTransition, FoamLuminance.
    perlin_foam: vec4<f32>,
    // ShoreWavesFrequency, ShoreWavesSpeed, ShoreWavesAmplitude, ShoreWavesFoamLuminance.
    shore: vec4<f32>,
    // ShoreWavesFoamFadeDistance, ShoreWavesFoamFadeLength, FoamBubblesLuminance, NormalFadeOut.
    foam: vec4<f32>,
}
struct OceanHeightAndFoam {
    height: f32,
    foam: f32,
}
struct OceanSurfaceSample {
    displacement: vec3<f32>,
    foam: f32,
}
struct OceanNormalSample {
    normal: vec3<f32>,
    foam: f32,
}
struct OceanSurfaceInput {
    world_position: vec2<f32>,
    camera_distance: f32,
    perlin_blend: f32,
    amplitude_mask: f32,
    prewarped: bool,
    foam_texture: f32,
}

fn ocean_linear_step(a: f32, b: f32, x: f32) -> f32 {
    // D3D saturate maps unordered results to zero. Preserve that case and
    // equal endpoints instead of depending on WGSL clamp's NaN behaviour.
    if a == b { return select(0.0, 1.0, x > a); }
    let value = (x - a) / (b - a);
    if !(value > 0.0) { return 0.0; }
    return min(value, 1.0);
}
fn ocean_smooth_step(a: f32, b: f32, x: f32) -> f32 {
    return noise_curve_three(ocean_linear_step(a, b, x));
}

fn ocean_perlin_waves(parameters: OceanSurfaceParameters,
                       permutation: texture_2d<f32>, gradients: texture_2d<f32>,
                       point_wrap: sampler, position: vec2<f32>,
                       camera_distance: f32, foam_texture: f32) -> OceanHeightAndFoam {
    var output = OceanHeightAndFoam(0.0, 0.0);
    if parameters.perlin.x > camera_distance { return output; }
    let raw_octaves = 4.0 * (1.0 - ocean_linear_step(
        parameters.clock_view.z + parameters.perlin.x, parameters.clock_view.w, camera_distance));
    let octaves = 1 + i32(floor(raw_octaves));
    let last_blend = raw_octaves % 1.0;
    let time = parameters.clock_view.x * parameters.clock_view.y;
    // PERLIN_WAVE_METHOD=0; its declared windSpeed is multiplied by zero.
    let noise_position = vec3(position * parameters.perlin.y, time * parameters.perlin.z);
    output.height = noise_fractal_three(permutation, gradients, point_wrap, noise_position,
        octaves, last_blend, vec3(2.054, 2.042, 1.3), parameters.perlin_foam.x, 2);
    output.height *= parameters.perlin.w;
    let foam_mask = ocean_smooth_step(parameters.perlin_foam.y,
        parameters.perlin_foam.y + parameters.perlin_foam.z, output.height);
    output.foam = foam_texture * foam_mask * parameters.perlin_foam.w;
    return output;
}

fn ocean_shore_waves(parameters: OceanSurfaceParameters, position: vec2<f32>,
                      amplitude_mask: f32, shore_distance: f32) -> OceanHeightAndFoam {
    var distance_mask = 1.0 - min(abs(shore_distance - 0.125) * 8.0, 1.0);
    distance_mask = noise_curve_three(distance_mask);
    let random = noise_shore_voronoi(position * 0.01);
    let wave_mask = distance_mask * random.y * amplitude_mask;
    let phase = shore_distance * parameters.shore.x + random.x * 80.0
        + parameters.clock_view.x * parameters.shore.y;
    let sine = sin(phase);
    let cosine = cos(phase);
    var waves = cosine * 0.5 + 0.5;
    waves = pow(waves, select(3.0, 23.0, 0.0 >= sine));
    let direction = parameters.camera.xy - position;
    let distance_squared = dot(direction, direction);
    let far_threshold = parameters.foam.x + parameters.foam.y;
    // Source smoothstep intentionally has descending endpoints here.
    let fade = ocean_smooth_step(far_threshold * far_threshold,
        parameters.foam.x * parameters.foam.x, distance_squared);
    let foam = ocean_smooth_step(0.5, 0.8, waves * wave_mask)
        * parameters.shore.w * fade * amplitude_mask;
    waves *= parameters.shore.z;
    waves *= wave_mask;
    return OceanHeightAndFoam(waves, foam);
}

// Pass the CURRENT or PREVIOUS FFT image as selected by the caller. The source
// previous path still uses current shore/perlin time; it does not rewind noise.
fn ocean_surface_displacement(parameters: OceanSurfaceParameters, input: OceanSurfaceInput,
                                permutation: texture_2d<f32>, gradients: texture_2d<f32>,
                                point_wrap: sampler, bilinear_wrap: sampler, bilinear_clamp: sampler,
                                shore_distance: texture_2d<f32>, fft_displacement: texture_2d<f32>,
                                collision_height: texture_2d<f32>, fft_foam: texture_2d<f32>) -> OceanSurfaceSample {
    var global_uv = input.world_position / parameters.spatial.w + vec2(0.5);
    global_uv.x = 1.0 - global_uv.x;
    let position = input.world_position * parameters.spatial.xy;
    let shore_dist = textureSampleLevel(shore_distance, bilinear_clamp, global_uv, 0.0).x;
    let shore = ocean_shore_waves(parameters, position, input.amplitude_mask, shore_dist);
    var foam = shore.foam * input.foam_texture;
    var fft_wave = vec3(0.0);
    var perlin_wave = vec3(0.0);
    if input.perlin_blend < 1.0 {
        let uv = position * 0.02;
        if input.prewarped {
            fft_wave.z = textureSampleLevel(collision_height, bilinear_wrap, uv, 0.0).x;
        } else {
            fft_wave = textureSampleLevel(fft_displacement, bilinear_wrap, uv, 0.0).xyz;
            fft_wave = vec3(fft_wave.xy * input.amplitude_mask, fft_wave.z);
        }
        let foam_mask = textureSampleLevel(fft_foam, bilinear_wrap, uv, 0.0).x;
        var fft_foam_value = foam_mask * parameters.foam.z;
        fft_foam_value += input.foam_texture * foam_mask * parameters.perlin_foam.w;
        fft_foam_value *= input.amplitude_mask * input.amplitude_mask * input.amplitude_mask;
        foam += fft_foam_value * (1.0 - input.perlin_blend);
    }
    if input.perlin_blend > 0.0 {
        let perlin = ocean_perlin_waves(parameters, permutation, gradients, point_wrap,
            position, input.camera_distance, input.foam_texture);
        perlin_wave.z = perlin.height;
        foam += perlin.foam * input.perlin_blend;
    }
    var waves = mix(fft_wave, perlin_wave, input.perlin_blend);
    waves.z *= input.amplitude_mask * parameters.spatial.z;
    waves += vec3(0.0, 0.0, shore.height);
    return OceanSurfaceSample(waves, foam);
}

fn ocean_surface_normal(parameters: OceanSurfaceParameters, input: OceanSurfaceInput,
                         permutation: texture_2d<f32>, gradients: texture_2d<f32>,
                         point_wrap: sampler, bilinear_wrap: sampler, bilinear_clamp: sampler,
                         shore_distance: texture_2d<f32>, fft_displacement: texture_2d<f32>,
                         collision_height: texture_2d<f32>, fft_foam: texture_2d<f32>) -> OceanNormalSample {
    let delta = 0.098;
    var sample_input = input;
    sample_input.prewarped = false;
    let origin = ocean_surface_displacement(parameters, sample_input, permutation, gradients,
        point_wrap, bilinear_wrap, bilinear_clamp, shore_distance, fft_displacement, collision_height, fft_foam);
    sample_input.foam_texture = 0.0;
    sample_input.world_position = input.world_position + vec2(delta, 0.0);
    let dx = ocean_surface_displacement(parameters, sample_input, permutation, gradients,
        point_wrap, bilinear_wrap, bilinear_clamp, shore_distance, fft_displacement, collision_height, fft_foam);
    sample_input.world_position = input.world_position + vec2(0.0, delta);
    let dy = ocean_surface_displacement(parameters, sample_input, permutation, gradients,
        point_wrap, bilinear_wrap, bilinear_clamp, shore_distance, fft_displacement, collision_height, fft_foam);
    let fade = max(input.camera_distance - parameters.camera.w, 0.0) * parameters.foam.w + 1.0;
    return OceanNormalSample(normalize(vec3(origin.displacement.z - dx.displacement.z,
        origin.displacement.z - dy.displacement.z, delta * fade)), origin.foam);
}
