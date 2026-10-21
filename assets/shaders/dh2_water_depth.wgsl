// Rigid arkWater cached VS14476/14463; wind/skinning have separate variants.
struct WaterDepthParameters {
    current:array<vec4<f32>,4>,
    previous:array<vec4<f32>,4>,
    // modelDepthHack, jitterRelativeToPreviousFrame.xy, unused
    controls:vec4<f32>,
}
@group(0) @binding(0) var<uniform> water_depth:WaterDepthParameters;
struct WaterDepthVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) @interpolate(perspective,center) current:vec4<f32>,
    @location(1) @interpolate(perspective,center) previous:vec4<f32>,
}
@vertex fn water_depth_vertex(@location(0) position:vec3<f32>)->WaterDepthVertex {
    let p=vec4(position,1.0);
    let current=vec4(dot(p,water_depth.current[0]),dot(p,water_depth.current[1]),
        dot(p,water_depth.current[2])*water_depth.controls.x,dot(p,water_depth.current[3]));
    let previous=vec4(dot(p,water_depth.previous[0]),dot(p,water_depth.previous[1]),
        dot(p,water_depth.previous[2])*water_depth.controls.x,dot(p,water_depth.previous[3]));
    return WaterDepthVertex(current,current,previous);
}
@fragment fn water_depth_fragment(vertex:WaterDepthVertex)->@location(0) vec4<f32> {
    return screen_motion_vector(vertex.current,vertex.previous,water_depth.controls.yz);
}
