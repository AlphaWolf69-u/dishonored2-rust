#define_import_path bevy_pbr::forward_io

struct Vertex {
#ifdef DH2_NORMAL_W
    @location(9) normal_w: f32,
#endif
    @builtin(instance_index) instance_index: u32,
#ifdef VERTEX_POSITIONS
    @location(0) position: vec3<f32>,
#endif
#ifdef VERTEX_NORMALS
    @location(1) normal: vec3<f32>,
#endif
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
#ifdef VERTEX_TANGENTS
    @location(4) tangent: vec4<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
#ifdef DH2_AUTHORED_BASIS
    @location(8) bitangent: vec3<f32>,
#endif
#ifdef DH2_VEGETATION_WIND
    // Original local POSITION, NORMAL.xy and COLOR.xyz for AnimateVegetation.
    @location(10) vegetation_position: vec3<f32>,
    @location(11) vegetation_normal: vec2<f32>,
    @location(12) vegetation_color: vec3<f32>,
#endif
#ifdef DH2_MODEL_DEPTH_HACK
    // The render model's cb_modeldepthhack (positive-hack models only).
    @location(13) model_depth_hack: f32,
#endif
#ifdef SKINNED
    @location(6) joint_indices: vec4<u32>,
    @location(7) joint_weights: vec4<f32>,
#endif
#ifdef DH2_VERTEX_PROBE
    @builtin(vertex_index) index: u32,
#else
#ifdef MORPH_TARGETS
    @builtin(vertex_index) index: u32,
#endif
#endif
};

struct VertexOutput {
#ifdef DH2_FOG_VERTEX
    @location(12) fog_color: vec4<f32>,
#endif
    // This is `clip position` when the struct is used as a vertex stage output
    // and `frag coord` when used as a fragment stage input
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
#ifdef VERTEX_UVS_A
    @location(2) uv: vec2<f32>,
#endif
#ifdef VERTEX_UVS_B
    @location(3) uv_b: vec2<f32>,
#endif
#ifdef VERTEX_TANGENTS
    @location(4) world_tangent: vec4<f32>,
#endif
#ifdef VERTEX_COLORS
    @location(5) color: vec4<f32>,
#endif
#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    @location(6) @interpolate(flat) instance_index: u32,
#endif
#ifdef DH2_AUTHORED_BASIS
    @location(8) world_bitangent: vec3<f32>,
#endif
#ifdef VISIBILITY_RANGE_DITHER
    @location(7) @interpolate(flat) visibility_range_dither: i32,
#endif
#ifdef DH2_VERTEX_PROBE
    @location(9) probe_luminance: f32,
    @location(10) probe_chroma_a: vec4<f32>,
    @location(11) probe_chroma_b: vec4<f32>,
    @location(13) reflection_indices_gain: vec4<f32>,
    @location(14) reflection_barycentrics: vec2<f32>,
#endif
}

struct FragmentOutput {
    @location(0) color: vec4<f32>,
}
