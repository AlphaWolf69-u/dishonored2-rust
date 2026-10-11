#import bevy_pbr::{
    prepass_io::VertexOutput,
    pbr_bindings,
    pbr_functions::alpha_discard,
}

#import dh2_surface::{surface, compiled_misc, scaled_uv, diffuse0, surface_sampler}

// arkDefault/arkVegetation Z branches return after ClipZorShadowPass:
// only diffuse0 alpha and DefaultAlphaTest, before layers, vertex color,
// gloss-in-alpha, normals, AO or lighting (arkDefaultHelpers.decl:29-40).
@fragment
fn fragment(vertex_output: VertexOutput) {
    if compiled_misc.z == 1u || compiled_misc.z == 2u {
        if (compiled_misc.w & 1u) != 0u {
            var uv0 = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
            uv0 = vertex_output.uv;
#endif
            var uv1 = vec2<f32>(0.0);
#ifdef VERTEX_UVS_B
            uv1 = vertex_output.uv_b;
#endif
            let uv = scaled_uv(select(uv0, uv1, surface.uv_sets[0].x != 0u), surface.uv[0]);
            let alpha = textureSampleGrad(diffuse0, surface_sampler, uv, dpdx(uv), dpdy(uv)).a;
            if alpha < surface.alpha.x { discard; }
        }
        return;
    }
    let material = pbr_bindings::material;
    alpha_discard(material, material.base_color);
}
