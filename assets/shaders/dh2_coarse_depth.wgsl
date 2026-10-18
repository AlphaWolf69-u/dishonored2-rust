// Native coarseDepthGen: frame82270 event26060 / CS19745,
// DXBC 17809935-776cb746-521c51df-a9103778 (34 instructions).
// One invocation per destination texel, not a parallel reduction. Source is
// the caller's last downsampled-depth mip; X/Z contain its average/max values.
struct CoarseDepthParameters {
    source_max: vec2<u32>,
    source_per_destination: vec2<f32>,
    // Logical inverseProjection[2].w, inverseProjection[3].w,
    // inverseProjection[3].z; W is unused. DXBC CB register indices are
    // column-major and must not be mistaken for these logical CPU rows.
    inverse_projection: vec4<f32>,
}
@group(0) @binding(0) var source_depth: texture_2d<f32>;
@group(0) @binding(1) var coarse_depth: texture_storage_2d<rg32float, write>;
@group(0) @binding(2) var<uniform> parameters: CoarseDepthParameters;

@compute @workgroup_size(1, 1, 1)
fn generate_coarse_depth(@builtin(workgroup_id) destination: vec3<u32>) {
    let start = vec2<u32>(floor(vec2<f32>(destination.xy) * parameters.source_per_destination));
    let count = vec2<i32>(ceil(parameters.source_per_destination));
    var interval = vec2<f32>(99999.0, 0.0);
    for (var x = 0; x < count.x; x += 1) {
        for (var y = 0; y < count.y; y += 1) {
            let point = min(start + vec2<u32>(u32(x), u32(y)), parameters.source_max);
            let depths = textureLoad(source_depth, vec2<i32>(point), 0).xz;
            interval = vec2<f32>(min(depths.x, interval.x), max(depths.y, interval.y));
        }
    }
    // DXBC computes both divisions before MOVC. Zero (including -0) is sky
    // and becomes exactly 1; there is deliberately no saturation here.
    let projected = ((-parameters.inverse_projection.x / interval)
        + parameters.inverse_projection.y) / parameters.inverse_projection.z;
    let result = select(projected, vec2<f32>(1.0), interval == vec2<f32>(0.0));
    textureStore(coarse_depth, vec2<i32>(destination.xy), vec4<f32>(result, result.xx));
}
