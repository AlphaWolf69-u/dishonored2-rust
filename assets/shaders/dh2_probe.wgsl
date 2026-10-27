#import bevy_pbr::mesh_view_bindings::view
#import dh2_ward::ward_surface
#ifdef DH2_SKIN
#import dh2_direct::{SkinSurface,skin_surface,skin_direct_diffuse,ward_inverse_roughness_squared}
#import dh2_surface::{skin_fresnel_lookup,sss_mask,fuzz_mask,translucency_mask}
#import dh2_indirect::{MaterialLightingValues,evaluate_mapped_diffuse_response}
#import dh2_parallel_receiver::parallel_cloud_shadowing
#import bevy_pbr::view_transformations::{frag_coord_to_ndc,position_ndc_to_world}
#endif

#import bevy_pbr::{
    forward_io::VertexOutput,
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting_with_shadows, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    decal::clustered::apply_decals,
}

#import dh2_surface::{evaluate_surface, unpack_bc5_normal_map, surface, compiled_misc, compiled_shadow}
#import dh2_surface::{diffuse0, bump0, gloss0, lightmap, emissive_map, surface_sampler, surface_anisotropic_sampler}
#import dh2_vista::vista_lighting
#import dh2_indirect::{probe_sh, PackedIrradiance, unpack_diffuse, probe_diffuse}
#import dh2_parallel_receiver::{parallel_directional_index, parallel_depth_visibility, ambient_occlusion}
#import dh2_surface::{filling_plane_alpha, filling_normal, filling_fresnel, scaled_uv}
#import dh2_material_samplers::linear_clamp
#import dh2_sky::sky_colour
#import dh2_surface::material_exposure
#import dh2_reflection::reflected_light
#import dh2_material_fog::{material_fog_pixel,material_fog_composite}

// arkSSSRender writes hdrBuffer and sssBuffer; the third target carries the
// host's per-pixel arkSSSBlur inputs (albedo, kernel width) so the blur runs
// as one screen pass instead of redrawing the sssSurf list (bevy_backend).
#ifdef DH2_SSS_TARGETS
struct ProbeOutput {
    @location(0) color: vec4<f32>,
    @location(1) sss_diffuse: vec4<f32>,
    @location(2) sss_surface: vec4<f32>,
}
#else
struct ProbeOutput {
    @location(0) color: vec4<f32>,
}
#endif

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> ProbeOutput {
    var pbr_input = pbr_input_from_standard_material(vertex_output, is_front);
    if compiled_misc.z==9u {
        var uv=vec2(0.0);var color=vec4(1.0);
#ifdef VERTEX_UVS_A
        uv=vertex_output.uv;
#endif
#ifdef VERTEX_COLORS
        color=vertex_output.color;
#endif
        var out:ProbeOutput;
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
        var out:ProbeOutput;
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
        var out:ProbeOutput;
        out.color=main_pass_post_lighting_processing(pbr_input,vec4(radiance*139.26*view.exposure,1.0));
        return out;
    }
    var diffuse_response=pbr_input.material.base_color.rgb;
    var indirect=vec3<f32>(0.0);
    var emissive=vec3<f32>(0.0);
    var specular_reflectance=vec3<f32>(0.0);
    // arkDefault and arkVegetation are the only programs evaluated by dh2_surface.
    // Original arkWater requires a separate depth/HDR-dependent shader; do not
    // feed it through the default layered-material equations.
    var skin_albedo=vec3<f32>(1.0);
    var skin_ambient_normal=vec3<f32>(0.0);
    var skin_kernel=0.0;
    if compiled_misc.z == 1u || compiled_misc.z == 2u || compiled_misc.z == 6u || compiled_misc.z == 11u {
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
        emissive=authored.emissive;
        diffuse_response=authored.diffuse_response;
        specular_reflectance=authored.specular_reflectance;
        pbr_input.material.perceptual_roughness = authored.roughness;
        pbr_input.material.metallic = authored.metallic;
        pbr_input.diffuse_occlusion = vec3<f32>(ambient_occlusion(
            vertex_output.position.xy, authored.occlusion, (compiled_misc.w & 64u) != 0u));
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
        pbr_input.direct_surface=ward_surface(pbr_input.N,pbr_input.V,authored.roughness,
            authored.direct_diffuse,authored.direct_ior,surface.direct.x,surface.direct.y,
            authored.fresnel_strength);
#ifdef DH2_SKIN
        {
            // arkSSSRender: specularTint 0.028 -> IOR (Fresnel_IORFromF0), diffuse
            // albedo 1, HQ_FetchGloss2's three ranges of one gloss sample.
            var gloss=vec3(surface.ranges.y,surface.skin[0].y,surface.skin[0].w);
            if authored.gloss_sample>=0.0 {
                gloss=vec3(mix(surface.ranges.x,surface.ranges.y,authored.gloss_sample),
                    mix(surface.skin[0].x,surface.skin[0].y,authored.gloss_sample),
                    mix(surface.skin[0].z,surface.skin[0].w,authored.gloss_sample));
            }
            let roughness=vec3(1.0)-gloss;
            let root=sqrt(0.028);
            let ior=(1.0+root)/(1.00001-root);
            // Mask images sample the layer-0 coordinates (pixelCtx.m_texCoords).
            let base_uv=scaled_uv(select(uv,uv1,surface.uv_sets[0].x!=0u),surface.uv[0]);
            let base_dx=dpdx(base_uv);let base_dy=dpdy(base_uv);
            var fuzz=1.0;
            if (surface.skin_flags.w&4u)!=0u {
                fuzz=textureSampleGrad(fuzz_mask,surface_sampler,base_uv,base_dx,base_dy).r;
            }
            skin_surface=SkinSurface(pbr_input.N,roughness.x,pbr_input.V,surface.skin_flags.x,
                vec3(ward_inverse_roughness_squared(roughness.x),ward_inverse_roughness_squared(roughness.y),
                    ward_inverse_roughness_squared(roughness.z)),surface.skin_flags.y,
                surface.skin[1].xyz,surface.skin_flags.z,vec3(ior),surface.skin[2].x,
                surface.skin[2].y,fuzz);
            skin_direct_diffuse=vec3(0.0);
            pbr_input.material.perceptual_roughness=roughness.x;
            let ratio=(ior-1.0)/(ior+1.0);
            specular_reflectance=vec3(ratio*ratio);
            // BRDF_skin::ComputeSkinAmbientLighting: INVPI * 1 * Fdr(IOR, roughness).
            diffuse_response=evaluate_mapped_diffuse_response(MaterialLightingValues(vec3(1.0),
                vec3(ior),specular_reflectance,1.0),roughness.x);
            skin_albedo=authored.albedo.rgb;
            // ComputeSkinAmbientLighting: translucency_hq bends the SH normal
            // toward the view by surf.translucency (translucency/mask or 1).
            skin_ambient_normal=pbr_input.N;
            if (surface.skin_flags.w&8u)!=0u {
                var translucency=1.0;
                if (surface.skin_flags.w&16u)!=0u {
                    translucency=textureSampleGrad(translucency_mask,surface_sampler,base_uv,base_dx,base_dy).x;
                }
                skin_ambient_normal=mix(pbr_input.N,pbr_input.V,translucency);
            }
            // arkSSSBlur: SSS/Scale.x (times SSS/Mask) over the vertex's linear
            // view depth; the screen pass applies resolutionScale/tanFov.
            var width=surface.skin[2].z;
            if (surface.skin_flags.w&2u)!=0u {
                width*=textureSampleGrad(sss_mask,surface_sampler,base_uv,base_dx,base_dy).x;
            }
            let view_z=dot(vec4(view.view_from_world[0].z,view.view_from_world[1].z,
                view.view_from_world[2].z,view.view_from_world[3].z),vertex_output.world_position);
            skin_kernel=width/(-view_z);
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

    var skin_ambient_out=vec3<f32>(0.0);
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        let probe_index = mesh[vertex_output.instance_index].tag;
#ifdef DH2_SKIN
        let ambient_normal=skin_ambient_normal;
#else
        let ambient_normal=pbr_input.N;
#endif
#ifdef DH2_VERTEX_PROBE
#ifdef DH2_AUTHORED_BASIS
        let bitangent = vertex_output.world_bitangent;
#else
        let bitangent = cross(vertex_output.world_normal, vertex_output.world_tangent.xyz) * vertex_output.world_tangent.w;
#endif
        let probe_irradiance = unpack_diffuse(PackedIrradiance(vertex_output.probe_luminance,
            vertex_output.probe_chroma_a, vertex_output.probe_chroma_b),
            ambient_normal, vertex_output.world_tangent.xyz, bitangent,
            vertex_output.world_normal, pbr_input.diffuse_occlusion.x, (compiled_misc.w & 4u) != 0u);
        indirect = probe_irradiance * diffuse_response;
#else
        indirect = probe_diffuse(ambient_normal, probe_index)
            * diffuse_response * pbr_input.diffuse_occlusion;
#endif
#ifdef DH2_SKIN
        skin_ambient_out=indirect;
        indirect=vec3(0.0);
#endif
        if compiled_misc.z==1u || compiled_misc.z==2u || compiled_misc.z==6u || compiled_misc.z==11u {
            var reflection_indices_gain=vec4(0.0);
            var reflection_barycentrics=vec2(0.0);
#ifdef DH2_VERTEX_PROBE
            reflection_indices_gain=vertex_output.reflection_indices_gain;
            reflection_barycentrics=vertex_output.reflection_barycentrics;
#endif
            indirect+=pbr_input.diffuse_occlusion*reflected_light(probe_index,vertex_output.world_position.xyz,
                pbr_input.N,normalize(view.world_position-vertex_output.world_position.xyz),
                pbr_input.material.perceptual_roughness,specular_reflectance,
                compiled_misc.z==2u || compiled_misc.z==11u,
                reflection_indices_gain,reflection_barycentrics)
#ifdef DH2_SKIN
                // BRDF_skin::ComputeIndirect weights COMPUTE_IBL by tweak.w.
                *surface.skin[1].w
#endif
                ;
        }
    }

    var out: ProbeOutput;
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
        out.color = vec4(indirect, 1.0);
        return out;
    }
#endif
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        // Original lighting uses wsVertexNormal for the sun's shadow offset,
        // not the normal-map BRDF normal. The same light.m_shadow attenuates
        // both direct lobes; probes/emission/local lights remain untouched.
#ifdef DH2_SKIN
        // IntegrateLighting's parallel shadow and BRDF_skin's cloud factor read
        // wsPositionHacked: the pixel's (hacked) depth through the inverse
        // view projection, not the interpolated vertex position.
        let hacked=position_ndc_to_world(frag_coord_to_ndc(vertex_output.position));
        let visibility = parallel_depth_visibility(hacked, vertex_output.world_normal,
            vertex_output.position.xy, compiled_shadow)*parallel_cloud_shadowing(hacked);
#else
        let visibility = parallel_depth_visibility(pbr_input.world_position.xyz, vertex_output.world_normal,
            vertex_output.position.xy, compiled_shadow);
#endif
        out.color = apply_pbr_lighting_with_shadows(pbr_input,
            parallel_directional_index(), visibility, visibility, compiled_shadow, indirect);
    } else {
        out.color = pbr_input.material.base_color;
        if compiled_misc.z==6u {
            var compensation=1.0;
            if surface.filling_flags.y==0u {compensation=mix(1.0,1.0/material_exposure[0],surface.filling[10].y);}
            out.color=vec4(out.color.rgb*139.26*view.exposure*compensation,out.color.a);
        }
    }
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color=vec4(out.color.rgb+emissive*139.26*view.exposure,out.color.a);
    }
#ifdef DH2_FOG_VERTEX
    out.color=vec4(material_fog_composite(out.color.rgb,vertex_output.fog_color),out.color.a);
#else
    if (compiled_misc.w&8u)!=0u {
        out.color=vec4(material_fog_pixel(out.color.rgb,vertex_output.position,vertex_output.world_position.xyz,mesh[vertex_output.instance_index].tag),out.color.a);
    }
#endif
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
#ifdef DH2_SSS_TARGETS
    // sv_target1: (accumDiffuse + accumAmbient) * SSS/SkinFresnelLookup, in
    // the same units as the hdrBuffer output above.
    var sss=view.exposure*(skin_direct_diffuse+skin_ambient_out*139.26);
    let pseudo_fresnel=max(dot(pbr_input.V,pbr_input.N),0.0);
    let fresnel_dx=dpdx(pseudo_fresnel);let fresnel_dy=dpdy(pseudo_fresnel);
    if (surface.skin_flags.w&1u)!=0u {
        sss*=textureSampleGrad(skin_fresnel_lookup,surface_anisotropic_sampler,vec2(pseudo_fresnel,0.0),
            vec2(fresnel_dx,0.0),vec2(fresnel_dy,0.0)).rgb;
    }
    out.sss_diffuse=vec4(sss,1.0);
    out.sss_surface=vec4(skin_albedo,skin_kernel);
#endif
    return out;
}
