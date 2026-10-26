// arkOcean depth stages. The installed normal-depth pixel shader (31) uses
// perspective/center interpolation for both clip-position varyings.
struct OceanDepthParameters {
    surface:OceanSurfaceParameters,
    vertex:OceanVertexParameters,
}
@group(0) @binding(0) var<uniform> ocean_depth:OceanDepthParameters;
@group(1) @binding(0) var permutation:texture_2d<f32>;
@group(1) @binding(1) var gradients:texture_2d<f32>;
@group(1) @binding(2) var shore_distance:texture_2d<f32>;
@group(1) @binding(3) var current_displacement:texture_2d<f32>;
@group(1) @binding(4) var previous_displacement:texture_2d<f32>;
@group(1) @binding(5) var collision_height:texture_2d<f32>;
@group(1) @binding(6) var fft_foam:texture_2d<f32>;
@group(1) @binding(7) var amplitude_mask:texture_2d<f32>;
@group(1) @binding(8) var point_wrap:sampler;
@group(1) @binding(9) var bilinear_wrap:sampler;
@group(1) @binding(10) var bilinear_clamp:sampler;
struct OceanDepthVertex {
    // The colour pass compares depth for exact equality. Preserve the shared
    // displacement/projection arithmetic through backend shader optimization.
    @invariant @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(perspective,center) current:vec4<f32>,
    @location(1) @interpolate(perspective,center) previous:vec4<f32>,
}
@vertex fn ocean_depth_vertex(@location(0) grid:vec2<f32>)->OceanDepthVertex {
    let value=ocean_mesh_vertex(ocean_depth.surface,ocean_depth.vertex,grid,true,
        permutation,gradients,point_wrap,bilinear_wrap,bilinear_clamp,
        shore_distance,current_displacement,previous_displacement,collision_height,
        fft_foam,amplitude_mask);
    return OceanDepthVertex(value.clip,value.clip,value.previous_clip);
}
@fragment fn ocean_depth_fragment(vertex:OceanDepthVertex)->@location(0) vec4<f32> {
    return screen_motion_vector(vertex.current,vertex.previous,
        ocean_depth.vertex.perlin_end_jitter.yz);
}
