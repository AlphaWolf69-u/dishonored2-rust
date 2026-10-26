#define_import_path dh2_alpha_clip
#import bevy_pbr::{pbr_bindings,pbr_functions::alpha_discard}
#import dh2_surface::{surface, compiled_misc, scaled_uv, diffuse0, surface_sampler, surface_anisotropic_sampler}
#import dh2_surface::{filling_colour,filling_plane_alpha,decal_diffuse}
#import bevy_pbr::mesh_view_bindings::view

// arkDefault/arkVegetation Z branches return after ClipZorShadowPass:
// only diffuse0 alpha and DefaultAlphaTest, before layers, vertex color,
// gloss-in-alpha, normals, AO or lighting (arkDefaultHelpers.decl:29-40).
fn clip_surface(uv0:vec2<f32>,uv1:vec2<f32>,world_position:vec4<f32>,vertex_color:vec4<f32>) {
    if compiled_misc.z==7u || compiled_misc.z==8u {discard;}
    // arkSimpleSprite has no clip/discard in any specialization. Its colour
    // output is masked off by the nonrefractive alpha-depth draw state.
    if compiled_misc.z==9u {return;}
#ifndef SHADOW_PASS
    if (compiled_misc.w&2u)!=0u {
        // arkDecal's Z-alpha branch returns before HQ/gloss changes opacity.
        // Projected and FX variants are admitted only with their own producers.
        let colour=decal_diffuse(uv0,uv1,vertex_color);
        if colour.a<surface.alpha.x {discard;}
        return;
    }
#endif
    if compiled_misc.z==6u {
        let color=filling_colour(uv0,uv1,vec4(1.0));
        let clip_w=(view.clip_from_world*world_position).w;
        let alpha=filling_plane_alpha(color.a,world_position.xyz,clip_w);
#ifdef SHADOW_PASS
        if alpha<0.7 {discard;}
#else
        if alpha<max(surface.filling[8].w,0.0001) {discard;}
#endif
        return;
    }
    if compiled_misc.z == 1u || compiled_misc.z == 2u {
        if (compiled_misc.w & 1u) != 0u {
            let uv = scaled_uv(select(uv0, uv1, surface.uv_sets[0].x != 0u), surface.uv[0]);
#ifdef SHADOW_PASS
            let alpha = textureSampleGrad(diffuse0, surface_sampler, uv, dpdx(uv), dpdy(uv)).a;
            if alpha < 0.5 { discard; }
#else
            let alpha = textureSampleGrad(diffuse0, surface_anisotropic_sampler, uv, dpdx(uv), dpdy(uv)).a;
            if alpha < surface.alpha.x { discard; }
#endif
        }
        return;
    }
    if compiled_misc.z == 4u {
        if (compiled_misc.w & 1u) != 0u {
#ifdef VERTEX_UVS_A
            let alpha=textureSampleGrad(diffuse0,surface_anisotropic_sampler,uv0,dpdx(uv0),dpdy(uv0)).a;
            if alpha<0.5 { discard; }
#endif
        }
        return;
    }
    let material = pbr_bindings::material;
    alpha_discard(material, material.base_color);
}
