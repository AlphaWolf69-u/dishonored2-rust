// Original tlf/cull_lights: 64 lanes, one group per tile, 48-byte light packet.
// The caller owns head/counter initialization and sufficient pool capacity.
// No guessed live buffer allocation/reset policy is supplied by this shader.
struct LightCull {
    view_radius: vec4<f32>,
    direction_scatter: vec4<f32>,
    spot: vec3<f32>,
    flags: u32,
}
struct Params {
    uv_to_ray: vec4<f32>,
    inv_tile_dimensions: vec4<f32>,
    view: array<vec4<f32>, 4>,
    projection: array<vec4<f32>, 4>,
    light_count: u32,
    gather_scattering: u32,
    padding: vec2<u32>,
}
@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_minmax: texture_2d<f32>;
@group(0) @binding(2) var<storage, read> lights: array<LightCull>;
@group(0) @binding(3) var<storage, read_write> heads: array<atomic<u32>>;
@group(0) @binding(4) var<storage, read_write> free_node: atomic<u32>;
@group(0) @binding(5) var<storage, read_write> pool: array<u32>;
const INF: f32 = 1e20;
fn sign_fast(v: f32) -> f32 { return f32(1 - 2 * i32(bitcast<u32>(v) >> 31u)); }
fn ray_xy(uv: vec2<f32>) -> vec2<f32> {
    return fma(fma(uv, vec2<f32>(2.0,-2.0), vec2<f32>(-1.0,1.0)), params.uv_to_ray.zw, params.uv_to_ray.xy);
}
fn ndc_uv(xy: vec2<f32>) -> vec2<f32> {
    return fma(xy, vec2<f32>(0.5,-0.5), vec2<f32>(0.5));
}
fn scattering_params(rmin: f32, cosa: f32, x: vec3<f32>, s: vec3<f32>) -> vec4<f32> {
    let c2=cosa*cosa;
    let delta=-rmin*cosa*inverseSqrt(1.0-c2);
    let p=fma(s,vec3<f32>(delta),x);
    let sd=dot(s,p);
    return vec4<f32>(c2,fma(c2,delta,-sd),fma(sd,sd,-c2*dot(p,p)),delta);
}
fn omni_trace(x: vec3<f32>, param: vec4<f32>, center: vec2<f32>) -> vec2<f32> {
    let halfpixel=params.inv_tile_dimensions.xy*0.5;
    let delta=param.xy-center;
    let uv=center+vec2<f32>(sign_fast(delta.x),sign_fast(delta.y))*halfpixel;
    let ray=vec3<f32>(ray_xy(uv),1.0);
    let invlen=inverseSqrt(dot(ray,ray));
    let rd=dot(ray,x)*invlen;
    let h=sqrt(max(fma(rd,rd,param.z),0.0));
    let sphere=select((vec2<f32>(rd)+h*vec2<f32>(-1.0,1.0))*invlen,vec2<f32>(INF,-INF),h==0.0);
    return select(vec2<f32>(x.z)+param.w*vec2<f32>(-1.0,1.0),sphere,any(abs(delta)>halfpixel));
}
fn spot_two_rays(v0: vec3<f32>, v1: vec3<f32>, l: vec3<f32>, sphere: f32,
                 s: vec3<f32>, ls: f32, param: vec4<f32>) -> vec2<f32> {
    let vs=vec2<f32>(dot(v0,s),dot(v1,s));
    let lv=vec2<f32>(dot(v0,l),dot(v1,l));
    let a=vec2<f32>(param.x)-vs*vs;
    let b0=fma(vec2<f32>(param.y),vs,param.x*lv);
    let cone=fma(b0,b0,param.z*a);
    let sph=fma(lv,lv,vec2<f32>(sphere));
    let d=sqrt(max(vec4<f32>(cone.x,sph.x,cone.y,sph.y),vec4<f32>(0.0)));
    let b=b0/a;
    let cd=d.xz/a;
    var u=b.xxyy+cd.xxyy*vec4<f32>(1.0,-1.0,1.0,-1.0);
    let test=fma(vs.xxyy,u,vec4<f32>(-param.w));
    let far=select(vec2<f32>(-INF),vec2<f32>(INF),vs>vec2<f32>(0.0));
    u=select(far.xxyy,u,test>vec4<f32>(0.0));
    let v=lv.xxyy+d.yyww*vec4<f32>(-1.0,1.0,-1.0,1.0);
    var near=max(min(u.xz,u.yw),v.xz);
    var distant=min(max(u.xz,u.yw),v.yw);
    let tp=-INF*vec2<f32>(sign_fast(vs.x),sign_fast(vs.y));
    if ls>0.0 { near=max(near,tp); }
    else { distant=select(distant,min(distant,tp),tp>vec2<f32>(0.0)); }
    var t=vec4<f32>(near.x,distant.x,near.y,distant.y)*vec4<f32>(v0.z,v0.z,v1.z,v1.z);
    if d.x==0.0 || d.y==0.0 {t=vec4<f32>(INF,-INF,t.z,t.w);}
    if d.z==0.0 || d.w==0.0 {t=vec4<f32>(t.x,t.y,INF,-INF);}
    return vec2<f32>(min(t.x,t.z),max(t.y,t.w));
}
fn inside(a: f32, b: f32, mask: u32, minmax: vec2<f32>) -> bool {
    let f=fma(minmax,vec2<f32>(a),vec2<f32>(b));
    if f.x<=1.0 && f.y>=0.0 {
        let bit=vec2<u32>(1u)<<vec2<u32>(trunc(clamp(f,vec2<f32>(0.0),vec2<f32>(1.0))*31.999));
        return (mask&(((bit.y-1u)|bit.y)&~(bit.x-1u)))!=0u;
    }
    return false;
}
@compute @workgroup_size(64)
fn cull(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32) {
    let tile=group.xy;
    let raw=textureLoad(depth_minmax,vec2<i32>(tile),0);
    let size=params.inv_tile_dimensions.xy;
    let uv=vec2<f32>(tile)*size;
    let center=uv+size*0.5;
    let rect=vec4<f32>(ray_xy(uv),ray_xy(uv+size));
    for(var index=thread;index<params.light_count;index+=64u) {
        let light=lights[index];
        var near=-raw.x;
        var far=-raw.z;
        var mask=bitcast<u32>(raw.w);
        if params.gather_scattering!=0u {
            if light.direction_scatter.w<=0.0 || light.view_radius.w<=0.0 {continue;}
            near=0.0;far=-raw.y;mask=0xffffffffu;
        } else {
            if (light.flags&8u)!=0u || light.view_radius.w<=0.0 {continue;}
        }
        let a=1.0/(far-near);
        let b=-near*a;
        let kind=(light.flags>>4u)&7u;
        var cosa=inverseSqrt(fma(light.spot.z,light.spot.z,1.0));
        var x=light.view_radius.xyz;
        x.z=-x.z;
        let world_s=select(light.direction_scatter.xyz,-light.direction_scatter.xyz,kind==2u);
        let direction=vec4<f32>(world_s,0.0);
        var s=vec3<f32>(dot(params.view[0],direction),dot(params.view[1],direction),dot(params.view[2],direction));
        s.z=-s.z;
        let conservative=max(x.z,0.0)*params.uv_to_ray.z*size.x;
        var r=light.view_radius.w+conservative;
        let position=vec4<f32>(light.view_radius.xyz,1.0);
        let projected=vec3<f32>(dot(params.projection[0],position),dot(params.projection[1],position),dot(params.projection[3],position));
        let projected_uv=ndc_uv(projected.xy/projected.zz);
        var minmax:vec2<f32>;
        if kind==0u {
            minmax=omni_trace(x,vec4<f32>(projected_uv,r*r-dot(x,x),r),center);
        } else {
            if kind==1u || kind==3u {
                cosa=cos(atan(1.0/0.1));
                x-=s*r*0.1;r+=r*0.1;
            }
            x-=s*conservative;
            r+=conservative;
            let sphere=r*r-dot(x,x);
            let ls=dot(x,s);
            var param=scattering_params(conservative,cosa,x,s);
            param.w+=ls;
            minmax=spot_two_rays(normalize(vec3<f32>(rect.xy,1.0)),normalize(vec3<f32>(rect.zw,1.0)),x,sphere,s,ls,param);
            let second=spot_two_rays(normalize(vec3<f32>(rect.xw,1.0)),normalize(vec3<f32>(rect.zy,1.0)),x,sphere,s,ls,param);
            minmax=vec2<f32>(min(minmax.x,second.x),max(minmax.y,second.y));
            if all(abs(projected_uv-center)<size*0.5) {
                minmax=vec2<f32>(min(minmax.x,x.z),max(minmax.y,x.z));
            }
        }
        if inside(a,b,mask,minmax) {
            let entry=atomicAdd(&free_node,1u);
            let tiles=vec2<u32>(params.inv_tile_dimensions.zw);
            let width=tiles.x*select(4u,3u,params.gather_scattering!=0u);
            let old=atomicExchange(&heads[tile.y*width+tile.x+kind*tiles.x],entry);
            pool[entry]=(old<<9u)|index;
        }
    }
}
