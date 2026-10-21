// arkScreenSpaceRayTracing: hierarchical full/half-resolution depth traversal.
// The depth pyramid stores (average, minimum, maximum, unused), in linear Z.
fn screen_ray_saturate(value:f32)->f32 {
    if (bitcast<u32>(value)&0x7fffffffu)>0x7f800000u {return 0.0;}
    return clamp(value,0.0,1.0);
}
fn screen_ray_smooth(a:f32,b:f32,value:f32)->f32 {
    let t=screen_ray_saturate((value-a)/(b-a));
    return t*t*(3.0-2.0*t);
}
fn screen_ray_thickness(depth:f32,factors:vec4<f32>,exponent:f32)->f32 {
    let t=pow(screen_ray_saturate((depth-factors.y)/(factors.w-factors.y)),exponent);
    return mix(factors.x,factors.z,t)*depth;
}
fn screen_ray_scene(pixel:vec2<u32>,mip:u32,full:texture_depth_2d,
    reduced:texture_2d<f32>,unproject:vec3<f32>,thickness:vec4<f32>,exponent:f32)->vec2<f32> {
    var range:vec2<f32>;
    if mip==0u {
        let raw=textureLoad(full,vec2<i32>(pixel),0);
        range=vec2(unproject.x/(unproject.y*raw+unproject.z));
    } else {
        range=textureLoad(reduced,vec2<i32>(pixel),i32(mip-1u)).yz;
    }
    range.y=screen_ray_thickness(range.y,thickness,exponent);
    return range;
}
struct ScreenRayHit {
    fade:f32,
    depth:f32,
    uv:vec2<f32>,
    debug:vec3<f32>,
}
fn screen_ray_trace(position:vec3<f32>,direction:vec3<f32>,max_steps:u32,
    full:texture_depth_2d,reduced:texture_2d<f32>,extent:vec2<u32>,
    unproject:vec3<f32>,thickness:vec4<f32>,exponent:f32,vertical_fade:f32,border_fade:f32,
    camera_to_clip:mat4x4<f32>)->ScreenRayHit {
    var result=ScreenRayHit(0.0,1e6,vec2(0.0),vec3(0.0));
    var fade=screen_ray_smooth(0.0,vertical_fade,-direction.z);
    if fade==0.0 {return result;}
    let end=fma(vec3(0.01),direction,position);
    var h0=camera_to_clip*vec4(position,1.0);
    var h1=camera_to_clip*vec4(end,1.0);
    let k0=1.0/h0.w;
    let k1=1.0/h1.w;
    h0=vec4(h0.xy*k0, k0, h0.w);
    h1=vec4(h1.xy*k1, k1, h1.w);
    h0.y=-h0.y;
    h1.y=-h1.y;
    let bend=1.0-screen_ray_smooth(0.5,1.0,abs(h0.x));
    h0=vec4(vec2<f32>(extent)*0.5*(vec2(1.0)+h0.xy),h0.zw);
    h1=vec4(vec2<f32>(extent)*0.5*(vec2(1.0)+h1.xy),h1.zw);
    let delta=h1.xy-h0.xy;
    var slope=(h1.xyz-h0.xyz)/max(abs(delta.x),abs(delta.y));
    slope.x=select(1e-3,slope.x,abs(slope.x)>1e-3);
    slope.y=select(1e-3,slope.y,abs(slope.y)>1e-3);
    slope.x*=bend;
    let inverse_slope=vec2(1.0)/slope.xy;
    let border=select(vec2(-1e-3),vec2(1.0+1e-3),slope.xy>vec2(0.0));
    var mip=4u;
    let pixel_size=f32(1u<<mip);
    h0=vec4(h0.xy/pixel_size,h0.zw);
    slope.z*=pixel_size;
    var max_y=f32(extent.y>>mip);
    var in_interval=false;
    var z=h0.w;
    var steps=0u;
    while steps<max_steps && h0.y>=0.0 && h0.y<max_y && h0.z>0.0 {
        let pixel=vec2<u32>(floor(h0.xy));
        let distance_to_border=vec2<f32>(pixel)+border-h0.xy;
        let times=distance_to_border*inverse_slope;
        let t=min(times.x,times.y);
        let next=fma(vec3(t),slope,h0.xyz);
        let next_z=1.0/max(1e-6,next.z);
        let range=screen_ray_scene(pixel,mip,full,reduced,unproject,thickness,exponent);
        steps+=1u;
        in_interval=z<=range.y && next_z>=range.x;
        if in_interval {
            if mip==0u {
                let s=screen_ray_saturate((range.x-z)*t/(next_z-z));
                result.depth=1.0/fma(s,slope.z,h0.z);
                break;
            }
            mip-=1u;
            h0=vec4(h0.xy*2.0,h0.zw);
            slope.z*=0.5;
            max_y=f32(extent.y>>mip);
            in_interval=false;
            continue;
        }
        h0=vec4(next,h0.w);
        z=next_z;
        if mip<4u {
            mip+=1u;
            h0=vec4(h0.xy*0.5,h0.zw);
            slope.z*=2.0;
            max_y=f32(extent.y>>mip);
        }
    }
    let uv=h0.xy/vec2<f32>(extent>>vec2(mip));
    result.uv=vec2(screen_ray_saturate(uv.x),screen_ray_saturate(uv.y));
    fade*=screen_ray_smooth(1.0001,border_fade,abs(2.0*result.uv.y-1.0));
    result.fade=select(0.0,fade,in_interval);
    result.debug=vec3(f32(steps)/f32(max_steps));
    return result;
}
