// PostFX_DWAO's two-sample AO. Params0 is published by 3053f0:
// texture width*.04, .04, .25, 2; Params1 is zero.
struct DwaoParameters { params:vec4<f32>, unproject:vec4<f32>, scale:vec4<f32> }
@group(0) @binding(0) var<uniform> dwao:DwaoParameters;
@group(0) @binding(1) var dwao_depth:texture_depth_2d;
@group(0) @binding(2) var dwao_reduced:texture_2d<f32>;
@vertex fn dwao_vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((i<<1u)&2u),f32(i&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
fn dwao_max_depth(pixel:vec2<i32>)->f32 {
    // D3D11 indexed loads return zero outside the allocation, not an edge texel.
    let size=vec2<i32>(textureDimensions(dwao_reduced));
    if any(pixel<vec2(0)) || any(pixel>=size) {return 0.0;}
    return textureLoad(dwao_reduced,pixel,0).z;
}
@fragment fn dwao_occlusion(@builtin(position) position:vec4<f32>)->@location(0) vec4<f32> {
    let pixel=vec2<i32>(position.xy);let raw=textureLoad(dwao_depth,pixel,0);
    // The source far-plane quad uses depth NOTEQUAL; untouched sky keeps R=1.
    if raw==0.0 {return vec4(1.0);}
    let z=dwao.unproject.x/(dwao.unproject.y*raw+dwao.unproject.z);
    let rotations=array<vec2<f32>,16>(
        vec2(-0.388235294,0.921568627),vec2(-0.929411765,0.380392157),
        vec2(0.921568627,0.380392157),vec2(0.380392157,-0.929411765),
        vec2(-0.003921569,-1.0),vec2(-0.709803922,-0.709803922),
        vec2(0.701960784,-0.709803922),vec2(0.380392157,0.921568627),
        vec2(-0.929411765,-0.388235294),vec2(-0.709803922,0.701960784),
        vec2(1.0,-0.003921569),vec2(-0.003921569,1.0),
        vec2(0.921568627,-0.388235294),vec2(-0.388235294,-0.929411765),
        vec2(-1.0,-0.003921569),vec2(0.701960784,0.701960784));
    let r=rotations[(pixel.y&3)*4+(pixel.x&3)];
    let rotation=vec2(-0.15*r.x-0.35*r.y,-0.15*r.y+0.35*r.x);
    let offset=dwao.params.x*rotation*dwao.scale.xy;
    let p=vec2<f32>(pixel);
    let delta=vec2(z-dwao_max_depth(vec2<i32>(0.5*(p+offset))),
        z-dwao_max_depth(vec2<i32>(0.5*(p-offset))));
    let contribution=clamp((delta/(dwao.params.y*z))*0.5+vec2(0.5),vec2(0.0),vec2(1.0));
    let max_difference=dwao.params.z*z;
    let distance=clamp((vec2(max_difference)-delta)/max_difference,vec2(0.0),vec2(1.0));
    let modified=contribution*distance;
    let amount=clamp(dwao.params.w*(modified.x+modified.y-1.0),0.0,1.0);
    return vec4(1.0-amount);
}
