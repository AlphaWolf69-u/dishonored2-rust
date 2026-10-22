@group(0) @binding(5) var<uniform> display_lens:LensCompositeParameters;
@group(0) @binding(6) var display_bloom:texture_2d<f32>;
@group(0) @binding(7) var display_dirt_from:texture_2d<f32>;
@group(0) @binding(8) var display_dirt_to:texture_2d<f32>;
@group(0) @binding(9) var display_flares:texture_2d<f32>;
@fragment fn display_lens_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let engine_luminance=display_adaptation[0].x;
    var raw=textureLoad(display_hdr,vec2<i32>(pixel.xy),0).rgb/display_lens.pixel.w;
    // Host primary HDR remains uncompressed. Encode it for the native decode
    // when requested, while bloom retains the original separate expansion.
    if (display_lens.options.x&2u)!=0u {
        raw=hdr_range_compress(raw,true,engine_luminance,display_lens.controls.w);
    }
    let radiance=lens_composite(display_lens,raw,pixel.xy*display_lens.pixel.xy,engine_luminance,
        display_bloom,display_dirt_from,display_dirt_to,display_flares,display_linear);
    var parameters=display_parameters;
    parameters.display.z*=engine_luminance*display_lens.pixel.w;
    if display_lens.flags.w!=0u {parameters.options.y=0u;}
    return vec4(standard_display_colour(parameters,radiance,display_cube,display_linear),1.0);
}
