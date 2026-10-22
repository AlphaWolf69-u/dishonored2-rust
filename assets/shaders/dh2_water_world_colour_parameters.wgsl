struct WorldProbeColourParameters {
    reflection:WaterReflectionParameters,
    refraction:WaterRefractionParameters,
    shadow:WaterShadowParameters,
    surface:WaterSurfaceParameters,
    colour:WaterColourParameters,
    output:ColourOutputParameters,
    sky_radius:vec4<f32>,
    options:vec4<u32>,
}

// The host exposes the selected view's single auto-exposure record. Native
// water reads this GPU value for reflection, refraction and final HDR storage.
fn water_world_colour_with_exposure(parameters:WorldProbeColourParameters,
    engine_luminance:f32)->WorldProbeColourParameters {
    var result=parameters;
    result.reflection.screen_hdr.z=engine_luminance;
    result.refraction.hdr.x=engine_luminance;
    result.output.tint_exposure.w=engine_luminance;
    return result;
}
