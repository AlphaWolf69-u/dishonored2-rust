// Installed blendFogLut / applyFogLutOverrides; compose after dh2_point_sampling.
// Raster output retains R16F storage without requiring optional typed UAVs.
@group(0) @binding(0) var<uniform> weights:vec4<f32>;
@group(0) @binding(1) var source:texture_2d<f32>;
@group(0) @binding(2) var target0:texture_2d<f32>;
@group(0) @binding(3) var target1:texture_2d<f32>;
@group(0) @binding(4) var target2:texture_2d<f32>;
@group(0) @binding(5) var target3:texture_2d<f32>;
@vertex fn vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn blend_fog_lookup(@builtin(position) position:vec4<f32>)->@location(0) f32 {
    let uv=vec2(position.x/4096.0,0.5);
    let a=point_clamp_sample(source,uv).r;
    let b=point_clamp_sample(target0,uv).r;
    return fma(b-a,weights.x,a);
}
@fragment fn override_fog_lookup(@builtin(position) position:vec4<f32>)->@location(0) f32 {
    let uv=vec2(position.x/4096.0,0.5);
    let a=point_clamp_sample(source,uv).r;
    var b=weights.x*point_clamp_sample(target0,uv).r;
    if weights.y>1e-6 {b=fma(weights.y,point_clamp_sample(target1,uv).r,b);}
    if weights.z>1e-6 {b=fma(weights.z,point_clamp_sample(target2,uv).r,b);}
    if weights.w>1e-6 {b=fma(weights.w,point_clamp_sample(target3,uv).r,b);}
    return fma(b-a,dot(weights,vec4(1.0)),a);
}
