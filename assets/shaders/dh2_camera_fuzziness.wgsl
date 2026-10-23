// PostFX/DOF/fuzziness: transparency, near-field and far-field mask producers.
struct CameraFuzzinessParameters {
    near_alpha:vec4<f32>, // near fuzzy/sharp, alpha sharp/fuzzy
    far_range:vec4<f32>, // far fuzzy/sharp/bias, common distance
    linear_alpha:vec4<f32>, // inverse projection Z coefficients, alpha bias
    centre_resolution:vec4<f32>,
    options:vec4<u32>, // pass (alpha=0, near=1, far=2), autofocus
}
@group(0) @binding(0) var<uniform> fuzz_p:CameraFuzzinessParameters;
@group(0) @binding(1) var fuzz_depth:texture_depth_2d;
@group(0) @binding(2) var fuzz_near:texture_2d<f32>;
@group(0) @binding(3) var fuzz_focus:texture_2d<f32>;
@group(0) @binding(4) var fuzz_linear:sampler;
struct FuzzVertex { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32>, }
@vertex fn fuzz_vertex(@builtin(vertex_index) id:u32)->FuzzVertex {
    let uv=vec2(f32((id<<1u)&2u),f32(id&2u));
    return FuzzVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv*fuzz_p.centre_resolution.zw);
}
@fragment fn fuzz_fragment(input:FuzzVertex)->@location(0) vec4<f32> {
    let p=fuzz_p;var far=p.far_range.w;var focus=vec2(0.0);
    if p.options.y!=0u {
        focus=point_clamp_sample(fuzz_focus,p.centre_resolution.xy).xy;
        far+=focus.x;
    }
    var depth=point_clamp_depth(fuzz_depth,input.uv);
    if p.options.x==1u {depth=textureSampleLevel(fuzz_depth,fuzz_linear,input.uv,0);}
    let linear=-p.linear_alpha.z/(p.linear_alpha.x*depth+p.linear_alpha.y);
    let normalized=clamp(linear/far,0.0,1.0);
    var result=vec2(0.0);
    if p.options.x==1u {
        var fuzzy=p.near_alpha.x;var sharp=p.near_alpha.y;
        if p.options.y!=0u {fuzzy=(fuzzy+focus.x)/far;sharp=(-sharp+focus.y)/far;}
        result.x=max(0.0,-((normalized-sharp)/(sharp-fuzzy)));
    } else {
        var sharp=p.far_range.y;var fuzzy=p.far_range.x;var bias=p.far_range.z;
        if p.options.x==0u {sharp=p.near_alpha.z;fuzzy=p.near_alpha.w;bias=p.linear_alpha.w;}
        if p.options.y!=0u {sharp=(sharp+focus.x)/far;fuzzy=(fuzzy+focus.y)/far;}
        let k=(normalized-sharp)/(fuzzy-sharp);
        let value=select(0.0,1.0,normalized>sharp)*k+bias;
        if p.options.x==0u {result=vec2(value,value*4.0);}
        else {result=clamp(vec2(textureSampleLevel(fuzz_near,fuzz_linear,input.uv,0.0).x,value),vec2(0.0),vec2(1.0));}
    }
    return result.xyxy;
}
