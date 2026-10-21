
struct Params {shadow:FogShadowParameters,world_from_clip:mat4x4<f32>,eye:vec4<f32>,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var depth:texture_depth_2d;
@group(0) @binding(2) var projected:texture_2d<f32>;
@group(0) @binding(3) var atlas:texture_depth_2d_array;
@group(0) @binding(4) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(5) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(6) var linear_clamp:sampler;
@group(0) @binding(7) var comparison:sampler_comparison;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];let v=fog_shadow_term(p.shadow,p.world_from_clip,p.eye.xyz,vec2<u32>(q.xy),q.zw,
        depth,projected,linear_clamp,atlas,comparison);output[id.x]=vec4(v);
}
