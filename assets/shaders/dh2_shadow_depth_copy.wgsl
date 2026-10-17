// depthCopy.decl: original vertex UV scale/bias followed by POINT SampleLevel
// of the selected source array slice, written directly to SV_DEPTH.
struct CopyRegion { uv_scale_bias:vec4<f32>, layer:vec4<u32> }
@group(0) @binding(0) var<uniform> region:CopyRegion;
@group(0) @binding(1) var source:texture_depth_2d_array;
@group(0) @binding(2) var point_sampler:sampler;
struct Output { @builtin(position) clip:vec4<f32>, @location(0) uv:vec2<f32> }
@vertex fn vertex(@builtin(vertex_index) index:u32)->Output {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    var output:Output;
    output.clip=vec4<f32>(uv*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0),0.0,1.0);
    output.uv=fma(uv,region.uv_scale_bias.xy,region.uv_scale_bias.zw);
    return output;
}
@fragment fn fragment(input:Output)->@builtin(frag_depth) f32 {
    return textureSampleLevel(source,point_sampler,input.uv,i32(region.layer.x),0);
}
