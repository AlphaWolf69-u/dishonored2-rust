// areaLights/replaceTexture: copy the same texel and mip, without resampling.
@group(0) @binding(0) var source:texture_2d<f32>;
@group(0) @binding(1) var destination:texture_storage_2d_array<rgba8unorm,write>;
@group(0) @binding(2) var<uniform> parameters:vec4<u32>;
@compute @workgroup_size(16,16,1)
fn copy_area_light(@builtin(global_invocation_id) id:vec3<u32>) {
    if all(id.xy<textureDimensions(source,parameters.x)) && all(id.xy<textureDimensions(destination)) {
        textureStore(destination,vec2<i32>(id.xy),0,textureLoad(source,vec2<i32>(id.xy),i32(parameters.x)));
    }
}
