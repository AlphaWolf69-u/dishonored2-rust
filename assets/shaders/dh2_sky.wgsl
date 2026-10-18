#define_import_path dh2_sky
#import dh2_surface::{surface,diffuse0,cloud_opacity,cloud_density,cloud_back_density,surface_sampler}
#import bevy_pbr::mesh_view_bindings::globals
fn sky_rand(n:vec2<f32>)->f32 {return fract(sin(dot(n,vec2(12.9898,78.233)))*43758.5453);}
fn sky_stars(position:vec3<f32>,size:f32)->vec3<f32> {
    let p=surface.sky;
    let grid=floor(position.xy/size);
    let value=sky_rand(grid+vec2(size));
    let probability=0.9+p[1].z;
    if value<=probability {return vec3(0.0);}
    let distance=length(position.xy-(size*grid+vec2(0.5*size)));
    var star=pow(abs(1.0-distance/(0.5*size)),distance);
    let blink=sin(sin(p[1].w*globals.time)+abs(value-probability)/(1.0-probability)*45.0);
    star*=blink*blink;
    let shift=fract(value*10000.0)-0.5;
    let shift2=fract(value*100000.0)-0.5;
    let colour=vec3(1.0-shift,1.0+(shift+shift2)*0.5,1.0+shift);
    let tint=mix(normalize(colour),vec3(1.0),0.5+shift2*0.5);
    let horizon=clamp((normalize(position).z-0.25)/0.75,0.0,1.0);
    return star*horizon*pow(p[2].x,size/30.0)*tint;
}
fn cloud_hg(cosine:f32,g:f32)->f32 {
    return (1.0-g)*(1.0-g)/(12.566370614359172*pow(1.0+g*g-2.0*g*cosine,1.5));
}
fn sky_colour(uv0:vec2<f32>,world:vec3<f32>,normal:vec3<f32>,vertex_colour:vec4<f32>,cloud:bool)->vec4<f32> {
    let p=surface.sky;
    var uv=uv0;
    if cloud {uv.x+=globals.time*p[2].y;}
    let diffuse=textureSampleGrad(diffuse0,surface_sampler,uv,dpdx(uv),dpdy(uv));
    // Original shader coordinates are Z-up. Star grid and horizon follow them.
    let position=vec3(world.x,-world.z,world.y);
    if !cloud {
        return vec4(mix(diffuse.rgb,diffuse.rgb*p[0].x,diffuse.a)
            +sky_stars(position,p[1].x)+sky_stars(position,p[1].y),1.0);
    }
    let opacity=textureSampleGrad(cloud_opacity,surface_sampler,uv,dpdx(uv),dpdy(uv)).r;
    if opacity<1.0/255.0 {discard;}
    let down=-normalize(normal).y;
    let t=clamp(down/p[2].z,0.0,1.0);
    let vertical_alpha=1.0-pow(1.0-t,4.0*t*t);
    let density=textureSampleGrad(cloud_density,surface_sampler,uv,dpdx(uv),dpdy(uv)).r;
    let extinction=exp(-density*p[2].w);
    let cosine=dot(p[5].xyz,normalize(world));
    let g=p[5].w;
    let phase=(1.5/12.566370614359172)*(1.0-g*g)
        *pow(max(1e-5,1.0+g*g-2.0*g*cosine),-1.5)*(1.0+cosine*cosine)/(2.0+g*g);
    let vertical_sun=pow(-normal.y,p[3].x);
    let back_density=textureSampleGrad(cloud_back_density,surface_sampler,uv,dpdx(uv),dpdy(uv)).r;
    let back_cosine=dot(-normalize(p[5].xyz),normalize(world));
    let back=cloud_hg(back_cosine,-(1.0-back_density))*p[3].y;
    let rim=cloud_hg(back_cosine,-(1.0-density))*p[3].z;
    let sun=p[6].rgb*(extinction*clamp(phase,0.0,1.0)+back+rim*vertical_sun);
    let ambient=diffuse.rgb*mix(vec3(p[0].x),p[6].rgb,vertical_sun)*p[3].w;
    return vec4((ambient+sun)*vertex_colour.rgb,vertical_alpha*opacity*vertex_colour.a);
}
