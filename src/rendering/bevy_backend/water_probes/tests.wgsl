@group(0) @binding(0) var<uniform> options:vec4<u32>;
struct ProbeQuery {weighted:vec4<f32>,bary:vec4<f32>,}
@group(0) @binding(3) var<storage,read> queries:array<ProbeQuery>;
@group(0) @binding(4) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];
    let vertex=water_probe_vertex(options.w,q.bary.z);
    let fragment=water_probe_pixel(options.x!=0u,options.y!=0u,options.z,q.weighted.xyz,q.bary.xy);
    let at=id.x*3u;
    output[at]=vec4(vertex.weighted_indices,0.0);
    output[at+1u]=vec4(vertex.barycentrics,fragment.barycentrics.xy);
    output[at+2u]=vec4(vec3<f32>(fragment.indices),fragment.barycentrics.z);
}
