@group(0) @binding(0) var<uniform> display_parameters:ToneMappingParameters;
@group(0) @binding(1) var display_hdr:texture_2d<f32>;
@group(0) @binding(2) var display_cube:texture_3d<f32>;
@group(0) @binding(3) var display_linear:sampler;
@group(0) @binding(4) var<storage,read> display_adaptation:array<vec4<f32>>;
@vertex fn display_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn display_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    // The host HDR buffer already includes camera exposure and the radiance
    // unit conversion. Its inverse and custom EV are provided in display.z;
    // the current EngineLuminanceFactor stays on the GPU.
    var parameters=display_parameters;
    parameters.display.z*=display_adaptation[0].x;
    let radiance=textureLoad(display_hdr,vec2<i32>(pixel.xy),0).rgb;
    return vec4(standard_display_colour(parameters,radiance,display_cube,display_linear),1.0);
}
