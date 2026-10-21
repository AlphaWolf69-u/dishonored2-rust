
struct Params {reflection:WaterReflectionParameters,sky_radius:vec4<f32>,environment:vec4<u32>,}
struct Query {position:vec4<f32>,view_position:vec4<f32>,view_direction:vec4<f32>,normal:vec4<f32>,indices:vec4<u32>,weights:vec4<f32>,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var opaque_depth:texture_depth_2d;
@group(0) @binding(2) var reduced_depth:texture_2d<f32>;
@group(0) @binding(3) var hdr:texture_2d<f32>;
@group(0) @binding(4) var fog:texture_2d<f32>;
@group(0) @binding(5) var probes:texture_cube_array<f32>;
@group(0) @binding(6) var sky:texture_cube<f32>;
@group(0) @binding(7) var<storage,read> queries:array<Query>;
@group(0) @binding(8) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(9) var linear_clamp:sampler;
@group(0) @binding(10) var probe_sampler:sampler;
@group(0) @binding(11) var sky_sampler:sampler;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];let r=p.reflection;
    let env=WaterEnvironment(p.sky_radius.xyz,p.sky_radius.w,p.environment.x!=0u,p.environment.y!=0u,p.environment.z!=0u);
    let input=WaterReflectionInput(q.position.xyz,q.view_position.xyz,q.view_direction.xyz,q.normal.xyz,
        q.position.w,q.indices.xyz,q.weights.xyz);
    output[id.x*2u]=vec4(water_reflection(r,env,input,opaque_depth,reduced_depth,hdr,fog,probes,sky,
        linear_clamp,probe_sampler,sky_sampler),1.0);
    var details=vec4(0.0);
    if r.options.x!=0u && r.options.y!=0u && q.position.w>0.0 {
        let n=normalize(vec3(q.normal.xy,q.normal.z*r.trace.w));
        let direction=(r.view_from_world*vec4(reflect(-q.view_direction.xyz,n),0.0)).xyz;
        let extent=vec2<u32>(r.screen_hdr.xy*vec2<f32>(textureDimensions(opaque_depth)));
        let h=screen_ray_trace(q.view_position.xyz,direction,r.options.z,opaque_depth,reduced_depth,
            extent,r.unproject.xyz,r.thickness,r.trace.x,r.trace.y,r.trace.z,r.camera_to_clip);
        details=vec4(h.fade,h.depth,h.uv);
    }
    output[id.x*2u+1u]=details;
}
