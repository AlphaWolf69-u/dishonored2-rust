#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting_with_native_shadows, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    decal::clustered::apply_decals,
}

#import dh2_surface::{evaluate_surface, unpack_bc5_normal_map, surface, compiled_misc, compiled_native_shadow}
#import dh2_indirect::{probe_sh, PackedIrradiance, unpack_diffuse, probe_diffuse}
#import dh2_parallel_receiver::{native_parallel_directional_index, native_parallel_depth_visibility}

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(vertex_output, is_front);
    var diffuse_response=pbr_input.material.base_color.rgb;
    var native_indirect=vec3<f32>(0.0);
    // arkDefault and arkVegetation are the only programs evaluated by dh2_surface.
    // Original arkWater requires a separate depth/HDR-dependent shader; do not
    // feed it through the default layered-material equations.
    if compiled_misc.z == 1u || compiled_misc.z == 2u {
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
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
