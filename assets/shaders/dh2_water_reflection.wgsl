// Complete default arkWaterHelpers::ComputeReflection. Compose after HDR,
// water-environment and hierarchical screen-ray helpers.
struct WaterReflectionParameters {
    view_from_world:mat4x4<f32>,
    camera_to_clip:mat4x4<f32>,
    unproject:vec4<f32>,
    thickness:vec4<f32>,
    // ZThicknessPow, VerticalFade, BorderFade, Soften.
    trace:vec4<f32>,
    // resolutionScale.xy, auto-exposure EngineLuminanceFactor, white_level.x.
    screen_hdr:vec4<f32>,
    // enableSSReflectionOption, enableSSReflection2, MaxStepsCount, compressedHDR.
    options:vec4<u32>,
}
struct WaterReflectionInput {
    position:vec3<f32>,
    view_position:vec3<f32>,
    view_direction:vec3<f32>,
    normal:vec3<f32>,
    amount:f32,
    probe_indices:vec3<u32>,
    barycentrics:vec3<f32>,
}
fn water_reflection(parameters:WaterReflectionParameters,environment:WaterEnvironment,
    input:WaterReflectionInput,opaque_depth:texture_depth_2d,reduced_depth:texture_2d<f32>,
    hdr:texture_2d<f32>,fog:texture_2d<f32>,probes:texture_cube_array<f32>,sky:texture_cube<f32>,
    linear_clamp:sampler,probe_sampler:sampler,sky_sampler:sampler)->vec3<f32> {
    var reflected=reflect(-input.view_direction,input.normal);
    if reflected.z<0.0 {
        reflected.z=1e-3;
        reflected=normalize(reflected);
    }
    if parameters.options.x!=0u && input.amount>0.0 && parameters.options.y!=0u {
        let softened=normalize(vec3(input.normal.xy,input.normal.z*parameters.trace.w));
        // The source only clamps the fallback direction above, not this SSR one.
        let soft_reflected=reflect(-input.view_direction,softened);
        let view_reflected=(parameters.view_from_world*vec4(soft_reflected,0.0)).xyz;
        let extent=vec2<u32>(parameters.screen_hdr.xy*vec2<f32>(textureDimensions(opaque_depth)));
        let hit=screen_ray_trace(input.view_position,view_reflected,parameters.options.z,
            opaque_depth,reduced_depth,extent,parameters.unproject.xyz,parameters.thickness,
            parameters.trace.x,parameters.trace.y,parameters.trace.z,parameters.camera_to_clip);
        let sky_colour=water_sky_color(environment,input.position,soft_reflected,
            input.probe_indices,input.barycentrics,probes,probe_sampler,sky,sky_sampler);
        let uv=hit.uv*parameters.screen_hdr.xy;
        var fade=hit.fade;
        if hit.depth>6599.0 {fade=0.0;}
        var reflected_scene=vec3(0.0);
        if fade>0.0 {
            reflected_scene=textureSampleLevel(hdr,linear_clamp,uv,0.0).rgb;
            reflected_scene=hdr_range_expand(reflected_scene,parameters.options.w!=0u,
                parameters.screen_hdr.z,parameters.screen_hdr.w);
        }
        let scattering=textureSampleLevel(fog,linear_clamp,uv,0.0);
        reflected_scene=(1.0-screen_ray_saturate(scattering.a))*reflected_scene+scattering.rgb;
        return mix(sky_colour,reflected_scene,fade);
    }
    return water_sky_color(environment,input.position,reflected,
        input.probe_indices,input.barycentrics,probes,probe_sampler,sky,sky_sampler);
}
