// PostFX/DOF/blur: preserve its vertex offsets and eight ordered sample terms.
struct CameraMaskBlurParameters {
    offsets:array<vec4<f32>,4>,
    weights:array<vec4<f32>,2>,
    resolution:vec4<f32>,
}
@group(0) @binding(0) var<uniform> blur:CameraMaskBlurParameters;
@group(0) @binding(1) var mask:texture_2d<f32>;
@group(0) @binding(2) var linear_clamp:sampler;
struct BlurVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) uv0:vec4<f32>,
    @location(1) uv1:vec4<f32>,
    @location(2) uv2:vec4<f32>,
    @location(3) uv3:vec4<f32>,
}
@vertex fn blur_vertex(@builtin(vertex_index) id:u32)->BlurVertex {
    let uv=vec2(f32((id<<1u)&2u),f32(id&2u));
    let coordinate=(uv*blur.resolution.xy).xyxy;
    return BlurVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),coordinate+blur.offsets[0],coordinate+blur.offsets[1],coordinate+blur.offsets[2],coordinate+blur.offsets[3]);
}
@fragment fn blur_fragment(input:BlurVertex)->@location(0) vec4<f32> {
    let uv=array<vec2<f32>,8>(input.uv0.xy,input.uv0.zw,input.uv1.xy,input.uv1.zw,input.uv2.xy,input.uv2.zw,input.uv3.xy,input.uv3.zw);
    var value=0.0;
    for(var i=0u;i<8u;i++) {value+=textureSampleLevel(mask,linear_clamp,uv[i],0.0).x*blur.weights[i>>2u][i&3u];}
    return vec4(value);
}
