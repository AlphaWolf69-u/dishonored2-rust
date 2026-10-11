#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
    decal::clustered::apply_decals,
}

#import dh2_surface::{evaluate_surface, unpack_bc5_normal_map, surface, compiled_misc}
#import dh2_indirect::{probe_sh, PackedIrradiance, unpack_diffuse}

fn evaluate_sh_irradiance(direction: vec3<f32>, probe_index: u32) -> vec3<f32> {
    let x = direction.x;
    let y = direction.y;
    let z = direction.z;
    let c1 = 0.4290427654048917;
    let c2 = 0.5116633539732442;
    let c3 = 0.2477079561003757;
    let c4 = 0.886226925452758;

    return max(
        vec3<f32>(0.0),
        c1 * (x * x - y * y) * probe_sh[probe_index][8].rgb
            + c3 * (3.0 * z * z - 1.0) * probe_sh[probe_index][6].rgb
            + c4 * probe_sh[probe_index][0].rgb
            + 2.0 * c1 * (probe_sh[probe_index][4].rgb * x * y + probe_sh[probe_index][7].rgb * x * z + probe_sh[probe_index][5].rgb * y * z)
            + 2.0 * c2 * (probe_sh[probe_index][3].rgb * x + probe_sh[probe_index][1].rgb * y + probe_sh[probe_index][2].rgb * z),
    );
}

// Match the game's three principal SH sample directions and 0.5 scale for a
// front-facing, unoccluded surface. Bevy does not supply the game's bent-normal
// AO term, so the authored coefficients remain unoccluded here.
fn probe_diffuse(normal: vec3<f32>, probe_index: u32) -> vec3<f32> {
    // Probe coefficients are authored in Dishonored 2 world XYZ. The viewer's
    // camera basis maps game XYZ to view XZY with a sign flip on game Y.
    let n = normalize(vec3<f32>(normal.x, -normal.z, normal.y));
    let helper = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.z) > 0.999);
    let tangent = normalize(cross(helper, n));
    let bitangent = cross(n, tangent);
    let r0 = vec3<f32>(0.7071, 0.0, 0.7071);
    let r1 = vec3<f32>(-0.35355, -0.61237, 0.7071);
    let r2 = vec3<f32>(-0.35355, 0.61237, 0.7071);
    let d0 = r0.x * tangent + r0.y * bitangent + r0.z * n;
    let d1 = r1.x * tangent + r1.y * bitangent + r1.z * n;
    let d2 = r2.x * tangent + r2.y * bitangent + r2.z * n;
    return 0.3535533906 * (
        evaluate_sh_irradiance(d0, probe_index)
        + evaluate_sh_irradiance(d1, probe_index)
        + evaluate_sh_irradiance(d2, probe_index)
    );
}

@fragment
fn fragment(
    vertex_output: VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(vertex_output, is_front);
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
        pbr_input.material.emissive = vec4(pbr_input.material.emissive.rgb
            + indirect * pbr_input.material.base_color.rgb, pbr_input.material.emissive.a);
#else
        pbr_input.material.emissive = vec4<f32>(
            pbr_input.material.emissive.rgb
                + probe_diffuse(pbr_input.N, probe_index) * pbr_input.material.base_color.rgb * pbr_input.diffuse_occlusion,
            pbr_input.material.emissive.a,
        );
#endif
    }

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color = apply_pbr_lighting(pbr_input);
    } else {
        out.color = pbr_input.material.base_color;
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
