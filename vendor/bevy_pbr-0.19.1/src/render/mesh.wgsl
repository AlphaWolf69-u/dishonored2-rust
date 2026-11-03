#import bevy_pbr::{
    mesh_bindings::mesh,
    mesh_functions,
    skinning,
    morph::{morph_position, morph_normal, morph_tangent},
    forward_io::{Vertex, VertexOutput},
    view_transformations::position_world_to_clip,
}

#ifdef DH2_VERTEX_PROBE
#import dh2_indirect::{vertex_lighting, pack_vertex_irradiance}
#endif
#ifdef DH2_FOG_VERTEX
#import dh2_material_fog::material_fog_vertex
#endif
#ifdef DH2_VEGETATION_WIND
#import dh2_vegetation_wind::{vegetation_slot, vegetation_world_position}
#endif
#ifdef DH2_PARTICLE_INSTANCES
#import dh2_particle_surface::{particle_surface_draw, particle_surface_instance, particle_surface_world_from_local, particle_surface_colour}
#endif

#ifdef DH2_DECAL
@group(#{MATERIAL_BIND_GROUP}) @binding(155) var<uniform> decal_zbias: f32;
#endif

#ifdef MORPH_TARGETS
// The instance_index parameter must match vertex_in.instance_index. This is a work around for a wgpu dx12 bug.
// See https://github.com/gfx-rs/naga/issues/2416
fn morph_vertex(vertex_in: Vertex, instance_index: u32) -> Vertex {
    var vertex = vertex_in;
    let first_vertex = mesh[instance_index].first_vertex_index;
    let vertex_index = vertex.index - first_vertex;

    let weight_count = bevy_pbr::morph::layer_count(instance_index);
    for (var i: u32 = 0u; i < weight_count; i ++) {
        let weight = bevy_pbr::morph::weight_at(i, instance_index);
        if weight == 0.0 {
            continue;
        }
        vertex.position += weight * morph_position(vertex_index, i, instance_index);
#ifdef VERTEX_NORMALS
        vertex.normal += weight * morph_normal(vertex_index, i, instance_index);
#endif
#ifdef VERTEX_TANGENTS
        vertex.tangent += vec4(weight * morph_tangent(vertex_index, i, instance_index), 0.0);
#endif
    }
    return vertex;
}
#endif

@vertex
fn vertex(vertex_no_morph: Vertex) -> VertexOutput {
    var out: VertexOutput;
#ifdef DH2_PARTICLE_INSTANCES
    // Direct instancing: instance_index counts this draw's instances; the
    // entity's own mesh uniform comes from the draw's immediates. Ordinary
    // draws keep reading vertex_no_morph.instance_index directly (naga 2416).
    let particle_mesh_index = particle_surface_draw.mesh_index;
    let particle = particle_surface_instance(vertex_no_morph.instance_index);
    out.particle_tag = particle.parameters.w;
#endif

#ifdef MORPH_TARGETS
    var vertex = morph_vertex(vertex_no_morph, vertex_no_morph.instance_index);
#else
    var vertex = vertex_no_morph;
#endif

#ifdef DH2_PARTICLE_INSTANCES
    let mesh_world_from_local = particle_surface_world_from_local(particle);
#else
    let mesh_world_from_local = mesh_functions::get_world_from_local(vertex_no_morph.instance_index);
#endif

#ifdef SKINNED
#ifdef DH2_SKIN8
    var world_from_local = skinning::skin_model8(
        vertex.joint_indices,
        vertex.dh2_joint_indices_b,
        vertex.joint_weights,
        vertex.dh2_joint_weights_b,
        vertex_no_morph.instance_index
    );
#else
    // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416 .
    var world_from_local = skinning::skin_model(
        vertex.joint_indices,
        vertex.joint_weights,
        vertex_no_morph.instance_index
    );
#endif
#else
    var world_from_local = mesh_world_from_local;
#endif

#ifdef VERTEX_NORMALS
#ifdef DH2_AUTHORED_BASIS
    // DH2 common vertex: model-linear N/T/B, independently normalized.
    out.world_normal = normalize((world_from_local * vec4(vertex.normal, 0.0)).xyz);
#ifdef DH2_SKIN8
    // The skin program's bitangent: cross of the skinned normal and tangent.
    out.world_bitangent = normalize(cross((world_from_local * vec4(vertex.normal, 0.0)).xyz,
        (world_from_local * vec4(vertex.tangent.xyz, 0.0)).xyz) * vertex.tangent.w);
#else
#ifdef DH2_DERIVED_BITANGENT
    // DH2: installed SNORM8 N/T; the CPU's bitangent was cross(N, T) * sign.
    out.world_bitangent = normalize((world_from_local * vec4(cross(vertex.normal, vertex.tangent.xyz) * vertex.tangent.w, 0.0)).xyz);
#else
    out.world_bitangent = normalize((world_from_local * vec4(vertex.bitangent, 0.0)).xyz);
#endif
#endif
#else
#ifdef SKINNED
    out.world_normal = skinning::skin_normals(world_from_local, vertex.normal);
#else
    out.world_normal = mesh_functions::mesh_normal_local_to_world(
        vertex.normal,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );
#endif
#endif
#endif

#ifdef VERTEX_POSITIONS
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
#ifdef DH2_VEGETATION_WIND
#ifdef DH2_PARTICLE_INSTANCES
    let vegetation = vegetation_slot(particle.parameters.w);
#else
    let vegetation = vegetation_slot(mesh[vertex_no_morph.instance_index].tag);
#endif
    if vegetation != 0xffffffffu {
        out.world_position = vec4(vegetation_world_position(vegetation, vertex.vegetation_position,
            vertex.vegetation_normal, vertex.vegetation_color, world_from_local), 1.0);
    }
#endif
    out.position = position_world_to_clip(out.world_position.xyz);
#ifdef DH2_MODEL_DEPTH_HACK
    // Installed vertex programs: clip.z = dot(pos, mvpZ) * cb_modeldepthhack;
    // X, Y and W keep the world view projection.
    out.position.z *= vertex.model_depth_hack;
#endif
#ifdef DH2_DECAL
    // arkDecal: offset homogeneous Z, preserving the authored world position.
    out.position.z -= decal_zbias;
#endif
#ifdef DH2_FAR_DEPTH
    out.position.z=0.0;
#endif
#endif

#ifdef VERTEX_UVS_A
    out.uv = vertex.uv;
#endif
#ifdef VERTEX_UVS_B
    out.uv_b = vertex.uv_b;
#endif

#ifdef VERTEX_TANGENTS
#ifdef DH2_AUTHORED_BASIS
    out.world_tangent = vec4(normalize((world_from_local * vec4(vertex.tangent.xyz, 0.0)).xyz), vertex.tangent.w);
#else
    out.world_tangent = mesh_functions::mesh_tangent_local_to_world(
        world_from_local,
        vertex.tangent,
        // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
        // See https://github.com/gfx-rs/naga/issues/2416
        vertex_no_morph.instance_index
    );
#endif
#endif

#ifdef VERTEX_COLORS
#ifdef DH2_PARTICLE_INSTANCES
    out.color = particle_surface_colour(particle, vertex.color);
#else
    out.color = vertex.color;
#endif
#endif

#ifdef VERTEX_OUTPUT_INSTANCE_INDEX
    // Use vertex_no_morph.instance_index instead of vertex.instance_index to work around a wgpu dx12 bug.
    // See https://github.com/gfx-rs/naga/issues/2416
#ifdef DH2_PARTICLE_INSTANCES
    out.instance_index = particle_mesh_index;
#else
    out.instance_index = vertex_no_morph.instance_index;
#endif
#endif

#ifdef VISIBILITY_RANGE_DITHER
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_no_morph.instance_index, mesh_world_from_local[3]);
#endif

#ifdef DH2_VERTEX_PROBE
#ifdef DH2_PARTICLE_INSTANCES
    let lighting = vertex_lighting(particle.parameters.w,
        vertex_no_morph.index - mesh[particle_mesh_index].first_vertex_index,
        out.world_position.xyz);
#else
    let lighting = vertex_lighting(mesh[vertex_no_morph.instance_index].tag,
        vertex_no_morph.index - mesh[vertex_no_morph.instance_index].first_vertex_index,
        out.world_position.xyz);
#endif
#ifdef DH2_AUTHORED_BASIS
    let bitangent = out.world_bitangent;
#else
    let bitangent = cross(out.world_normal, out.world_tangent.xyz) * out.world_tangent.w;
#endif
    let packed = pack_vertex_irradiance(lighting.sh, out.world_tangent.xyz, bitangent,
        out.world_normal, (#{DH2_ALPHA_FLAGS}u & 4u) != 0u);
    out.probe_luminance = packed.luminance;
    out.probe_chroma_a = packed.chroma_a;
    out.probe_chroma_b = packed.chroma_b;
    // vertex/common: NORMAL.w carries the triangle corner, independently of
    // TANGENT.w's handedness. Perspective interpolation transports three IDs.
    var bary=vec3(1.0,0.0,0.0);
#ifdef DH2_NORMAL_W
    let corner=vertex_no_morph.normal_w;
    bary=vec3(clamp(-corner,0.0,1.0),select(0.0,1.0,abs(corner)<=0.5),clamp(corner,0.0,1.0));
#endif
    out.reflection_indices_gain=vec4(bary*f32(lighting.cube_index),lighting.reflection_gain);
    out.reflection_barycentrics=bary.xy;
#endif
#ifdef DH2_FOG_VERTEX
#ifdef DH2_PARTICLE_INSTANCES
    out.fog_color=material_fog_vertex(out.position,out.world_position.xyz,(#{DH2_ALPHA_FLAGS}u&32u)!=0u,particle.parameters.w);
#else
    out.fog_color=material_fog_vertex(out.position,out.world_position.xyz,(#{DH2_ALPHA_FLAGS}u&32u)!=0u,mesh[vertex.instance_index].tag);
#endif
#endif
    return out;
}

@fragment
fn fragment(
    mesh: VertexOutput,
) -> @location(0) vec4<f32> {
#ifdef VERTEX_COLORS
    return mesh.color;
#else
    return vec4<f32>(1.0, 0.0, 1.0, 1.0);
#endif
}
