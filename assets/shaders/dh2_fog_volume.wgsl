// renderMinMax / renderVolume. Mesh positions use the host's Y-up basis;
// atmospheric directions and camera height retain the game's Z-up basis.
struct AreaFogUniform {
    view_from_local: mat4x4<f32>,
    clip_from_view: mat4x4<f32>,
    world_from_view: mat4x4<f32>,
    distance: vec4<f32>,
    color_start: vec4<f32>,
    color_end: vec4<f32>,
    density: vec4<f32>, // density factor, falloff, camera altitude, flags
    scattering_direction_phase: vec4<f32>,
    scattering_color_intensity: vec4<f32>,
    extent: vec4<f32>, // mask dimensions, destination dimensions
    ray_scale: vec4<f32>,
}
@group(0) @binding(0) var<uniform> area: AreaFogUniform;
@group(0) @binding(1) var area_depth: texture_2d<f32>;
@group(0) @binding(2) var area_interval: texture_2d<f32>;
@group(0) @binding(3) var area_clamp: sampler;
struct AreaMeshVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) view_position: vec3<f32>,
}
@vertex fn area_interval_vertex(@location(0) position:vec3<f32>) -> AreaMeshVertex {
    let view = area.view_from_local * vec4(position,1.0);
    return AreaMeshVertex(area.clip_from_view*view,view.xyz);
}
fn area_interval_value(z:f32,scene:f32,front:bool)->vec4<f32> {
    let end=min(z,scene);
    return vec4(z,1.0-end,end,select(end,-end,front));
}
@fragment fn area_interval_fragment(in:AreaMeshVertex,@builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    let uv=in.position.xy/area.extent.xy;
    let scene=textureSampleLevel(area_depth,area_clamp,uv,0.0).x;
    return area_interval_value(-in.view_position.z,scene,front);
}
struct AreaVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
}
@vertex fn area_vertex(@builtin(vertex_index) index:u32)->AreaVertex {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return AreaVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
fn area_radiance(p:AreaFogUniform,ray:vec3<f32>,interval:vec4<f32>)->vec4<f32> {
    var distance=fog_saturate((interval.w-p.distance.x)*p.distance.y);
    var color=mix(p.color_start,p.color_end,distance).rgb;
    let flags=u32(p.density.w);
    var extinction=exp(-p.distance.z*distance);
    if (flags&4u)!=0u {
        let direction=ray/length(ray);
        let start=select(0.0,interval.x,interval.x<1.0-interval.y);
        let altitude=p.density.z+length(ray)*start*direction.z;
        distance*=p.density.x;
        let falloff=p.density.y;
        let density=p.distance.z*(exp(-falloff*altitude)
            -exp(-falloff*(altitude+direction.z*distance)))/(falloff*direction.z);
        extinction=exp(-density);
    }
    if (flags&2u)!=0u {
        let phase=fog_mie_phase(dot(p.scattering_direction_phase.xyz,normalize(ray)),p.scattering_direction_phase.w);
        let scattering=p.scattering_color_intensity.rgb*(p.scattering_color_intensity.w*phase);
        color+=scattering*fog_saturate(max(max(scattering.r,scattering.g),scattering.b));
    }
    return vec4(color,1.0-extinction);
}
@fragment fn area_fragment(in:AreaVertex)->@location(0) vec4<f32> {
    let interval=textureSampleLevel(area_interval,area_clamp,in.uv,0.0);
    if interval.w<0.001 {discard;}
    let cs=vec3((in.uv*vec2(2.0,-2.0)+vec2(-1.0,1.0))*area.ray_scale.xy,-1.0);
    let host=(area.world_from_view*vec4(cs,0.0)).xyz;
    return area_radiance(area,vec3(host.x,-host.z,host.y),interval);
}
