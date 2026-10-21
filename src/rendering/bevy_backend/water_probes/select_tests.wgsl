@group(0) @binding(0) var<uniform> options:vec4<u32>;
struct ProbeQuery {position:vec4<f32>,points:array<vec4<f32>,4>,weights:array<vec4<f32>,4>,ids:vec4<u32>,}
@group(0) @binding(3) var<storage,read> queries:array<ProbeQuery>;
@group(0) @binding(4) var<storage,read_write> output:array<vec4<u32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x+options.x];
    let points=array<vec3<f32>,4>(q.points[0].xyz,q.points[1].xyz,q.points[2].xyz,q.points[3].xyz);
    let probe=water_probe_select(q.position.xyz,points,q.weights,q.ids);
    output[id.x]=vec4(probe,0u,0u,0u);
}
