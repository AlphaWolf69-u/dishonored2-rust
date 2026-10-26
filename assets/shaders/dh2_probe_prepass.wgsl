#import bevy_pbr::prepass_io::VertexOutput
#import dh2_alpha_clip::clip_surface
@fragment fn fragment(vertex_output:VertexOutput) {
    var uv0=vec2(0.0);var uv1=vec2(0.0);
#ifdef VERTEX_UVS_A
    uv0=vertex_output.uv;
#endif
#ifdef VERTEX_UVS_B
    uv1=vertex_output.uv_b;
#endif
    clip_surface(uv0,uv1,vertex_output.world_position);
}
