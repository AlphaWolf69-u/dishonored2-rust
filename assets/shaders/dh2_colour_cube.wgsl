// PostFX/ToneMapping/{blendColorCubes,applyColorCubeOverrides}.
@group(0) @binding(0) var<uniform> cube_weights:vec4<f32>;
@group(0) @binding(1) var cube_source:texture_3d<f32>;
@group(0) @binding(2) var cube_target0:texture_3d<f32>;
@group(0) @binding(3) var cube_target1:texture_3d<f32>;
@group(0) @binding(4) var cube_target2:texture_3d<f32>;
@group(0) @binding(5) var cube_target3:texture_3d<f32>;
@group(0) @binding(6) var cube_output:texture_storage_3d<rgba8unorm,write>;
@group(0) @binding(7) var cube_point:sampler;
// The host backend uses 4x4 groups of 8x8 lanes instead of one 32x32 group,
// preserving the exact texel coordinates on devices with a 256-lane limit.
@compute @workgroup_size(8,8,1)
fn blend_cubes(@builtin(global_invocation_id) id:vec3<u32>) {
    let uv=(vec3<f32>(id)+vec3(0.5))*(1.0/32.0);
    let a=textureSampleLevel(cube_source,cube_point,uv,0.0);
    let b=textureSampleLevel(cube_target0,cube_point,uv,0.0);
    textureStore(cube_output,vec3<i32>(id),mix(a,b,cube_weights.x));
}
@compute @workgroup_size(8,8,1)
fn apply_cube_overrides(@builtin(global_invocation_id) id:vec3<u32>) {
    let uv=(vec3<f32>(id)+vec3(0.5))*(1.0/32.0);
    let a=textureSampleLevel(cube_source,cube_point,uv,0.0);
    var b=cube_weights.x*textureSampleLevel(cube_target0,cube_point,uv,0.0);
    if cube_weights.y>0.000001 {b+=cube_weights.y*textureSampleLevel(cube_target1,cube_point,uv,0.0);}
    if cube_weights.z>0.000001 {b+=cube_weights.z*textureSampleLevel(cube_target2,cube_point,uv,0.0);}
    if cube_weights.w>0.000001 {b+=cube_weights.w*textureSampleLevel(cube_target3,cube_point,uv,0.0);}
    textureStore(cube_output,vec3<i32>(id),mix(a,b,dot(cube_weights,vec4(1.0))));
}
