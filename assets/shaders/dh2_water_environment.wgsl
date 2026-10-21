// arkWaterHelpers::GetSkyColor. Values and specialization choices are supplied
// by the selected water material; no fallback sky or probe is inferred here.
struct WaterEnvironment {
    sky_color:vec3<f32>,
    sphere_radius:f32,
    use_probes:bool,
    blend_probes:bool,
    use_sky_cube:bool,
}
fn water_sky_color(parameters:WaterEnvironment,position:vec3<f32>,direction:vec3<f32>,
    probe_indices:vec3<u32>,barycentrics:vec3<f32>,
    probes:texture_cube_array<f32>,probe_sampler:sampler,
    sky:texture_cube<f32>,sky_sampler:sampler)->vec3<f32> {
    if parameters.use_probes {
        var irradiance=textureSampleLevel(probes,probe_sampler,direction,i32(probe_indices.x),0.0).rgb;
        if parameters.blend_probes && (probe_indices.y!=probe_indices.x || probe_indices.z!=probe_indices.x) {
            irradiance*=barycentrics.x;
            irradiance+=barycentrics.y*textureSampleLevel(probes,probe_sampler,direction,i32(probe_indices.y),0.0).rgb;
            irradiance+=barycentrics.z*textureSampleLevel(probes,probe_sampler,direction,i32(probe_indices.z),0.0).rgb;
        }
        return irradiance;
    }
    if parameters.use_sky_cube {
        // Source assumes the position is inside a sphere centred at zero.
        let c=dot(position,position)-parameters.sphere_radius*parameters.sphere_radius;
        let b=dot(position,direction);
        let delta=b*b-c;
        let t=-b+sqrt(delta);
        let hit=position+t*direction;
        return textureSampleLevel(sky,sky_sampler,normalize(hit),0.0).rgb;
    }
    return parameters.sky_color;
}
