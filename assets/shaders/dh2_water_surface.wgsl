// Complete default arkWaterHelpers::WaterShading, above and below the surface.
struct WaterSurfaceParameters {
    // UnderwaterVisibilityBoost, ShoreFoamLuminance, unused.
    optical:vec4<f32>,
    // UseSunLight, disableSSReflectionOnRoughWaterOption, unused.
    options:vec4<u32>,
}
struct WaterSurfaceInput {
    is_ocean:bool,
    underwater:bool,
    water_level:f32,
    pixel:vec2<f32>,
    position:vec3<f32>,
    view_position:vec3<f32>,
    normal:vec3<f32>,
    view_direction:vec3<f32>,
    color:vec3<f32>,
    extinction:f32,
    inscattering:f32,
    sun_direction:vec3<f32>,
    sun_intensity:vec3<f32>,
    specular_power:f32,
    additional_shadow:f32,
    refraction:f32,
    foam_fft:f32,
    foam:f32,
    foam_depth:f32,
    wave_amplitude:f32,
    probe_indices:vec3<u32>,
    barycentrics:vec3<f32>,
}
fn water_surface(parameters:WaterSurfaceParameters,reflection_parameters:WaterReflectionParameters,
    refraction_parameters:WaterRefractionParameters,shadow_parameters:WaterShadowParameters,
    environment:WaterEnvironment,input:WaterSurfaceInput,
    opaque_depth:texture_depth_2d,reduced_depth:texture_2d<f32>,hdr:texture_2d<f32>,fog:texture_2d<f32>,
    caustics:texture_2d_array<f32>,probes:texture_cube_array<f32>,sky:texture_cube<f32>,
    shadow:texture_depth_2d_array,linear_clamp:sampler,trilinear_wrap:sampler,
    probe_sampler:sampler,sky_sampler:sampler,comparison:sampler_comparison)->vec4<f32> {
    let use_sun=parameters.options.x!=0u;
    let sun=select(vec3(1.0),input.sun_intensity,use_sun);
    let visibility=water_parallel_shadow(shadow_parameters,input.position,shadow,comparison)*input.additional_shadow;
    let depth=-input.view_position.z;
    let normal=input.normal*select(1.0,-1.0,input.underwater);
    let ior=select(1.0/1.333,1.333,input.underwater);
    var extinction=(vec3(1.0)-input.color)*input.extinction;
    if input.underwater {extinction*=1.0/parameters.optical.x;}
    let fresnel=water_fresnel(ior,screen_ray_saturate(dot(normal,input.view_direction))).value;
    var amount=1.0;
    if !input.underwater && parameters.options.y!=0u {
        amount=screen_ray_smooth(0.7,0.8,1.0-input.wave_amplitude);
    }
    let reflected_input=WaterReflectionInput(input.position,input.view_position,input.view_direction,
        normal,amount,input.probe_indices,input.barycentrics);
    var reflected=water_reflection(reflection_parameters,environment,reflected_input,
        opaque_depth,reduced_depth,hdr,fog,probes,sky,linear_clamp,probe_sampler,sky_sampler);
    if !input.underwater {
        if use_sun {
            let half_direction=normalize(input.sun_direction+input.view_direction);
            let incidence=screen_ray_saturate(dot(normal,half_direction));
            reflected+=sun*(pow(incidence,input.specular_power)*visibility);
        }
        var refract_parameters=refraction_parameters;
        refract_parameters.extinction=vec4(extinction,refraction_parameters.extinction.w);
        refract_parameters.light_level.w=input.water_level;
        let refracted_input=WaterRefractionInput(input.position,normal,input.view_direction,input.pixel,depth,sun);
        let refracted=water_refraction(refract_parameters,refracted_input,shadow_parameters,
            opaque_depth,hdr,caustics,linear_clamp,trilinear_wrap,shadow,comparison);
        // The default source deliberately applies extinction in both refraction
        // and this distance composition; they measure different depths.
        let attenuation=exp(-extinction*refracted.water_depth);
        let scattering=sun*water_inscattering(0.0,refracted.water_depth,refracted.water_depth,extinction)*input.inscattering;
        let transmitted=refracted.color*attenuation+scattering;
        var color=mix(transmitted,reflected,fresnel);
        color+=mix(input.foam_fft,input.foam*parameters.optical.y,input.foam_depth)*sun*visibility;
        let raw=textureLoad(opaque_depth,vec2<i32>(input.pixel),0);
        let geometry_depth=refraction_parameters.unproject.x/(refraction_parameters.unproject.y*raw+refraction_parameters.unproject.z);
        return vec4(color,screen_ray_smooth(0.0,0.2,abs(geometry_depth-depth)));
    }
    let screen_uv=input.pixel/vec2<f32>(textureDimensions(hdr));
    let limit=refraction_parameters.screen.xy-refraction_parameters.screen.zw*0.5;
    let uv=clamp(screen_uv+normal.xy*input.refraction*refraction_parameters.screen.xy,vec2(0.0),limit);
    var transmitted=textureSampleLevel(hdr,linear_clamp,uv,0.0).rgb;
    transmitted=hdr_range_expand(transmitted,refraction_parameters.extinction.w!=0.0,
        refraction_parameters.hdr.x,refraction_parameters.hdr.y);
    if use_sun {
        let refracted_direction=refract(-input.view_direction,normal,ior);
        let incidence=max(dot(refracted_direction,input.sun_direction),0.0);
        transmitted+=sun*(pow(incidence,input.specular_power)*visibility);
    }
    var color=mix(transmitted,reflected,fresnel*select(0.0,1.0,input.is_ocean));
    color+=mix(input.foam_fft,input.foam*parameters.optical.y,
        screen_ray_saturate(0.1+input.foam_depth))*sun*visibility;
    return vec4(color,1.0);
}
