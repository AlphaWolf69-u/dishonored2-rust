#import bevy_pbr::mesh_view_bindings::view
#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting_with_native_shadows, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#import dh2_indirect::{PackedIrradiance,unpack_diffuse,probe_diffuse}
#import dh2_surface::{evaluate_surface, unpack_bc5_normal_map, surface, compiled_misc, compiled_native_shadow}
#import dh2_parallel_receiver::{native_parallel_directional_index, native_parallel_depth_visibility}

@fragment
fn fragment(vertex_output: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(vertex_output, is_front);
    var diffuse_response=pbr_input.material.base_color.rgb;
    var native_indirect=vec3<f32>(0.0);
    var native_emissive=vec3<f32>(0.0);
    if compiled_misc.z != 0u {
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
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        // arkDecal's standard pass calls the same shadowed IntegrateLighting
        // as arkDefault. Its alpha/depth bias does not disable receiving.
        let visibility = native_parallel_depth_visibility(pbr_input.world_position.xyz,
            vertex_output.world_normal, vertex_output.position.xy, compiled_native_shadow);
        out.color = apply_pbr_lighting_with_native_shadows(pbr_input,
            native_parallel_directional_index(), visibility, visibility, compiled_native_shadow, native_indirect);
    } else { out.color = pbr_input.material.base_color; }
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color=vec4(out.color.rgb+native_emissive*139.26*view.exposure,out.color.a);
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
