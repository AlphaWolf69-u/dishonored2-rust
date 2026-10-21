
@group(0) @binding(0) var<uniform> p:ColourOutputParameters;
@group(0) @binding(1) var<storage,read> queries:array<vec4<f32>>;
@group(0) @binding(2) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];output[id.x]=vec4(colour_output(p,q.xyz),q.w);
}
