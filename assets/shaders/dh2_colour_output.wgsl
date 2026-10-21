// Shared global::BlackAndWhiteOverlay and RangeCompressColorToR11G11B10.
struct ColourOutputParameters {
    // blackWhiteModeColor.xyz, actual auto-exposure EngineLuminanceFactor.
    tint_exposure:vec4<f32>,
    // view_white_level.x, unused.
    white_level:vec4<f32>,
    // blackWhiteModeRuntime, blackWhiteModeExcluded, useCompressedHdrBuffers.
    options:vec4<u32>,
}
fn black_white_overlay(colour:vec3<f32>,tint:vec3<f32>,enabled:bool,excluded:bool)->vec3<f32> {
    if !enabled {return colour;}
    let luminance=dot(colour,vec3(0.2126,0.7152,0.0722));
    return vec3(luminance)*select(vec3(1.0),tint,excluded);
}
fn hdr_range_compress(colour:vec3<f32>,compressed:bool,engine_luminance:f32,white_level:f32)->vec3<f32> {
    if !compressed {return colour;}
    return colour/(engine_luminance*white_level);
}
fn colour_output(parameters:ColourOutputParameters,colour:vec3<f32>)->vec3<f32> {
    let transformed=black_white_overlay(colour,parameters.tint_exposure.xyz,
        parameters.options.x!=0u,parameters.options.y!=0u);
    return hdr_range_compress(transformed,parameters.options.z!=0u,
        parameters.tint_exposure.w,parameters.white_level.x);
}
