// water/UnderWaterStencil and water/UnderWaterVolumeStencil. The caller
// supplies the actual wave-height mask or the original waterMatrix.
struct Parameters {
    world_from_clip:mat4x4<f32>,
    water_matrix:mat4x4<f32>,
    viewport:vec4<f32>, // origin XY, extent XY
    resolution_scale:vec4<f32>,
}
@group(0) @binding(0) var<uniform> parameters:Parameters;
@group(0) @binding(1) var water_line:texture_2d<f32>;
@group(0) @binding(2) var linear_clamp:sampler;
struct Vertex {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
    @location(1) height:f32,
}
@vertex fn vertex(@location(0) position:vec3<f32>,@location(1) uv:vec2<f32>)->Vertex {
    let p=vec4(position,1.0);
    return Vertex(p,uv,
        (parameters.water_matrix*p).z);
}
@fragment fn volume_fragment(in:Vertex) {
        let half_extent=parameters.viewport.zw*0.5;
        let ndc=vec2(1.0,-1.0)*((in.position.xy-parameters.viewport.xy-half_extent)/half_extent);
        let clip=vec4(ndc,0.01,1.0);
        let world=parameters.world_from_clip*clip;
        if world.z/world.w>in.height {discard;}
}
@fragment fn ocean_fragment(in:Vertex) {
        let mask=textureSampleLevel(water_line,linear_clamp,
            in.uv*parameters.resolution_scale.xy,0.0).r;
        if mask<0.5 {discard;}
}
