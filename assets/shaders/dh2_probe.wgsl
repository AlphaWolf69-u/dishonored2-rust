#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting_with_native_shadows, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    decal::clustered::apply_decals,
}

#import dh2_surface::{evaluate_surface, unpack_bc5_normal_map, surface, compiled_misc, compiled_native_shadow}
#import dh2_surface::{diffuse0, bump0, gloss0, lightmap, emissive_map, surface_sampler, surface_anisotropic_sampler}
#import dh2_vista::vista_lighting
#import dh2_indirect::{probe_sh, PackedIrradiance, unpack_diffuse, probe_diffuse}
#import dh2_parallel_receiver::{native_parallel_directional_index, native_parallel_depth_visibility}
#import dh2_surface::{filling_plane_alpha, filling_normal, filling_fresnel, linear_clamp, scaled_uv}
#import dh2_sky::sky_colour
#import dh2_surface::material_exposure

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(vertex_output, is_front);
    if compiled_misc.z==9u {
        var uv=vec2(0.0);var color=vec4(1.0);
#ifdef VERTEX_UVS_A
        uv=vertex_output.uv;
#endif
#ifdef VERTEX_COLORS
        color=vertex_output.color;
#endif
        var out:FragmentOutput;
        out.color=main_pass_post_lighting_processing(pbr_input,
            textureSampleGrad(diffuse0,surface_anisotropic_sampler,uv,dpdx(uv),dpdy(uv))*color);
        return out;
    }
    if compiled_misc.z==7u || compiled_misc.z==8u {
        var uv=vec2(0.0);var color=vec4(1.0);
#ifdef VERTEX_UVS_A
        uv=vertex_output.uv;
#endif
#ifdef VERTEX_COLORS
        color=vertex_output.color;
#endif
        let sky=sky_colour(uv,vertex_output.world_position.xyz,vertex_output.world_normal,color,compiled_misc.z==8u);
        var out:FragmentOutput;
        out.color=main_pass_post_lighting_processing(pbr_input,vec4(sky.rgb*139.26*view.exposure,sky.a));
        return out;
    }
    if compiled_misc.z == 4u {
        var uv0=vec2<f32>(0.0);
        var uv1=vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
        uv0=vertex_output.uv;
#endif
#ifdef VERTEX_UVS_B
        uv1=vertex_output.uv_b;
#endif
        let diffuse=textureSampleGrad(diffuse0,surface_anisotropic_sampler,uv0,dpdx(uv0),dpdy(uv0));
        if diffuse.a < 0.5 { discard; }
        let face=select(1.0,-1.0,!is_front && (compiled_misc.w & 4u)!=0u);
        let geometric_normal=normalize(vertex_output.world_normal*face);
        var N=geometric_normal;
        var T=vec3<f32>(0.0);
        var B=vec3<f32>(0.0);
#ifdef VERTEX_TANGENTS
        T=vertex_output.world_tangent.xyz;
        B=cross(N,T)*vertex_output.world_tangent.w;
#endif
        if compiled_misc.x!=0u {
            let xy=textureSampleGrad(bump0,surface_sampler,uv0,dpdx(uv0),dpdy(uv0)).xy;
            let normal=unpack_bc5_normal_map(xy);
            N=normalize(normal.x*T+normal.y*B+normal.z*N);
            T=normalize(cross(B,N));
            B=cross(N,T);
        }
        var gloss=surface.ranges.y;
        if #{DH2_HAS_GLOSS}u!=0u {
            gloss=mix(surface.ranges.x,surface.ranges.y,
                textureSampleGrad(gloss0,surface_sampler,uv0,dpdx(uv0),dpdy(uv0)).r);
        }
        let baked=textureSampleGrad(lightmap,surface_sampler,uv1,dpdx(uv1),dpdy(uv1));
        var emission=vec3<f32>(0.0);
        if surface.emissive_flags.x!=0u {
            emission=surface.vista[3].w*textureSampleGrad(emissive_map,surface_anisotropic_sampler,
                uv0,dpdx(uv0),dpdy(uv0)).rgb;
        }
        let sun=surface.vista_sun[0].rgb;
        let L=surface.vista_sun[1].xyz;
        let V=normalize(view.world_position-vertex_output.world_position.xyz);
        let radiance=vista_lighting(diffuse.rgb,baked,gloss,surface.specular_dielectric.rgb,
            surface.diffuse_metal.rgb,surface.ranges.w,surface.vista,V,L,sun,N,T,B,geometric_normal,emission);
        var out:FragmentOutput;
        out.color=main_pass_post_lighting_processing(pbr_input,vec4(radiance*139.26*view.exposure,1.0));
        return out;
    }
    var diffuse_response=pbr_input.material.base_color.rgb;
    var native_indirect=vec3<f32>(0.0);
    var native_emissive=vec3<f32>(0.0);
    // arkDefault and arkVegetation are the only programs evaluated by dh2_surface.
    // Original arkWater requires a separate depth/HDR-dependent shader; do not
    // feed it through the default layered-material equations.
    if compiled_misc.z == 1u || compiled_misc.z == 2u || compiled_misc.z == 6u {
        var uv = vec2<f32>(0.0);
#ifdef VERTEX_UVS_A
        uv = vertex_output.uv;
#endif
        var uv1 = vec2<f32>(0.0);
#ifdef VERTEX_UVS_B
        uv1 = vertex_output.uv_b;
#endif
        var color = vec4<f32>(1.0);
#ifdef VERTEX_COLORS
        color = vertex_output.color;
#endif
        let authored = evaluate_surface(uv, uv1, color);
        pbr_input.material.base_color = authored.albedo;
        native_emissive=authored.emissive;
        diffuse_response=authored.diffuse_response;
        pbr_input.material.perceptual_roughness = authored.roughness;
        pbr_input.material.metallic = authored.metallic;
        pbr_input.diffuse_occlusion = vec3<f32>(authored.occlusion);
        // Native twoSidedLighting reverses the whole geometric TBN, not just
        // the normal-map Z lane (BRDF_vegetation's original-basis recovery).
        let face_factor = select(1.0, -1.0, !is_front && (compiled_misc.w & 4u) != 0u);
        pbr_input.world_normal = face_factor * vertex_output.world_normal;
        pbr_input.N = normalize(pbr_input.world_normal);
#ifdef VERTEX_TANGENTS
        if compiled_misc.x != 0u {
            let normal = unpack_bc5_normal_map(authored.normal_xy);
            let n = vertex_output.world_normal;
            let t = vertex_output.world_tangent;
#ifdef DH2_AUTHORED_BASIS
            let b = vertex_output.world_bitangent;
#else
            let b = cross(n, t.xyz) * t.w;
#endif
            pbr_input.N = face_factor * normalize(normal.x * t.xyz + normal.y * b + normal.z * n);
        }
#endif
        if compiled_misc.z==6u {
            let clip_w=(view.clip_from_world*vertex_output.world_position).w;
            var alpha=filling_plane_alpha(authored.albedo.a,vertex_output.world_position.xyz,clip_w);
            if (surface.filling_flags.x & 2u)!=0u {
                var N=pbr_input.N;
#ifdef VERTEX_TANGENTS
                if (surface.filling_flags.x & 4u)!=0u {
                    let coords=scaled_uv(select(uv,uv1,surface.uv_sets[0].x!=0u),surface.uv[0]);
                    let normal=unpack_bc5_normal_map(textureSampleGrad(filling_normal,surface_sampler,coords,dpdx(coords),dpdy(coords)).xy);
                    let T=vertex_output.world_tangent;
#ifdef DH2_AUTHORED_BASIS
                    let B=vertex_output.world_bitangent;
#else
                    let B=cross(vertex_output.world_normal,T.xyz)*T.w;
#endif
                    N=normalize(normal.x*T.xyz+normal.y*B+normal.z*vertex_output.world_normal);
                }
#endif
                let cosine=clamp(dot(normalize(view.world_position-vertex_output.world_position.xyz),N),0.0,1.0);
                var fresnel=min(cosine/surface.filling[8].y,1.0);
                if (surface.filling_flags.x & 8u)!=0u {
                    fresnel=textureSampleGrad(filling_fresnel,linear_clamp,vec2(cosine),vec2(dpdx(cosine)),vec2(dpdy(cosine))).r;
                }
                alpha*=pow(fresnel,surface.filling[8].z);
            }
            if surface.filling_flags.y>0u && alpha<max(surface.filling[8].w,0.0001) {discard;}
            pbr_input.material.base_color.a=alpha;
        }
    } else {
        // Native paths already clip raw diffuse alpha before tints/layers.
        pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    }
    apply_decals(&pbr_input);

    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        let probe_index = mesh[vertex_output.instance_index].tag;
#ifdef DH2_VERTEX_PROBE
#ifdef DH2_AUTHORED_BASIS
        let bitangent = vertex_output.world_bitangent;
#else
        let bitangent = cross(vertex_output.world_normal, vertex_output.world_tangent.xyz) * vertex_output.world_tangent.w;
#endif
        let indirect = unpack_diffuse(PackedIrradiance(vertex_output.probe_luminance,
            vertex_output.probe_chroma_a, vertex_output.probe_chroma_b),
            pbr_input.N, vertex_output.world_tangent.xyz, bitangent,
            vertex_output.world_normal, pbr_input.diffuse_occlusion.x, (compiled_misc.w & 4u) != 0u);
        native_indirect = indirect * diffuse_response;
#else
        native_indirect = probe_diffuse(pbr_input.N, probe_index)
            * diffuse_response * pbr_input.diffuse_occlusion;
#endif
    }

    var out: FragmentOutput;
#ifdef DH2_TRACE_LIGHT_COMPONENT
#ifdef DH2_VERTEX_PROBE
    if #{DH2_TRACE_LIGHT_COMPONENT} == 6u {
        out.color = vec4(vec3(vertex_output.probe_luminance), 1.0);
        return out;
    }
#endif
    if #{DH2_TRACE_LIGHT_COMPONENT} == 7u {
        out.color = vec4(pbr_input.diffuse_occlusion, 1.0);
        return out;
    }
    if #{DH2_TRACE_LIGHT_COMPONENT} == 8u {
        out.color = vec4(native_indirect, 1.0);
        return out;
    }
#endif
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        // Original lighting uses wsVertexNormal for the sun's shadow offset,
        // not the normal-map BRDF normal. The same light.m_shadow attenuates
        // both direct lobes; probes/emission/local lights remain untouched.
        let visibility = native_parallel_depth_visibility(pbr_input.world_position.xyz, vertex_output.world_normal,
            vertex_output.position.xy, compiled_native_shadow);
        out.color = apply_pbr_lighting_with_native_shadows(pbr_input,
            native_parallel_directional_index(), visibility, visibility, compiled_native_shadow, native_indirect);
    } else {
        out.color = pbr_input.material.base_color;
        if compiled_misc.z==6u {
            var compensation=1.0;
            if surface.filling_flags.y==0u {compensation=mix(1.0,1.0/material_exposure[0],surface.filling[10].y);}
            out.color=vec4(out.color.rgb*139.26*view.exposure*compensation,out.color.a);
        }
    }
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color=vec4(out.color.rgb+native_emissive*139.26*view.exposure,out.color.a);
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
