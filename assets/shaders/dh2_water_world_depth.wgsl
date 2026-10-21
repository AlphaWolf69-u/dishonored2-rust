// Cached arkWater VS14487: current world instances, current/previous cameras.
struct WaterWorldMatrix {x:vec4<f32>,y:vec4<f32>,z:vec4<f32>,}
@group(1) @binding(0) var<storage,read> water_world_indices:array<u32>;
@group(1) @binding(1) var<storage,read> water_world_matrices:array<WaterWorldMatrix>;
@vertex fn water_world_depth_vertex(@location(0) position:vec3<f32>,
    @builtin(instance_index) instance:u32)->WaterDepthVertex {
    let source_instance=instance-bitcast<u32>(water_depth.controls.w);
    let index=water_world_indices[bitcast<u32>(water_depth.controls.x)+source_instance]&0x00ffffffu;
    let model=water_world_matrices[index];
    let p=vec4(position,1.0);
    let world=vec4(dot(p,model.x),dot(p,model.y),dot(p,model.z),1.0);
    let current=shadow_world_project(world,water_depth.current[0],water_depth.current[1],
        water_depth.current[2],water_depth.current[3]);
    let previous=shadow_world_project(world,water_depth.previous[0],water_depth.previous[1],
        water_depth.previous[2],water_depth.previous[3]);
    return WaterDepthVertex(current,current,previous);
}
