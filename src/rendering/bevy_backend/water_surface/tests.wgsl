
struct Params {reflection:WaterReflectionParameters,refraction:WaterRefractionParameters,
shadow:WaterShadowParameters,surface:WaterSurfaceParameters,sky_radius:vec4<f32>,environment:vec4<u32>,}
struct Query {position:vec4<f32>,view_position:vec4<f32>,normal:vec4<f32>,view_direction:vec4<f32>,
pixel:vec4<f32>,color:vec4<f32>,sun_direction:vec4<f32>,sun_intensity:vec4<f32>,foam:vec4<f32>,indices:vec4<u32>,weights:vec4<f32>,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var opaque:texture_depth_2d;
@group(0) @binding(2) var pyramid:texture_2d<f32>;
@group(0) @binding(3) var hdr:texture_2d<f32>;
@group(0) @binding(4) var fog:texture_2d<f32>;
@group(0) @binding(5) var probes:texture_cube_array<f32>;
@group(0) @binding(6) var sky:texture_cube<f32>;
@group(0) @binding(7) var caustics:texture_2d_array<f32>;
@group(0) @binding(8) var shadow:texture_depth_2d_array;
@group(0) @binding(9) var<storage,read> queries:array<Query>;
@group(0) @binding(10) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(11) var linear_clamp:sampler;
@group(0) @binding(12) var probe_sampler:sampler;
@group(0) @binding(13) var sky_sampler:sampler;
@group(0) @binding(14) var trilinear_wrap:sampler;
@group(0) @binding(15) var comparison:sampler_comparison;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];
    let env=WaterEnvironment(p.sky_radius.xyz,p.sky_radius.w,p.environment.x!=0u,p.environment.y!=0u,p.environment.z!=0u);
    let input=WaterSurfaceInput(q.view_position.w!=0.0,q.normal.w!=0.0,q.position.w,q.pixel.xy,
        q.position.xyz,q.view_position.xyz,q.normal.xyz,q.view_direction.xyz,q.color.xyz,q.color.w,
        q.sun_direction.w,q.sun_direction.xyz,q.sun_intensity.xyz,q.view_direction.w,q.sun_intensity.w,
        q.pixel.z,q.pixel.w,q.foam.x,q.foam.y,q.foam.z,q.indices.xyz,q.weights.xyz);
    output[id.x]=water_surface(p.surface,p.reflection,p.refraction,p.shadow,env,input,
        opaque,pyramid,hdr,fog,caustics,probes,sky,shadow,linear_clamp,trilinear_wrap,probe_sampler,sky_sampler,comparison);
}
