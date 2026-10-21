// PC arkWater's vertex-corner encoding and fragment probe reconstruction.
struct WaterProbeVaryings {weighted_indices:vec3<f32>,barycentrics:vec2<f32>,}
struct WaterProbeInput {indices:vec3<u32>,barycentrics:vec3<f32>,}
// InterpolateSH's float2 selection retains the first slot on tied weights.
// Probe-ID conversion through float is part of the installed shader contract.
fn water_probe_select(position:vec3<f32>,positions:array<vec3<f32>,4>,
    weights:array<vec4<f32>,4>,probe_ids:vec4<u32>)->u32 {
    let w=tetrahedron_probe_weights(position,positions,weights);
    var best=vec2(w.x,f32(probe_ids.x));
    if w.y>best.x {best=vec2(w.y,f32(probe_ids.y));}
    if w.z>best.x {best=vec2(w.z,f32(probe_ids.z));}
    if w.w>best.x {best=vec2(w.w,f32(probe_ids.w));}
    return u32(best.y);
}
fn water_probe_vertex(cube_index:u32,normal_w:f32)->WaterProbeVaryings {
    let bary=vec3(shadow_saturate(-normal_w),select(0.0,1.0,abs(normal_w)<=0.5),
        shadow_saturate(normal_w));
    return WaterProbeVaryings(bary*f32(cube_index),bary.xy);
}
fn water_probe_pixel(enabled:bool,per_vertex:bool,single_cube:u32,
    weighted_indices:vec3<f32>,barycentrics_xy:vec2<f32>)->WaterProbeInput {
    // In the single-probe source variant the barycentric field is unused.
    if !per_vertex {return WaterProbeInput(vec3(select(0u,single_cube,enabled)),vec3(0.0));}
    if !enabled {return WaterProbeInput(vec3(0u),vec3(1.0));}
    let bary=vec3(barycentrics_xy,1.0-barycentrics_xy.x-barycentrics_xy.y);
    let indices=vec3<u32>(select(weighted_indices,round(weighted_indices/bary),bary>vec3(0.0)));
    return WaterProbeInput(indices,bary);
}
