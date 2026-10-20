#import bevy_pbr::{
    prepass_io::VertexOutput,
    pbr_bindings,
    pbr_functions::alpha_discard,
}

#import dh2_surface::{surface, compiled_misc, scaled_uv, diffuse0, surface_sampler}
#import dh2_surface::{filling_colour,filling_plane_alpha}
#import bevy_pbr::mesh_view_bindings::view

// arkDefault/arkVegetation Z branches return after ClipZorShadowPass:
// only diffuse0 alpha and DefaultAlphaTest, before layers, vertex color,
// gloss-in-alpha, normals, AO or lighting (arkDefaultHelpers.decl:29-40).
@fragment
fn fragment(vertex_output: VertexOutput) {
    if compiled_misc.z==7u || compiled_misc.z==8u {discard;}
    if compiled_misc.z==6u {
        var uv=vec2(0.0);var uv1=vec2(0.0);
#ifdef VERTEX_UVS_A
        uv=vertex_output.uv;
#endif
#ifdef VERTEX_UVS_B
        uv1=vertex_output.uv_b;
#endif
        let color=filling_colour(uv,uv1,vec4(1.0));
        let clip_w=(view.clip_from_world*vertex_output.world_position).w;
        let alpha=filling_plane_alpha(color.a,vertex_output.world_position.xyz,clip_w);
#ifdef SHADOW_PASS
        if alpha<0.7 {discard;}
#else
        if alpha<max(surface.filling[8].w,0.0001) {discard;}
#endif
        return;
    }
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
    if compiled_misc.z == 4u {
        if (compiled_misc.w & 1u) != 0u {
#ifdef VERTEX_UVS_A
            let alpha=textureSampleGrad(diffuse0,surface_sampler,vertex_output.uv,dpdx(vertex_output.uv),dpdy(vertex_output.uv)).a;
            if alpha<0.5 { discard; }
#endif
        }
        return;
    }
    let material = pbr_bindings::material;
    alpha_discard(material, material.base_color);
}
