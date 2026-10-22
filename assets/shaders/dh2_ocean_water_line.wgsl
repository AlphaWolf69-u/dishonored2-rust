// water/DepthWaterLine. Compose after the shared ocean noise/surface helpers.
struct WaterLineParameters {
    world_from_clip:mat4x4<f32>,
    viewport:vec4<f32>,
    water_level:vec4<f32>,
}
@group(0) @binding(0) var<uniform> line:WaterLineParameters;
@group(0) @binding(1) var<uniform> surface:OceanSurfaceParameters;
@group(0) @binding(2) var amplitude_mask:texture_2d<f32>;
@group(0) @binding(3) var shore_distance:texture_2d<f32>;
@group(0) @binding(4) var collision_height:texture_2d<f32>;
@group(0) @binding(5) var linear_clamp:sampler;
@group(0) @binding(6) var linear_wrap:sampler;
@vertex fn vertex(@location(0) position:vec3<f32>)->@builtin(position) vec4<f32> {
    return vec4(position,1.0);
}
@fragment fn fragment(@builtin(position) position:vec4<f32>)->@location(0) f32 {
    let half_extent=line.viewport.zw*0.5;
    let ndc=vec2(1.0,-1.0)*((position.xy-line.viewport.xy-half_extent)/half_extent);
    let projected=line.world_from_clip*vec4(ndc,0.05,1.0);
    let world=projected.xyz/projected.w;
    var uv=world.xy/surface.spatial.w+vec2(0.5);
    uv.x=1.0-uv.x;
    let scaled=world.xy*surface.spatial.xy;
    let amplitude=textureSampleLevel(amplitude_mask,linear_clamp,uv,0.0).r;
    let distance=textureSampleLevel(shore_distance,linear_clamp,uv,0.0).r;
    let shore=ocean_shore_waves(surface,scaled,amplitude,distance);
    var height=textureSampleLevel(collision_height,linear_wrap,scaled*0.02,0.0).r;
    height*=amplitude*surface.spatial.z;
    height+=shore.height;
    height+=line.water_level.x;
    height+=0.02;
    return select(0.0,1.0,world.z<=height);
}
