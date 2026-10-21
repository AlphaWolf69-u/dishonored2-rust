// arkOceanForPhysics. Compose after the shared noise and surface helpers.
// Only scheduling/readback belongs to the host; the query math stays shared.
struct OceanHeightQuery {
    count: u32,
    water_level: f32,
    padding: vec2<u32>,
}
@group(0) @binding(0) var<uniform> surface: OceanSurfaceParameters;
@group(0) @binding(1) var<uniform> query: OceanHeightQuery;
@group(0) @binding(2) var permutation: texture_2d<f32>;
@group(0) @binding(3) var gradients: texture_2d<f32>;
@group(0) @binding(4) var point_wrap: sampler;
@group(0) @binding(5) var bilinear_wrap: sampler;
@group(0) @binding(6) var bilinear_clamp: sampler;
@group(0) @binding(7) var shore_distance: texture_2d<f32>;
@group(0) @binding(8) var displacement: texture_2d<f32>;
@group(0) @binding(9) var collision_height: texture_2d<f32>;
@group(0) @binding(10) var foam: texture_2d<f32>;
@group(0) @binding(11) var wave_amplitude: texture_2d<f32>;
@group(0) @binding(12) var<storage, read> positions: array<vec2<f32>>;
// HLSL StructuredBuffer { float mHeight; float3 mNormal; } is packed into
// 16 bytes. A WGSL struct with a vec3 after a scalar would have 32-byte stride.
@group(0) @binding(13) var<storage, read_write> evaluations: array<vec4<f32>>;

@compute @workgroup_size(64, 1, 1)
fn ocean_height_query(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= query.count { return; }
    let position = positions[id.x];
    var global_uv = position / surface.spatial.w + vec2(0.5);
    global_uv.x = 1.0 - global_uv.x;
    // The source samples this mask once and reuses it for both offset queries.
    let amplitude = textureSampleLevel(wave_amplitude, bilinear_clamp, global_uv, 0.0).x;
    let epsilon = 0.098;
    var input = OceanSurfaceInput(position, 0.0, 0.0, amplitude, true, 0.0);
    let origin = ocean_surface_displacement(surface, input, permutation, gradients,
        point_wrap, bilinear_wrap, bilinear_clamp, shore_distance, displacement, collision_height, foam);
    input.world_position = position + vec2(epsilon, 0.0);
    let dx = ocean_surface_displacement(surface, input, permutation, gradients,
        point_wrap, bilinear_wrap, bilinear_clamp, shore_distance, displacement, collision_height, foam);
    input.world_position = position + vec2(0.0, epsilon);
    let dy = ocean_surface_displacement(surface, input, permutation, gradients,
        point_wrap, bilinear_wrap, bilinear_clamp, shore_distance, displacement, collision_height, foam);
    let normal = normalize(vec3(origin.displacement.z - dx.displacement.z,
        origin.displacement.z - dy.displacement.z, epsilon));
    evaluations[id.x] = vec4(origin.displacement.z + query.water_level, normal);
}
