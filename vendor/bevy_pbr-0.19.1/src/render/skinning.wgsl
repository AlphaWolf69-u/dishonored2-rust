#define_import_path bevy_pbr::skinning

#import bevy_pbr::mesh_types::SkinnedMesh
#import bevy_pbr::mesh_bindings::mesh

#ifdef SKINNED

#ifdef SKINS_USE_UNIFORM_BUFFERS
@group(2) @binding(1) var<uniform> joint_matrices: SkinnedMesh;
#else   // SKINS_USE_UNIFORM_BUFFERS
@group(2) @binding(1) var<storage> joint_matrices: array<mat4x4<f32>>;
#endif  // SKINS_USE_UNIFORM_BUFFERS

// An array of matrices specifying the joint positions from the previous frame.
//
// This is used for motion vector computation.
//
// If this is the first frame, or we're otherwise prevented from using data from
// the previous frame, this is simply the same as `joint_matrices` above.
#ifdef SKINS_USE_UNIFORM_BUFFERS
@group(2) @binding(6) var<uniform> prev_joint_matrices: SkinnedMesh;
#else   // SKINS_USE_UNIFORM_BUFFERS
@group(2) @binding(6) var<storage> prev_joint_matrices: array<mat4x4<f32>>;
#endif  // SKINS_USE_UNIFORM_BUFFERS

fn skin_model(
    indexes: vec4<u32>,
    weights: vec4<f32>,
    instance_index: u32,
) -> mat4x4<f32> {
#ifdef SKINS_USE_UNIFORM_BUFFERS
    return weights.x * joint_matrices.data[indexes.x]
        + weights.y * joint_matrices.data[indexes.y]
        + weights.z * joint_matrices.data[indexes.z]
        + weights.w * joint_matrices.data[indexes.w];
#else   // SKINS_USE_UNIFORM_BUFFERS
    var skin_index = mesh[instance_index].current_skin_index;
    return weights.x * joint_matrices[skin_index + indexes.x]
        + weights.y * joint_matrices[skin_index + indexes.y]
        + weights.z * joint_matrices[skin_index + indexes.z]
        + weights.w * joint_matrices[skin_index + indexes.w];
#endif  // SKINS_USE_UNIFORM_BUFFERS
}

#ifdef DH2_SKIN8
fn dh2_skin_term(m: mat4x4<f32>, w: f32, acc: mat4x4<f32>) -> mat4x4<f32> {
    return mat4x4<f32>(fma(m[0], vec4(w), acc[0]), fma(m[1], vec4(w), acc[1]),
        fma(m[2], vec4(w), acc[2]), fma(m[3], vec4(w), acc[3]));
}

fn dh2_joint(skin_index: u32, joint: u32) -> mat4x4<f32> {
#ifdef SKINS_USE_UNIFORM_BUFFERS
    return joint_matrices.data[joint];
#else
    return joint_matrices[skin_index + joint];
#endif
}

// DH2: the installed skin vertex program's eight-weight blend
// (edge_skeleton::vertex_skin_matrix): joint 1 times weight 1, multiply-add
// joint 0, then joints 2..7 in order.
fn skin_model8(
    first: vec4<u32>,
    second: vec4<u32>,
    first_weights: vec4<f32>,
    second_weights: vec4<f32>,
    instance_index: u32,
) -> mat4x4<f32> {
    let skin_index = mesh[instance_index].current_skin_index;
    var m = dh2_joint(skin_index, first.y) * first_weights.y;
    m = dh2_skin_term(dh2_joint(skin_index, first.x), first_weights.x, m);
    m = dh2_skin_term(dh2_joint(skin_index, first.z), first_weights.z, m);
    m = dh2_skin_term(dh2_joint(skin_index, first.w), first_weights.w, m);
    m = dh2_skin_term(dh2_joint(skin_index, second.x), second_weights.x, m);
    m = dh2_skin_term(dh2_joint(skin_index, second.y), second_weights.y, m);
    m = dh2_skin_term(dh2_joint(skin_index, second.z), second_weights.z, m);
    return dh2_skin_term(dh2_joint(skin_index, second.w), second_weights.w, m);
}
#endif

// Returns the skinned position of a vertex with the given weights from the
// previous frame.
//
// This is used for motion vector computation.
fn skin_prev_model(
    indexes: vec4<u32>,
    weights: vec4<f32>,
    instance_index: u32,
) -> mat4x4<f32> {
#ifdef SKINS_USE_UNIFORM_BUFFERS
    return weights.x * prev_joint_matrices.data[indexes.x]
        + weights.y * prev_joint_matrices.data[indexes.y]
        + weights.z * prev_joint_matrices.data[indexes.z]
        + weights.w * prev_joint_matrices.data[indexes.w];
#else   // SKINS_USE_UNIFORM_BUFFERS
    let skin_index = mesh[instance_index].current_skin_index;
    return weights.x * prev_joint_matrices[skin_index + indexes.x]
        + weights.y * prev_joint_matrices[skin_index + indexes.y]
        + weights.z * prev_joint_matrices[skin_index + indexes.z]
        + weights.w * prev_joint_matrices[skin_index + indexes.w];
#endif  // SKINS_USE_UNIFORM_BUFFERS
}

fn inverse_transpose_3x3m(in: mat3x3<f32>) -> mat3x3<f32> {
    let x = cross(in[1], in[2]);
    let y = cross(in[2], in[0]);
    let z = cross(in[0], in[1]);
    let det = dot(in[2], z);
    return mat3x3<f32>(
        x / det,
        y / det,
        z / det
    );
}

fn skin_normals(
    world_from_local: mat4x4<f32>,
    normal: vec3<f32>,
) -> vec3<f32> {
    return normalize(
        inverse_transpose_3x3m(
            mat3x3<f32>(
                world_from_local[0].xyz,
                world_from_local[1].xyz,
                world_from_local[2].xyz
            )
        ) * normal
    );
}

#endif
