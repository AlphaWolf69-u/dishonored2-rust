// lightscattering/intersection helpers. Concatenate after dh2_shadow.
// tlf/light_t's 112-byte storage layout, shared by forward lighting and fog.
struct TiledLightRecord {
    position:vec3<f32>, falloff:f32,
    color:vec3<f32>, cutoff:f32,
    direction:vec3<f32>, decay:f32,
    specular:f32, scatter_value:f32, scatter_scale:f32, scatter_offset:f32,
    spot:vec3<f32>, scatter_turbulence:f32,
    axis:vec3<f32>, origin_offset:f32,
    shadow_setup:u32, flags:u32, padding:vec2<u32>,
}
// This is the working light context, decoded from the shared TLF light record.
struct FogScatterLight {
    position:vec3<f32>, cutoff:f32,
    direction:vec3<f32>, falloff:f32,
    axis:vec3<f32>, origin_offset:f32,
    color:vec3<f32>, value:f32,
    spot:vec3<f32>, scale:f32,
    offset:f32, turbulence:f32, custom_shape:u32, second_canvas:u32,
}
// LightContext::FetchLightData. Shadow setup and volumetric-shadow flag remain
// on the source record; the transparent overload never uses either one.
fn scatter_light_from_record(light:TiledLightRecord)->FogScatterLight {
    return FogScatterLight(light.position,light.cutoff,light.direction,light.falloff,
        light.axis,light.origin_offset,light.color,light.scatter_value,light.spot,
        light.scatter_scale,light.scatter_offset,light.scatter_turbulence,
        (light.flags>>2u)&1u,light.flags&1u);
}
// IntegrateScatter and PostFX/LightScatteing accumulate RGB unpremultiplied
// within each type, with alpha weighted by that sample's saturated max RGB.
fn scatter_add(accumulated:vec4<f32>,sample:vec4<f32>)->vec4<f32> {
    let luminance=shadow_saturate(max(sample.x,max(sample.y,sample.z)));
    return vec4(accumulated.xyz+sample.xyz,accumulated.w+sample.w*luminance);
}
fn scatter_resolve(point:vec4<f32>,half_point:vec4<f32>,spot:vec4<f32>)->vec4<f32> {
    let color=point.xyz+half_point.xyz+spot.xyz;
    let alpha=min(1.0,max(color.x,max(color.y,color.z)));
    return vec4(color*alpha,shadow_saturate(point.w+half_point.w+spot.w));
}
struct RayQuadratic {
    interval:vec2<f32>, hit:bool,
}
// HLSL cross emits MUL then MAD. Preserve contraction across backends: the
// low bits of these axes decide the inclined-plane apex rejection below.
fn scatter_cross(a:vec3<f32>,b:vec3<f32>)->vec3<f32> {
    return fma(a.yzx,b.zxy,-(a.zxy*b.yzx));
}
fn ray_quadratic(a:f32,b:f32,c:f32)->RayQuadratic {
    let discriminant=b*b-4.0*a*c;
    if discriminant<0.0 {return RayQuadratic(vec2(0.0),false);}
    let t=-0.5*(b+sign(b)*sqrt(discriminant));
    let closest=t/a;
    let furthest=c/t;
    return RayQuadratic(vec2(min(closest,furthest),max(closest,furthest)),true);
}
fn ray_cone_interval(origin:vec3<f32>, direction:vec3<f32>, light:FogScatterLight)->vec2<f32> {
    let y=light.direction;
    let z=normalize(scatter_cross(y,vec3(0.0,1.0,0.0)));
    let x=scatter_cross(y,z);
    let delta=origin-light.position;
    let o=vec3(dot(x,delta),dot(y,delta),dot(z,delta));
    let d=vec3(dot(x,direction),dot(y,direction),dot(z,direction));
    let tangent=light.spot.z*light.spot.z;
    let a=d.x*d.x+d.z*d.z-d.y*d.y*tangent;
    let b=2.0*(o.x*d.x+o.z*d.z-o.y*d.y*tangent);
    let c=o.x*o.x+o.z*o.z-o.y*o.y*tangent;
    var result=ray_quadratic(a,b,c).interval;
    let y1=o.y+d.y*result.x;
    let y2=o.y+d.y*result.y;
    let cap_z=-vec2(light.origin_offset,light.cutoff);
    let cap=(cap_z-vec2(o.y))/d.y;
    if y1<cap_z.y {result.x=cap.y;}
    if y1>cap_z.x {result.x=cap.x;}
    if y2<cap_z.y {result.y=cap.y;}
    if y2>cap_z.x {result.y=cap.x;}
    if y1>0.0 && y2>0.0 {
        result=vec2(0.0,-1.0);
    } else if y1>0.0 && y2<0.0 {
        result=vec2(result.y,10000.0);
    } else if y1<0.0 && y2>0.0 {
        result=vec2(0.0,result.x);
    }
    return result;
}
fn scatter_light_basis(light:FogScatterLight,scale:vec3<f32>)->mat3x3<f32> {
    let x=-light.direction;
    let z=normalize(scatter_cross(x,light.axis));
    let y=scatter_cross(x,z);
    // HLSL constructed ROWS; WGSL constructs columns.
    return transpose(mat3x3(x*scale.x,y*scale.y,z*scale.z));
}
fn ray_cube_interval(origin:vec3<f32>, direction:vec3<f32>, light:FogScatterLight)->vec2<f32> {
    let scale=vec3(1.0/light.origin_offset,1.0/light.scale,1.0);
    let basis=scatter_light_basis(light,scale);
    let o=basis*(origin-light.position);
    let d=basis*direction;
    let radius=light.cutoff/sqrt(2.0);
    let inverse=1.0/d;
    let bottom=inverse*(-radius-o);
    let top=inverse*(radius-o);
    let low=min(top,bottom);
    let high=max(top,bottom);
    var near=max(max(low.x,low.y),low.z);
    var far=min(min(high.x,high.y),high.z);
    if far<=0.0 || far<near {return vec2(0.0,-1.0);}
    if near<=0.0 {near=0.0;}
    return vec2(near,far);
}
fn ray_frustum_interval(origin:vec3<f32>, direction:vec3<f32>, light:FogScatterLight)->vec2<f32> {
    let scale=vec3(1.0,1.0/light.scale,1.0);
    let basis=scatter_light_basis(light,scale);
    let o=basis*(origin-light.position);
    let d=basis*direction;
    let aperture=light.spot.z*(1.0/3.14159265358979323846);
    var bottom:vec3<f32>;
    var top:vec3<f32>;
    let inverse_x=1.0/d.x;
    bottom.x=inverse_x*(light.origin_offset-o.x);
    top.x=inverse_x*(light.cutoff-o.x);
    // The installed shader emits MAD for each inclined-plane numerator,
    // denominator and apex test. Separate operations can change the sign of
    // an intersection at the apex, rejecting the whole scattering volume.
    let axial=aperture*d.x;
    bottom.y=fma(-aperture,o.x,-0.5*o.y)/fma(0.5,d.y,axial);
    top.y=fma(aperture,o.x,-0.5*o.y)/fma(0.5,d.y,-axial);
    bottom.z=fma(-aperture,o.x,-0.5*o.z)/fma(0.5,d.z,axial);
    top.z=fma(aperture,o.x,-0.5*o.z)/fma(0.5,d.z,-axial);
    var times=vec4(bottom.yz,top.yz);
    times=select(vec4(7000.0*sign(d.x)),times,fma(times,vec4(d.x),vec4(o.x))>vec4(0.0));
    bottom=vec3(bottom.x,times.xy);
    top=vec3(top.x,times.zw);
    let low=min(top,bottom);
    let high=max(top,bottom);
    let near=max(max(low.x,low.y),low.z);
    let far=min(min(high.x,high.y),high.z);
    if far<=0.0 || far<near {return vec2(0.0,-1.0);}
    return vec2(near,far);
}
fn scatter_light_interval(kind:u32, origin:vec3<f32>, direction:vec3<f32>, distance:f32,
    light:FogScatterLight)->vec2<f32> {
    var interval=ray_sphere_interval(origin,direction,light.position,light.cutoff);
    if light.custom_shape!=0u && kind<2u {interval=ray_cube_interval(origin,direction,light);}
    if kind==2u {
        var spot:vec2<f32>;
        if light.custom_shape!=0u {spot=ray_frustum_interval(origin,direction,light);}
        else {spot=ray_cone_interval(origin,direction,light);}
        interval=vec2(max(interval.x,spot.x),min(interval.y,spot.y));
    }
    // Unlike WGSL clamp, HLSL clamp accepts inverted bounds. Preserve its
    // min(max(value,low),high) order when the near hit is past the surface.
    interval.y=min(max(interval.y,interval.x),distance);
    return vec2(interval.x,max(0.0,interval.y-interval.x));
}
fn scatter_integral(start:vec3<f32>, direction:vec3<f32>, light:vec3<f32>, distance:f32)->f32 {
    let q=start-light;
    let b=dot(direction,q);
    let c=dot(q,q);
    let s=1.0/sqrt(max(0.0001,fma(-b,b,c)));
    return s*atan(distance/max(0.0001,s*fma(b,distance,c)));
}
fn scatter_attenuation(kind:u32, origin:vec3<f32>, direction:vec3<f32>, minimum:f32,
    distance:f32, light:FogScatterLight)->f32 {
    let middle=light.position-fma(direction,vec3(fma(distance,0.5,minimum)),origin);
    let square=dot(middle,middle);
    let cutoff=light.cutoff*light.cutoff;
    let t=shadow_saturate((square-cutoff)/(light.falloff*light.falloff-cutoff));
    var attenuation=t*t*(3.0-2.0*t);
    if kind==2u {
        let angular=1.0-shadow_saturate(fma(fma(dot(normalize(middle),light.direction),0.5,0.5),light.spot.x,light.spot.y));
        attenuation*=angular;
    }
    return attenuation*light.value;
}
fn scatter_turbulence(first:f32,second:f32,light:FogScatterLight)->f32 {
    return fma(select(first,second,light.second_canvas!=0u)-1.0,light.turbulence,1.0);
}
// The transparent SurfaceContext overload passes NO_SHADOW(9999), which
// skips both shadow taps AND canvas turbulence. The opaque owner is separate.
fn scatter_unshadowed(kind:u32, origin:vec3<f32>, direction:vec3<f32>, distance:f32,
    light:FogScatterLight)->vec4<f32> {
    let segment=scatter_light_interval(kind,origin,direction,distance,light);
    var scatter=0.0;
    if segment.y>0.0 {
        let position=light.position-light.direction*light.offset;
        scatter=scatter_integral(origin+direction*segment.x,direction,position,segment.y);
        scatter*=scatter_attenuation(kind,origin,direction,segment.x,segment.y,light);
    }
    return vec4(light.color*scatter,scatter);
}
