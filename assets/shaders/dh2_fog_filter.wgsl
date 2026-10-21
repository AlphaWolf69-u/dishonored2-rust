// atm/distanceFog/renderBilateralBlur. Retain its Gaussian depth coordinates
// as authored, including the asymmetric horizontal/vertical depth fetches.
override FOG_GAUSSIAN:bool=false;
override FOG_GODRAYS:bool=false;
const GAUSSIAN_WEIGHTS=array<f32,4>(0.1964825502,0.2969069647,0.0944703979,0.0103813624);
const GAUSSIAN_OFFSETS=array<f32,4>(0.0,1.4117647059,3.2941176471,5.1764705882);
struct FogBlurUniform {ratio:vec4<f32>,offset:vec4<f32>,misc:vec4<f32>,}
@group(0) @binding(0) var<uniform> blur:FogBlurUniform;
@group(0) @binding(1) var source:texture_2d<f32>;
@group(0) @binding(2) var depth:texture_2d<f32>;
@group(0) @binding(3) var rays:texture_2d<f32>;
@group(0) @binding(4) var linear_clamp:sampler;
@group(0) @binding(5) var point_clamp:sampler;
struct Vertex { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32>, }
@vertex fn vertex(@builtin(vertex_index) index:u32)->Vertex {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return Vertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
fn fetch_depth(uv:vec2<f32>)->f32 {
    if FOG_GAUSSIAN {return textureSampleLevel(depth,linear_clamp,uv,0.0).r;}
    return textureSampleLevel(depth,point_clamp,uv,0.0).g;
}
fn gaussian_depth_weight(uv:vec2<f32>,center:f32,sharpness:f32)->f32 {
    let difference=fetch_depth(uv)-center;
    return exp(-difference*difference*sharpness)+0.001;
}
@fragment fn fragment(in:Vertex)->@location(0) vec4<f32> {
    let center=fetch_depth(in.uv);
    var sharpness=mix(blur.ratio.z,blur.ratio.w,clamp(center/blur.misc.x,0.0,1.0));
    if FOG_GODRAYS {sharpness=mix(sharpness,blur.misc.y,textureSampleLevel(rays,linear_clamp,in.uv,0.0).r);}
    var color=vec4(0.0);var total=0.0;
    if FOG_GAUSSIAN {
        color=textureSampleLevel(source,linear_clamp,in.uv,0.0)*GAUSSIAN_WEIGHTS[0];total=GAUSSIAN_WEIGHTS[0];
        for(var i=1u;i<4u;i++) {
            let positive=GAUSSIAN_WEIGHTS[i]*gaussian_depth_weight(in.uv+vec2(blur.offset.x*GAUSSIAN_OFFSETS[i],0.0),center,sharpness);
            let negative=GAUSSIAN_WEIGHTS[i]*gaussian_depth_weight(in.uv-vec2(blur.offset.y*GAUSSIAN_OFFSETS[i],0.0),center,sharpness);
            color+=textureSampleLevel(source,linear_clamp,in.uv+blur.offset.xy*GAUSSIAN_OFFSETS[i],0.0)*positive;
            color+=textureSampleLevel(source,linear_clamp,in.uv-blur.offset.xy*GAUSSIAN_OFFSETS[i],0.0)*negative;
            total+=positive+negative;
        }
    } else {
        for(var radius=-blur.ratio.x;radius<=blur.ratio.x;radius+=1.0) {
            let uv=in.uv+radius*blur.offset.xy;
            let difference=fetch_depth(uv)-center;
            let weight=exp(-radius*radius*blur.ratio.y-difference*difference*sharpness)+0.001;
            color+=weight*textureSampleLevel(source,linear_clamp,uv,0.0);total+=weight;
        }
    }
    return color/total;
}
