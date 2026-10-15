// Installed downsampling/depthreduction.decl, dispatched by FUN_1402fbe40.
// Four RGBA16F mips: average, minimum, maximum, zero. Later levels reduce
// full-precision shared values, NOT the quantized values stored in each mip.
struct Parameters {
    source_size: vec2<u32>,
    padding: vec2<u32>,
    // inverseProjection[2].w, inverseProjection[3].z/w, unused.
    projection: vec4<f32>,
}
@group(0) @binding(0) var source: texture_depth_2d;
@group(0) @binding(1) var mip0: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var mip1: texture_storage_2d<rgba16float, write>;
@group(0) @binding(3) var mip2: texture_storage_2d<rgba16float, write>;
@group(0) @binding(4) var mip3: texture_storage_2d<rgba16float, write>;
@group(0) @binding(5) var<uniform> parameters: Parameters;
var<workgroup> samples: array<vec4<f32>, 64>;

fn linear_z(position: vec2<u32>) -> f32 {
    let raw = textureLoad(source, vec2<i32>(position), 0);
    return -parameters.projection.x / (parameters.projection.y * raw + parameters.projection.z);
}
fn reduce(a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(0.25 * (((a.x + b.x) + c.x) + d.x),
        min(min(min(a.y, b.y), c.y), d.y),
        max(max(max(a.z, b.z), c.z), d.z), 0.0);
}
fn sample_value(z: f32) -> vec4<f32> { return vec4<f32>(z, z, z, 0.0); }

@compute @workgroup_size(8, 8, 1)
fn reduce_depth(@builtin(workgroup_id) group: vec3<u32>,
    @builtin(local_invocation_id) thread: vec3<u32>,
    @builtin(local_invocation_index) index: u32) {
    let p00 = min((group.xy * 8u + thread.xy) * 2u, parameters.source_size - 1u);
    let p11 = min(p00 + 1u, parameters.source_size - 1u);
    let p01 = vec2<u32>(p11.x, p00.y);
    let p10 = vec2<u32>(p00.x, p11.y);
    samples[index] = reduce(sample_value(linear_z(p00)), sample_value(linear_z(p01)),
        sample_value(linear_z(p10)), sample_value(linear_z(p11)));
    textureStore(mip0, vec2<i32>(group.xy * 8u + thread.xy), samples[index]);
    workgroupBarrier();

    let first = index * 2u;
    var next = vec4<f32>(0.0);
    if all(thread.xy < vec2<u32>(4u)) {
        next = reduce(samples[first], samples[first + 1u], samples[first + 8u], samples[first + 9u]);
    }
    // HLSL relied on lockstep execution of the surviving lanes. Portable WGSL
    // must finish all reads before any lane overwrites the shared source.
    workgroupBarrier();
    if all(thread.xy < vec2<u32>(4u)) {
        samples[index] = next;
        textureStore(mip1, vec2<i32>(group.xy * 4u + thread.xy), next);
    }
    workgroupBarrier();
    if all(thread.xy < vec2<u32>(2u)) {
        next = reduce(samples[first], samples[first + 1u], samples[first + 8u], samples[first + 9u]);
    }
    workgroupBarrier();
    if all(thread.xy < vec2<u32>(2u)) {
        samples[index] = next;
        textureStore(mip2, vec2<i32>(group.xy * 2u + thread.xy), next);
    }
    workgroupBarrier();
    if index == 0u {
        next = reduce(samples[0], samples[1], samples[8], samples[9]);
        textureStore(mip3, vec2<i32>(group.xy), next);
    }
}
