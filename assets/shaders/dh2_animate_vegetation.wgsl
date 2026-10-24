// arkvegetation's AnimateVegetation, as compiled into the installed rigid and
// instanced shadow VS and the forward VS (674e25d0: identical bending ops).
// Wind is supplied by each ORIGINAL input ABI; this helper does not choose a
// clock, wind direction, amplitude or fallback. Shadow programs concatenate
// this file; forward materials import it as `dh2_animate_vegetation`.
struct VegetationAnimation {
    main_variance: vec4<f32>,
    main_frequency: vec4<f32>,
    main_amplitude: vec4<f32>,
    leaf_minimum: vec4<f32>,
    leaf_frequency: vec4<f32>,
    leaf_amplitude: vec4<f32>,
    branch_amplitude: vec4<f32>,
}
fn animate_vegetation(p: VegetationAnimation, time: f32,
    local_wind: vec2<f32>, leaf_strength: f32,
    position: vec3<f32>, normal_xy: vec2<f32>, color: vec3<f32>) -> vec3<f32>
{
    let phase = dot(position, vec3<f32>(color.y + 1.0));
    let wave0 = fract((vec4<f32>(phase, phase, color.y, color.y) + vec4<f32>(time))
        * vec4<f32>(1.975, 0.793, 0.375, 0.193));
    let wave1 = fma(wave0, vec4<f32>(2.0), vec4<f32>(-1.0));
    let wave2 = fract(fma(wave1, vec4<f32>(p.leaf_frequency.x), vec4<f32>(0.5)));
    let wave3 = abs(fma(wave2, vec4<f32>(2.0), vec4<f32>(-1.0)));
    let smooth_wave = fma(-wave3, vec4<f32>(2.0), vec4<f32>(3.0)) * (wave3 * wave3);
    let summed_wave = vec2<f32>(smooth_wave.x + smooth_wave.y, smooth_wave.z + smooth_wave.w);
    let leaf = (leaf_strength * p.leaf_amplitude.x) * color.z;
    let amplitudes = vec3<f32>(leaf * normal_xy.x,
        color.x * p.branch_amplitude.x, leaf * normal_xy.y);
    let bent = fma(vec3<f32>(summed_wave.x - 1.0, summed_wave.y, summed_wave.x - 1.0),
        amplitudes, position);
    let main_wave = sin(p.main_frequency.x * time);
    let main_amplitude = fma(main_wave, p.main_variance.x, p.main_amplitude.x);
    let main_weight = fma(bent.z, main_amplitude, 1.0);
    let squared_weight = main_weight * main_weight;
    let main_bend = fma(squared_weight, squared_weight, -squared_weight);
    let displaced = vec3<f32>(fma(vec2<f32>(main_bend), local_wind, bent.xy), bent.z);
    return sqrt(dot(bent, bent)) * (inverseSqrt(dot(displaced, displaced)) * displaced);
}
