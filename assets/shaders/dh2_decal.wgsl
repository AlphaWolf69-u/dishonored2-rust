#import bevy_pbr::mesh_view_bindings::view
#import dh2_ward::ward_surface

#import bevy_pbr::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_bindings::mesh,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting_with_shadows, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
}
#import dh2_indirect::{PackedIrradiance,unpack_diffuse,probe_diffuse}
#import dh2_surface::{evaluate_surface, unpack_bc5_normal_map, surface, compiled_misc, compiled_shadow}
#import dh2_parallel_receiver::{parallel_directional_index, parallel_depth_visibility, ambient_occlusion}
#import dh2_material_fog::material_fog_pixel
#ifdef DH2_DECAL_INSTANCES
#import dh2_decal_projection::{decal_record, decal_position, decal_inside, decal_min_diff, decal_depth_coordinates, decal_set_gradients, decal_effect_colour}
#import dh2_indirect::{material_lighting_values,evaluate_mapped_diffuse_response,pack_vertex_irradiance}
#ifdef DH2_DECAL_DEPTH_BOX
#import bevy_pbr::{prepass_utils::prepass_depth,view_transformations::{position_ndc_to_world,frag_coord_to_ndc}}
fn decal_receiver_position(pixel: vec2<f32>) -> vec3<f32> {
    let sample_pixel=clamp(pixel,view.viewport.xy+vec2(0.5),view.viewport.xy+view.viewport.zw-vec2(0.5));
    let depth=prepass_depth(vec4(sample_pixel,0.0,1.0),0u);
    return position_ndc_to_world(frag_coord_to_ndc(vec4(pixel,depth,1.0)));
}
#endif
#endif


@fragment
fn fragment(input: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var vertex_output=input;
#ifdef DH2_DECAL_INSTANCES
    let projection=decal_record(input.decal_index);
    var geometry_alpha=1.0;
    var projected_uv=vec2(0.0);
#ifdef DH2_DECAL_DEPTH_BOX
    let receiver=decal_receiver_position(input.position.xy);
    let local=decal_position(projection,receiver);
    if !decal_inside(local) {discard;}
    let right=decal_receiver_position(input.position.xy+vec2(1.0,0.0));
    let left=decal_receiver_position(input.position.xy-vec2(1.0,0.0));
    let up=decal_receiver_position(input.position.xy-vec2(0.0,1.0));
    let down=decal_receiver_position(input.position.xy+vec2(0.0,1.0));
    let du=decal_min_diff(receiver,right,left);
    let dv=decal_min_diff(receiver,up,down);
    vertex_output.world_position=vec4(receiver,1.0);
    vertex_output.world_normal=cross(normalize(du),normalize(dv));
    decal_set_gradients(dpdxCoarse(du),dpdyCoarse(dv));
    let coordinates=decal_depth_coordinates(projection,local,vertex_output.world_normal);
    projected_uv=coordinates.uv;
    geometry_alpha=coordinates.alpha;
#else
#ifdef DH2_DECAL_PROJECTS_UV
    let local=decal_position(projection,vertex_output.world_position.xyz);
    if !decal_inside(local) || dot(projection.projection.world_to_decal[2].xyz,vertex_output.world_normal)<0.0001 {discard;}
    projected_uv=local.xy*0.5+0.5;
#endif
#endif
#endif
    var pbr_input = pbr_input_from_standard_material(vertex_output, is_front);
    var diffuse_response=pbr_input.material.base_color.rgb;
    var indirect=vec3<f32>(0.0);
    var emissive=vec3<f32>(0.0);
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
#ifdef DH2_DECAL_INSTANCES
#ifdef DH2_DECAL_PROJECTS_UV
        uv=projected_uv; uv1=projected_uv;
#endif
#endif
        let authored = evaluate_surface(uv, uv1, color);
        var direct_diffuse=authored.direct_diffuse;
        var direct_ior=authored.direct_ior;
        var fresnel_strength=authored.fresnel_strength;
        pbr_input.material.base_color = authored.albedo;
#ifdef DH2_DECAL_INSTANCES
        pbr_input.material.base_color=decal_effect_colour(projection,uv,authored.albedo)*geometry_alpha;
        let projected_lighting=material_lighting_values(pbr_input.material.base_color.rgb,authored.roughness,authored.metallic,
            authored.specular_sample,#{DH2_HAS_SPECULAR}u==0u,surface.specular_dielectric.rgb,surface.diffuse_metal.rgb);
        direct_diffuse=projected_lighting.diffuse; direct_ior=projected_lighting.ior; fresnel_strength=projected_lighting.fresnel_strength;
#endif
        emissive=authored.emissive;
        diffuse_response=authored.diffuse_response;
#ifdef DH2_DECAL_INSTANCES
        diffuse_response=evaluate_mapped_diffuse_response(projected_lighting,authored.roughness);
#endif
        pbr_input.material.perceptual_roughness = authored.roughness;
        pbr_input.material.metallic = authored.metallic;
        pbr_input.diffuse_occlusion = vec3<f32>(ambient_occlusion(
            vertex_output.position.xy, authored.occlusion, (compiled_misc.w & 64u) != 0u));
        let face_factor = select(1.0, -1.0, !is_front && (compiled_misc.w & 4u) != 0u);
        pbr_input.world_normal = face_factor * vertex_output.world_normal;
        pbr_input.N = normalize(pbr_input.world_normal);
#ifdef VERTEX_TANGENTS
        if compiled_misc.x != 0u {
            let normal = unpack_bc5_normal_map(authored.normal_xy);
            let n = vertex_output.world_normal;
#ifdef DH2_DECAL_INSTANCES
#ifdef DH2_DECAL_PROJECTS_UV
            let t=vec4(normalize(cross(projection.projection.world_to_decal[1].xyz,n)),1.0);
#ifdef DH2_DECAL_DEPTH_BOX
            let b=cross(t.xyz,n);
#else
            let b=cross(n,t.xyz);
#endif
#else
            let t=vertex_output.world_tangent;
            let b=cross(n,t.xyz)*t.w;
#endif
#else
            let t = vertex_output.world_tangent;
#ifdef DH2_AUTHORED_BASIS
            let b = vertex_output.world_bitangent;
#else
            let b = cross(n, t.xyz) * t.w;
#endif
#endif // DH2_DECAL_INSTANCES
            pbr_input.N = face_factor * normalize(normal.x * t.xyz + normal.y * b + normal.z * n);
        }
#endif
        pbr_input.direct_surface=ward_surface(pbr_input.N,pbr_input.V,authored.roughness,
            direct_diffuse,direct_ior,surface.direct.x,surface.direct.y,
            fresnel_strength);
    } else {
        // Native paths already clip raw diffuse alpha before tints/layers.
        pbr_input.material.base_color = alpha_discard(pbr_input.material, pbr_input.material.base_color);
    }
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
#ifdef DH2_DECAL_INSTANCES
        let probe_index=projection.receiver.indices.x;
#ifdef DH2_DECAL_DEPTH_BOX
        var sh:array<vec3<f32>,9>;
        for(var i=0u;i<9u;i++){sh[i]=projection.projection.lighting[i].rgb;}
        let n=vertex_output.world_normal;
        let t=normalize(cross(projection.projection.world_to_decal[1].xyz,n));
        let b=cross(t,n);
        let packed=pack_vertex_irradiance(sh,t,b,n,(compiled_misc.w&4u)!=0u);
        indirect=unpack_diffuse(packed,pbr_input.N,t,b,n,pbr_input.diffuse_occlusion.x,(compiled_misc.w&4u)!=0u)*diffuse_response;
#endif
#else
        let probe_index = mesh[vertex_output.instance_index].tag;
#endif
#ifndef DH2_DECAL_DEPTH_BOX
#ifdef DH2_VERTEX_PROBE
#ifdef DH2_AUTHORED_BASIS
        let bitangent = vertex_output.world_bitangent;
#else
        let bitangent = cross(vertex_output.world_normal, vertex_output.world_tangent.xyz) * vertex_output.world_tangent.w;
#endif
        let probe_irradiance = unpack_diffuse(PackedIrradiance(vertex_output.probe_luminance,
            vertex_output.probe_chroma_a, vertex_output.probe_chroma_b),
            pbr_input.N, vertex_output.world_tangent.xyz, bitangent,
            vertex_output.world_normal, pbr_input.diffuse_occlusion.x, (compiled_misc.w & 4u) != 0u);
        indirect = probe_irradiance * diffuse_response;
#else
        indirect = probe_diffuse(pbr_input.N, probe_index)
            * diffuse_response * pbr_input.diffuse_occlusion;
#endif
#endif // DH2_DECAL_DEPTH_BOX
    }

    var out: FragmentOutput;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        // arkDecal's standard pass calls the same shadowed IntegrateLighting
        // as arkDefault. Its alpha/depth bias does not disable receiving.
        let visibility = parallel_depth_visibility(pbr_input.world_position.xyz,
            vertex_output.world_normal, vertex_output.position.xy, compiled_shadow);
        out.color = apply_pbr_lighting_with_shadows(pbr_input,
            parallel_directional_index(), visibility, visibility, compiled_shadow, indirect);
    } else { out.color = pbr_input.material.base_color; }
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        out.color=vec4(out.color.rgb+emissive*139.26*view.exposure,out.color.a);
    }
    if (compiled_misc.w&8u)!=0u {
#ifdef DH2_DECAL_INSTANCES
        let fog_tag=projection.receiver.indices.x;
#else
        let fog_tag=mesh[vertex_output.instance_index].tag;
#endif
        out.color=vec4(material_fog_pixel(out.color.rgb,vertex_output.position,vertex_output.world_position.xyz,fog_tag),out.color.a);
    }
    out.color = main_pass_post_lighting_processing(pbr_input, out.color);
    return out;
}
