// PostFX/Bloom/upsample. Repeated offsets in the authored shader are
// intentional; every left/right term samples the same two coordinates.
struct BloomParameters {
    pixel_scale:vec4<f32>, // original positionToViewTexture; xy are unused
    viewport:vec4<f32>, // resolutionScale.xy, subpixelOffset.xy
    blur:vec4<f32>, // PostFX/Bloom/dUV.xy, env/bloom/veil_size, unused
}
fn bloom_upsample(p:BloomParameters,pixel:vec2<f32>,source:texture_2d<f32>,linear_mirror:sampler)->vec4<f32> {
    let uv=(pixel+p.viewport.zw)*p.pixel_scale.zw;
    let delta=p.blur.xy*p.blur.z;
    let limit=p.viewport.xy-p.pixel_scale.zw;
    let left=textureSampleLevel(source,linear_mirror,min(uv-delta,limit),0.0);
    let right=textureSampleLevel(source,linear_mirror,min(uv+delta,limit),0.0);
    var sum=0.18*textureSampleLevel(source,linear_mirror,uv,0.0);
    sum+=0.15*left;
    sum+=0.15*right;
    sum+=0.12*left;
    sum+=0.12*right;
    sum+=0.09*left;
    sum+=0.09*right;
    sum+=0.05*left;
    sum+=0.05*right;
    return sum;
}
@group(0) @binding(0) var<uniform> bloom:BloomParameters;
@group(0) @binding(1) var bloom_source:texture_2d<f32>;
@group(0) @binding(2) var bloom_mirror:sampler;
@vertex fn bloom_vertex(@builtin(vertex_index) vertex:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((vertex<<1u)&2u),f32(vertex&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn bloom_fragment(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    return bloom_upsample(bloom,position.xy,bloom_source,bloom_mirror);
}
