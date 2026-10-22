// PostFX/Luminance/downsample. The host HDR input is linear and includes
// camera exposure; undo those host units only in the first pass.
struct LuminanceParameters { screen:vec4<f32>, } // inverse target size, camera exposure, unused
@group(0) @binding(0) var<uniform> luminance:LuminanceParameters;
@group(0) @binding(1) var luminance_source:texture_2d<f32>;
@group(0) @binding(2) var luminance_clamp:sampler;
@vertex fn luminance_vertex(@builtin(vertex_index) vertex:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((vertex<<1u)&2u),f32(vertex&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn luminance_first(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    let uv=position.xy*luminance.screen.xy;
    let colour=max(vec3(0.0),textureSampleLevel(luminance_source,luminance_clamp,uv,0.0).rgb/luminance.screen.z)/139.26;
    return vec4(colour,dot(colour,vec3(0.2126,0.7152,0.0722)));
}
@fragment fn luminance_standard(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    let delta=luminance.screen.xy;
    var uv=position.xy*delta-0.5*delta;
    let v00=textureSampleLevel(luminance_source,luminance_clamp,uv,0.0);
    uv.x+=delta.x;
    let v01=textureSampleLevel(luminance_source,luminance_clamp,uv,0.0);
    uv.y+=delta.y;
    let v11=textureSampleLevel(luminance_source,luminance_clamp,uv,0.0);
    uv.x-=delta.x;
    let v10=textureSampleLevel(luminance_source,luminance_clamp,uv,0.0);
    return 0.25*(v00+v01+v10+v11);
}
