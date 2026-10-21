// FFTWaves/UpdateFoam/BasedOnHeight and BasedOnJacobian. The host must enable
// the storage-texture read/write capability and supply the persistent R32F mask.
struct FoamUniform {
    // FoamThreshold, FoamSpawnRate, FoamDecayRate, localTime.y.
    controls: vec4<f32>,
}
@group(0) @binding(0) var<uniform> parameters: FoamUniform;
@group(0) @binding(1) var source: texture_2d<f32>;
@group(0) @binding(2) var foam_mask: texture_storage_2d<r32float, read_write>;

@compute @workgroup_size(16, 16, 1)
fn foam_from_height(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    var foam = textureLoad(foam_mask, pixel).x;
    if textureLoad(source, pixel, 0).z > parameters.controls.x {
        foam += parameters.controls.y * parameters.controls.w;
        foam = min(foam, 1.0);
    }
    foam -= foam * parameters.controls.z * parameters.controls.w;
    foam = max(foam, 0.0);
    textureStore(foam_mask, pixel, vec4<f32>(foam, 0.0, 0.0, 0.0));
}

@compute @workgroup_size(16, 16, 1)
fn foam_from_jacobian(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    var foam = textureLoad(foam_mask, pixel).x;
    let fold = smoothstep(0.7, 1.3, textureLoad(source, pixel, 0).w);
    foam += fold * parameters.controls.y * parameters.controls.w;
    foam = min(foam, 1.0);
    foam -= foam * parameters.controls.z * parameters.controls.w;
    foam = max(foam, 0.0);
    textureStore(foam_mask, pixel, vec4<f32>(foam, 0.0, 0.0, 0.0));
}
