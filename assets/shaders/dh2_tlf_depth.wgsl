// Installed tlf/gatherdepthminmax, single-sample branch (8x4 threads).
// The original combines separate opaque and alpha depth, omits sky from
// the 32-bin mask, and expands the two finite endpoints by one float ULP.
struct DepthParams {
    unproject_z: vec4<f32>,
    tile: vec4<u32>,
};
@group(0) @binding(0) var opaque_depth: texture_depth_2d;
@group(0) @binding(1) var alpha_depth: texture_depth_2d;
@group(0) @binding(2) var bilinear_clamp: sampler;
@group(0) @binding(3) var<uniform> params: DepthParams;
@group(0) @binding(4) var tile_depth: texture_storage_2d<rgba32float, write>;
var<workgroup> near_depth: atomic<u32>;
var<workgroup> far_with_sky: atomic<u32>;
var<workgroup> far_without_sky: atomic<u32>;
var<workgroup> occupied_bins: atomic<u32>;

fn depth_mask(depth: vec4<f32>, a: f32, b: f32) -> u32 {
    // Preserve the original float4 conversion and MAD, not four scalar
    // divisions: backend rounding is observable on constant-depth tiles.
    let z = -vec4<f32>(params.unproject_z.x) / (depth + vec4<f32>(params.unproject_z.y));
    let bins = vec4<u32>(trunc(clamp(fma(z, vec4<f32>(a), vec4<f32>(b)), vec4<f32>(0.0), vec4<f32>(1.0)) * 31.999));
    let bits = select(vec4<u32>(0u), vec4<u32>(1u) << bins, depth != vec4<f32>(0.0));
    return bits.x | bits.y | bits.z | bits.w;
}
@compute @workgroup_size(8, 4, 1)
fn gather_depth(@builtin(workgroup_id) tile: vec3<u32>,
                @builtin(local_invocation_id) thread: vec3<u32>,
                @builtin(local_invocation_index) index: u32) {
    if index == 0u {
        atomicStore(&near_depth, bitcast<u32>(0.0));
        atomicStore(&far_with_sky, bitcast<u32>(1.0));
        atomicStore(&far_without_sky, bitcast<u32>(1.0));
        atomicStore(&occupied_bins, 0u);
    }
    // WGSL needs explicit workgroup synchronization even for a 32-lane group.
    workgroupBarrier();
    let uv = (vec2<f32>(tile.xy * params.tile.x + thread.xy * 2u)
        + vec2<f32>(0.5)) / vec2<f32>(textureDimensions(opaque_depth));
    let depth = array<vec4<f32>, 4>(
        textureGather(opaque_depth, bilinear_clamp, uv),
        textureGather(opaque_depth, bilinear_clamp, uv, vec2<i32>(0, 8)),
        textureGather(alpha_depth, bilinear_clamp, uv),
        textureGather(alpha_depth, bilinear_clamp, uv, vec2<i32>(0, 8)));
    for (var block = 0u; block < 4u; block++) {
        for (var lane = 0u; lane < 4u; lane++) {
            let d = depth[block][lane];
            atomicMax(&near_depth, bitcast<u32>(d));
            atomicMin(&far_with_sky, bitcast<u32>(d));
            atomicMin(&far_without_sky, bitcast<u32>(max(d, select(0.0, 1.0, d <= 0.0))));
        }
    }
    workgroupBarrier();
    let minmax_d = vec3<f32>(bitcast<f32>(atomicLoad(&near_depth)),
        bitcast<f32>(atomicLoad(&far_with_sky)), bitcast<f32>(atomicLoad(&far_without_sky)));
    let minmax_z = -vec3<f32>(params.unproject_z.x) / (minmax_d + vec3<f32>(params.unproject_z.y));
    let near_z = bitcast<f32>(bitcast<u32>(minmax_z.x) - 1u);
    let sky_z = minmax_z.y;
    let far_z = bitcast<f32>(bitcast<u32>(minmax_z.z) + 1u);
    let a = 1.0 / (far_z - near_z);
    let b = -near_z * a;
    var mask = 0u;
    for (var block = 0u; block < 4u; block++) { mask |= depth_mask(depth[block], a, b); }
    atomicOr(&occupied_bins, mask);
    workgroupBarrier();
    if index == 0u {
        textureStore(tile_depth, vec2<i32>(tile.xy),
            vec4<f32>(-near_z, -sky_z, -far_z, bitcast<f32>(atomicLoad(&occupied_bins))));
    }
}
