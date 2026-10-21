// Active (#if1) arkWaterHelpers::ComputeRefraction. Compose with point-address,
// array-layer and shadow helpers. All matrices/options are owner supplied.
struct WaterRefractionParameters {
    // localTime.x, CausticsSpeed, CausticsTiling, CausticsChromaticAberration.
    clock_caustics:vec4<f32>,
    // resolutionScale.xy, positionToViewTexture.zw.
    screen:vec4<f32>,
    // -inverseProjection[2].w, inverseProjection[3][2], [3][3], Refraction.
    unproject:vec4<f32>,
    // wsRefractedLightDir.xy, CausticsFade, WaterLevel.
    light_level:vec4<f32>,
    // ExtinctionCoeff.xyz; useCompressedHdrBuffers.
    extinction:vec4<f32>,
    // Actual auto-exposure EngineLuminanceFactor, view_white_level.x.
    hdr:vec4<f32>,
}
struct WaterRefractionInput {
    position:vec3<f32>,
    normal:vec3<f32>,
    view_direction:vec3<f32>,
    pixel:vec2<f32>,
    linear_depth:f32,
    sun_intensity:vec3<f32>,
}
struct WaterShadowParameters {
    world_to_shadow:array<mat4x4<f32>,5>,
    view_row:vec4<f32>,
    slice_fars:vec4<f32>,
    sun_direction:vec4<f32>,
    shadow_size:vec4<f32>,
    bias:vec4<f32>,
    // hasParallelShadow, actual shadowCommon options bitmask, unused, unused.
    options:vec4<u32>,
}
struct WaterRefraction {
    color:vec3<f32>,
    water_depth:f32,
}
fn water_parallel_shadow(parameters:WaterShadowParameters,position:vec3<f32>,
    shadow:texture_depth_2d_array,comparison:sampler_comparison)->f32 {
    if parameters.options.x==0u {return 1.0;}
    let slice=parallel_shadow_slice(position,parameters.view_row,parameters.slice_fars);
    let flags=parameters.options.y;
    // Installed water callers pass this exact placeholder normal; actual
    // normal-offset/slope settings still come from the selected shader path.
    let coordinates=shadow_offset(parameters.world_to_shadow[slice],position,vec3(0.0,1.0,0.0),
        parameters.sun_direction.xyz,true,(flags&1u)!=0u,(flags&2u)!=0u,
        (flags&4u)!=0u,(flags&8u)!=0u,parameters.bias.x,parameters.bias.y,parameters.bias.z);
    // SHADOW_USE_TAPS=1 suppresses cloud attenuation; LOW_QUALITY is one PCF tap.
    return sample_directional_shadow(shadow,comparison,coordinates,i32(slice),parameters.shadow_size.xy,0,0);
}
fn water_refraction(parameters:WaterRefractionParameters,input:WaterRefractionInput,
    shadow_parameters:WaterShadowParameters,opaque_depth:texture_depth_2d,
    hdr:texture_2d<f32>,caustics:texture_2d_array<f32>,
    bilinear_clamp:sampler,trilinear_wrap:sampler,
    shadow:texture_depth_2d_array,comparison:sampler_comparison)->WaterRefraction {
    let time=(parameters.clock_caustics.x*parameters.clock_caustics.y)%32.0;
    let distortion=parameters.unproject.w/input.linear_depth;
    let screen_uv=input.pixel/vec2<f32>(textureDimensions(hdr));
    let limit=parameters.screen.xy-parameters.screen.zw*0.5;
    var uv=clamp(screen_uv+input.normal.xy*distortion*parameters.screen.xy,vec2(0.0),limit);
    var raw=point_clamp_depth(opaque_depth,uv);
    var opaque=parameters.unproject.x/(parameters.unproject.y*raw+parameters.unproject.z);
    if opaque<input.linear_depth {
        uv=clamp(screen_uv,vec2(0.0),limit);
        raw=point_clamp_depth(opaque_depth,uv);
        opaque=parameters.unproject.x/(parameters.unproject.y*raw+parameters.unproject.z);
    }
    let depth=max(opaque-input.linear_depth,0.0);
    var refraction=textureSampleLevel(hdr,bilinear_clamp,uv,0.0).rgb;
    refraction=hdr_range_expand(refraction,parameters.extinction.w!=0.0,parameters.hdr.x,parameters.hdr.y);
    let underwater=input.position-input.view_direction*depth;
    let caustic_uv=(underwater.xy+underwater.z*parameters.light_level.xy)*parameters.clock_caustics.z+input.normal.xy*distortion;
    let layer=texture_array_layer(time,textureNumLayers(caustics));
    let chromatic=vec2(parameters.clock_caustics.w);
    let pattern=vec3(
        textureSampleLevel(caustics,trilinear_wrap,caustic_uv-chromatic,layer,0.0).r,
        textureSampleLevel(caustics,trilinear_wrap,caustic_uv,layer,0.0).r,
        textureSampleLevel(caustics,trilinear_wrap,caustic_uv+chromatic,layer,0.0).r);
    let vertical_depth=max(parameters.light_level.w-underwater.z,0.0);
    let extinction=exp(-parameters.extinction.xyz*vertical_depth);
    let visibility=water_parallel_shadow(shadow_parameters,underwater,shadow,comparison);
    refraction+=pattern*input.sun_intensity*parameters.light_level.z*visibility;
    refraction*=extinction;
    return WaterRefraction(refraction,depth);
}
