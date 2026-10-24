#define_import_path dh2_vegetation_wind
// Forward/Z arkvegetation vertex placement (installed VS 674e25d0 family):
// AnimateVegetation on the original LOCAL position, then the original model
// rows. Multi-instanced needsWind instances project their windInfo.wind onto
// the normalized model rows; every other instance uses the HLSL's fixed
// localWind (0, 0.01, 0, 0). Normals and tangents are not bent.
#import dh2_animate_vegetation::{VegetationAnimation, animate_vegetation}

// One record per vegetation render instance, rebuilt every frame.
struct VegetationInstance {
    // Game-space model rows: the world matrix record or baked placement rows.
    x: vec4<f32>,
    y: vec4<f32>,
    z: vec4<f32>,
    // vegetAnim/MainBendingVar, MainBendingFreq, MainBendingAmplitude,
    // LeafMinWindStrength (.x of each material vector).
    bending: vec4<f32>,
    // vegetAnim/LeafFreq, LeafAmplitude, BranchAmplitude, then $localTime.x.
    leaf: vec4<f32>,
    // windInfo.wind from 77fbc0: unit direction and magnitude.
    wind: vec4<f32>,
    // x: nonzero when this instance reads the wind buffer. y: nonzero when an
    // unbaked placed model follows its current mesh transform instead of rows.
    flags: vec4<u32>,
}

// [0] = world matrix count; then one slot per world matrix (tag bit31 set)
// and one per probe record (tag bit31 clear). 0xffffffff = no record.
@group(#{MATERIAL_BIND_GROUP}) @binding(168)
var<storage, read> vegetation_slots: array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(169)
var<storage, read> vegetation_instances: array<VegetationInstance>;

// Instances created after the table was built have no record.
fn vegetation_slot(tag: u32) -> u32 {
    var index = 1u + vegetation_slots[0] + tag;
    if (tag & 0x80000000u) != 0u {
        index = 1u + (tag & 0x7fffffffu);
    }
    if index >= arrayLength(&vegetation_slots) {
        return 0xffffffffu;
    }
    let slot = vegetation_slots[index];
    if slot >= arrayLength(&vegetation_instances) {
        return 0xffffffffu;
    }
    return slot;
}

// Bent position in Bevy world (view-axis) space.
fn vegetation_world_position(slot: u32, position: vec3<f32>, normal_xy: vec2<f32>,
    color: vec3<f32>, world_from_local: mat4x4<f32>) -> vec3<f32>
{
    let v = vegetation_instances[slot];
    var local_wind = vec2<f32>(0.0, 0.01);
    var strength = 0.0;
    if v.flags.x != 0u {
        let row_x = inverseSqrt(dot(v.x.xyz, v.x.xyz)) * v.x.xyz;
        let row_y = inverseSqrt(dot(v.y.xyz, v.y.xyz)) * v.y.xyz;
        local_wind = vec2<f32>(dot(row_x, v.wind.xyz), dot(row_y, v.wind.xyz)) * v.wind.w;
        strength = v.wind.w;
    }
    let animation = VegetationAnimation(vec4<f32>(v.bending.x), vec4<f32>(v.bending.y),
        vec4<f32>(v.bending.z), vec4<f32>(v.bending.w), vec4<f32>(v.leaf.x),
        vec4<f32>(v.leaf.y), vec4<f32>(v.leaf.z));
    let bent = vec4<f32>(animate_vegetation(animation, v.leaf.w, local_wind,
        max(strength, v.bending.w), position, normal_xy, color), 1.0);
    if v.flags.y != 0u {
        return (world_from_local * vec4<f32>(bent.x, bent.z, -bent.y, 1.0)).xyz;
    }
    let game = vec3<f32>(dot(bent, v.x), dot(bent, v.y), dot(bent, v.z));
    return vec3<f32>(game.x, game.z, -game.y);
}
