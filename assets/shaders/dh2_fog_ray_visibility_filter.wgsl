// atm/distanceFog/renderBilateralBlurNoDepth. This expands ray visibility;
// it neither averages the samples nor reads or changes fog shadowing.
struct VisibilityFilterParameters {
    extent:vec2<f32>,
    radius:f32,
    horizontal:u32,
}
@group(0) @binding(0) var<uniform> filter_parameters:VisibilityFilterParameters;
@group(0) @binding(1) var source:texture_2d<f32>;
@group(0) @binding(2) var linear_mirror:sampler;
@vertex fn visibility_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn visibility_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) f32 {
    let uv=pixel.xy/filter_parameters.extent;
    let direction=select(vec2(0.0,1.0),vec2(1.0,0.0),filter_parameters.horizontal!=0u);
    var colour=0.0;
    for(var radius=-filter_parameters.radius;radius<=filter_parameters.radius;radius+=1.0) {
        // The installed shader uses aInvResolution.x for BOTH directions.
        colour+=textureSampleLevel(source,linear_mirror,
            uv+direction*(radius/filter_parameters.extent.x),0.0).r;
    }
    return clamp(colour,0.0,1.0);
}
