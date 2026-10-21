struct WaterVertexQuery {position_normal_w:vec4<f32>,indices:vec4<u32>,}
@group(0) @binding(3) var<storage,read> queries:array<WaterVertexQuery>;
@group(0) @binding(4) var<storage,read_write> output:array<vec4<f32>>;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];
    let v=water_world_colour_input(q.position_normal_w.xyz,q.position_normal_w.w,q.indices.x,q.indices.y);
    output[3u*id.x]=v.indices_world_x;
    output[3u*id.x+1u]=v.world_yz_barycentrics;
    output[3u*id.x+2u]=v.clip;
}
