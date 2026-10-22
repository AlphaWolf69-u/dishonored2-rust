// PostFX_LensFlares_Threshold, PostFX_LensFlares_Generate and
// PostFX_Incremental_Gaussian_Blur, as shipped in the installed declarations.
struct FlareParameters {
    view:vec4<f32>,       // inverse viewport size, resolution scale
    threshold:vec4<f32>,  // Downscale/TexelSize.xy, bias, scale
    ghost:vec4<f32>,      // dispersal, distortion, halo width, halo weight
    controls:vec4<f32>,   // profile blend, blur radius, ghost count, BW program
    direction:vec4<f32>,  // GaussianBlur/Direction.xy
}
@group(0) @binding(0) var<uniform> p:FlareParameters;
@group(0) @binding(1) var image:texture_2d<f32>;
@group(0) @binding(2) var profile_from:texture_2d<f32>;
@group(0) @binding(3) var profile_to:texture_2d<f32>;
@group(0) @binding(4) var linear_clamp:sampler;
@group(0) @binding(5) var linear_mirror:sampler;

fn flare_threshold(uv:vec2<f32>)->vec4<f32> {
    let d=p.threshold.xy;
    let a=textureSampleLevel(image,linear_clamp,uv+vec2(-d.x,-d.y),0.0);
    let b=textureSampleLevel(image,linear_clamp,uv+vec2(d.x,-d.y),0.0);
    let c=textureSampleLevel(image,linear_clamp,uv+vec2(-d.x,d.y),0.0);
    let e=textureSampleLevel(image,linear_clamp,uv+d,0.0);
    var rgb=max(vec3(0.0),a.rgb-vec3(p.threshold.z))*p.threshold.w;
    rgb+=max(vec3(0.0),b.rgb-vec3(p.threshold.z))*p.threshold.w;
    rgb+=max(vec3(0.0),c.rgb-vec3(p.threshold.z))*p.threshold.w;
    rgb+=max(vec3(0.0),e.rgb-vec3(p.threshold.z))*p.threshold.w;
    return vec4(rgb*0.25,0.25*(a.a+b.a+c.a+e.a));
}
fn flare_distort(uv:vec2<f32>,direction:vec2<f32>,distortion:vec3<f32>)->vec3<f32> {
    let r=textureSampleLevel(image,linear_clamp,(uv+direction*distortion.r)*p.view.zw,0.0).r;
    let g=textureSampleLevel(image,linear_clamp,(uv+direction*distortion.g)*p.view.zw,0.0).g;
    let b=textureSampleLevel(image,linear_clamp,(uv+direction*distortion.b)*p.view.zw,0.0).b;
    return vec3(r,g,b);
}
fn flare_generate(uv:vec2<f32>)->vec4<f32> {
    let coord=vec2(1.0)-uv;
    let texel=p.view.xy*p.view.zw;
    let ghost=(vec2(0.500001)-coord)*p.ghost.x;
    let direction=normalize(ghost);
    let halo=direction*p.ghost.z;
    let distortion=vec3(-texel.x*p.ghost.y,0.0,texel.x*p.ghost.y);
    var result=vec3(0.0);
    for(var i=0u;i<u32(p.controls.z);i++) {
        let offset=fract(coord+ghost*f32(i));
        let weight=pow(1.0-length(vec2(0.5)-offset)/sqrt(0.5),10.0);
        result+=flare_distort(offset,direction,distortion)*weight;
    }
    let profile_uv=vec2(length(vec2(0.5)-coord)/sqrt(0.5),0.5);
    let a=textureSampleLevel(profile_from,linear_clamp,profile_uv,0.0).rgb;
    let b=textureSampleLevel(profile_to,linear_clamp,profile_uv,0.0).rgb;
    result*=mix(a,b,p.controls.x);
    let halo_uv=fract(coord+halo);
    let weight=pow(1.0-length(vec2(0.5)-halo_uv)/sqrt(0.5),10.0)*p.ghost.w;
    result+=flare_distort(halo_uv,direction,distortion)*weight;
    if(p.controls.w!=0.0) {result=vec3(dot(result,vec3(0.2126,0.7152,0.0722)));}
    return vec4(result,1.0);
}
fn flare_blur(uv:vec2<f32>)->vec4<f32> {
    let count=u32(p.controls.y)/2u;
    // The small-radius branch deliberately samples unscaled UVs.
    if(count<=1u) {return textureSampleLevel(image,linear_mirror,uv,0.0);}
    let sigma=p.controls.y/8.0;
    var increment=vec3(1.0/(sqrt(6.2831853071795864769)*sigma),exp(-0.5/(sigma*sigma)),0.0);
    increment.z=increment.y*increment.y;
    let texel=vec2(1.0)/vec2<f32>(textureDimensions(image));
    let limit=p.view.zw-p.view.xy*0.5;
    var result=textureSampleLevel(image,linear_mirror,clamp(uv*p.view.zw,vec2(0.0),limit),0.0)*increment.x;
    for(var i=1u;i<count;i++) {
        increment=vec3(increment.xy*increment.yz,increment.z);
        let offset=f32(i)*p.direction.xy*texel;
        result+=textureSampleLevel(image,linear_mirror,clamp((uv-offset)*p.view.zw,vec2(0.0),limit),0.0)*increment.x;
        result+=textureSampleLevel(image,linear_mirror,clamp((uv+offset)*p.view.zw,vec2(0.0),limit),0.0)*increment.x;
    }
    // The source does not normalize the truncated Gaussian.
    return result;
}
struct FlareVertex {@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,}
@vertex fn flare_vertex(@builtin(vertex_index) i:u32)->FlareVertex {
    let uv=vec2<f32>(f32((i<<1u)&2u),f32(i&2u));
    return FlareVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
@fragment fn threshold_fragment(v:FlareVertex)->@location(0) vec4<f32> {return flare_threshold(v.uv);}
@fragment fn generate_fragment(v:FlareVertex)->@location(0) vec4<f32> {return flare_generate(v.uv);}
@fragment fn blur_fragment(v:FlareVertex)->@location(0) vec4<f32> {return flare_blur(v.uv);}
