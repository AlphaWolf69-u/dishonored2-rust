// Same32-byte camera adaptation owner used by placed water and HDR conversion.
@group(0) @binding(1) var<storage,read> ocean_view_adaptation:array<vec4<f32>,2>;
@fragment fn ocean_colour_fragment_with_exposure(vertex:OceanColourVertex,
    @builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    var parameters=ocean;
    let luminance=ocean_view_adaptation[0].x;
    parameters.reflection.screen_hdr.z=luminance;
    parameters.refraction.hdr.x=luminance;
    parameters.output.tint_exposure.w=luminance;
    return ocean_colour(vertex,front,parameters);
}
