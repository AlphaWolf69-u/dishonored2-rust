// Decoded scene positions and current rigid instance matrices. The installed
// arkWater Z prepass and arkBoatVolume use ordinary TransformVertex; neither
// has fragment clipping or wave vertex displacement in these branches.
@group(0) @binding(0) var<uniform> clip_from_world:mat4x4<f32>;
@group(1) @binding(0) var<storage,read> world_from_local:array<mat4x4<f32>>;
@vertex fn position_depth(@location(0) position:vec3<f32>,
    @builtin(instance_index) instance:u32)->@builtin(position) vec4<f32> {
    return clip_from_world*world_from_local[instance]*vec4(position,1.0);
}
