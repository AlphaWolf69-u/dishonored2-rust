// PostFX/motionBlurDisto: the raw ZW offset and four XY-offset colour taps.
struct MotionDistortionParameters {
    // resolutionScale.xy, positionToViewTexture.zw.
    scale_texel:vec4<f32>,
    // vectorFieldScales.x, subpixelOffset.xy, unused.
    scales_offset:vec4<f32>,
}
@group(0) @binding(0) var<uniform> motion:MotionDistortionParameters;
@group(0) @binding(1) var vectors:texture_2d<f32>;
@group(0) @binding(2) var colour:texture_2d<f32>;
@group(0) @binding(3) var bilinear_clamp:sampler;
@group(0) @binding(4) var bilinear_mirror:sampler;
struct MotionDistortionVertex {@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,}
@vertex fn motion_distortion_vertex(@builtin(vertex_index) index:u32)->MotionDistortionVertex {
    let uv=vec2(f32((index<<1u)&2u),f32(index&2u));
    return MotionDistortionVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
@fragment fn motion_distortion_fragment(input:MotionDistortionVertex)->@location(0) vec4<f32> {
    let raw=textureSampleLevel(vectors,bilinear_clamp,
        (input.uv+motion.scales_offset.yz)*motion.scale_texel.xy,0.0);
    let vector=raw.xy*motion.scales_offset.x;
    let uv0=input.uv+raw.zw;
    let taps=array<vec2<f32>,4>(uv0,uv0+vector,uv0+vector*2.0,uv0+vector*3.0);
    let limit=motion.scale_texel.xy-motion.scale_texel.zw*0.5;
    var sum=vec4(0.0);
    for(var i=0u;i<4u;i++) {
        var uv=taps[i]*motion.scale_texel.xy;
        uv*=sign(uv);
        uv=2.0*min(uv,limit)-uv;
        sum+=textureSampleLevel(colour,bilinear_mirror,uv,0.0);
    }
    sum*=0.25;
    sum.a=clamp(sum.a,0.0,1.0);
    return sum;
}
