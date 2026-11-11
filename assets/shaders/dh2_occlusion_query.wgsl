// Installed renderprog/occlusionquery (OcclusionQuery_0.material): the unit
// quad placed in clip space by OcclusionQuery/Transform1 (x and y axes) and
// OcclusionQuery/Transform2 (projected centre and depth). Depth only; the
// colour branch exists only for r_showOcclusionQuery (DebugDraw).
struct OcclusionQuery {
    transform1: vec4<f32>,
    transform2: vec4<f32>,
};
@group(0) @binding(0) var<uniform> query: OcclusionQuery;

@vertex
fn occlusion_query_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // The shared unit quad's corners, as a triangle strip.
    let corner = vec2<f32>(select(-1.0, 1.0, (index & 1u) != 0u), select(-1.0, 1.0, (index & 2u) != 0u));
    let xy = corner.xx * query.transform1.xy + corner.yy * query.transform1.zw + query.transform2.xy;
    return vec4<f32>(xy, query.transform2.z, 1.0);
}
