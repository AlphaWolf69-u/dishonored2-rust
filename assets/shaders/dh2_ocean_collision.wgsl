// FFTWaves/UpdateCollisionHeightField, using the finite 1402fa4e0 grid.
@group(0) @binding(0) var displacement: texture_2d<f32>;
@group(0) @binding(1) var bilinear_wrap: sampler;
struct CollisionVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) height: f32,
}
@vertex fn collision_vertex(@location(0) coordinates: vec2<f32>) -> CollisionVertex {
    let shifted = textureSampleLevel(displacement, bilinear_wrap, coordinates, 0.0).xyz;
    let uv = coordinates + shifted.xy / 50.0;
    let ndc = uv * 2.0 - 1.0;
    var output: CollisionVertex;
    output.position = vec4<f32>(ndc.x, -ndc.y, 0.0, 1.0);
    output.height = shifted.z;
    return output;
}
@fragment fn collision_fragment(input: CollisionVertex) -> @location(0) f32 {
    return input.height;
}
