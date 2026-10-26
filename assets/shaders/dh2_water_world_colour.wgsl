// arkWater VS14494. Positions and packed normal W retain game-space values.
// controls: instance-buffer offset, allocator vertex base, draw first-instance.
struct WaterWorldColourParameters {vp:array<vec4<f32>,4>,controls:vec4<u32>,}
@group(0) @binding(0) var<uniform> water_world_colour:WaterWorldColourParameters;
struct WaterColourMatrix {x:vec4<f32>,y:vec4<f32>,z:vec4<f32>,}
@group(1) @binding(0) var<storage,read> water_colour_instances:array<u32>;
@group(1) @binding(1) var<storage,read> water_colour_matrices:array<WaterColourMatrix>;
// Uses the original PNW section offsets at header words 0..3. The shared
// ProbeLightingSet header also retains metadata for other lighting consumers.
@group(1) @binding(2) var<storage,read> water_colour_pnw:array<u32>;
@group(1) @binding(3) var<storage,read> water_colour_probe_info:array<vec2<u32>>;
@group(1) @binding(4) var<storage,read> water_colour_cube_indices:array<u32>;
@group(1) @binding(5) var<storage,read> water_colour_object_probes:array<vec4<f32>>;
struct WaterWorldColourVertex {
    @location(0) indices_world_x:vec4<f32>,
    @location(1) world_yz_barycentrics:vec4<f32>,
    @builtin(position) clip:vec4<f32>,
    @location(2) @interpolate(flat) single_cube:u32,
}
fn water_world_colour_input(position:vec3<f32>,normal_w:f32,
    vertex:u32,instance:u32)->WaterWorldColourVertex {
    // D3D11 SV_InstanceID is draw-relative; WGPU's builtin includes first-instance.
    let source_instance=instance-water_world_colour.controls.z;
    let index=water_colour_instances[water_world_colour.controls.x+source_instance]&0x00ffffffu;
    let model=water_colour_matrices[index];
    let p=vec4(position,1.0);
    let world=vec3(dot(p,model.x),dot(p,model.y),dot(p,model.z));
    let reference=water_colour_pnw[water_colour_pnw[0]+index]
        +vertex-water_world_colour.controls.y;
    let tetra=water_colour_pnw[water_colour_pnw[1]+reference];
    let base=water_colour_pnw[2]+6u*tetra;
    var points:array<vec3<f32>,4>;
    var weights:array<vec4<f32>,4>;
    for(var i=0u;i<4u;i++) {
        let v=water_colour_pnw[3]+4u*water_colour_pnw[base+i];
        points[i]=bitcast<vec3<f32>>(vec3(water_colour_pnw[v],water_colour_pnw[v+1u],water_colour_pnw[v+2u]));
        let packed=water_colour_pnw[v+3u];
        weights[i]=vec4<f32>(vec4(packed&255u,(packed>>8u)&255u,(packed>>16u)&255u,packed>>24u))*(1.0/255.0);
    }
    let ids0=water_colour_pnw[base+4u];
    let ids1=water_colour_pnw[base+5u];
    // Selection compares interpolated weights but carries IBL IDs from ProbeInfo.
    let ids=vec4(water_colour_probe_info[ids0&65535u].x,
        water_colour_probe_info[ids0>>16u].x,water_colour_probe_info[ids1&65535u].x,
        water_colour_probe_info[ids1>>16u].x);
    let selected=water_probe_select(world,points,weights,ids);
    let transport=water_probe_vertex(water_colour_cube_indices[selected],normal_w);
    let vp=water_world_colour.vp;
    return WaterWorldColourVertex(vec4(transport.weighted_indices,world.x),
        vec4(world.yz,transport.barycentrics),
        shadow_world_project(vec4(world,1.0),vp[0],vp[1],vp[2],vp[3]),0u);
}
@vertex fn water_world_colour_vertex(@location(0) position:vec3<f32>,
    @location(1) normal_w:f32,@builtin(vertex_index) vertex:u32,
    @builtin(instance_index) instance:u32)->WaterWorldColourVertex {
    return water_world_colour_input(position,normal_w,vertex,instance);
}

// VS14454/14465/14480 omit PNW and normal-W entirely. Carry the same world
// position into shared shading without accessing any probe reference buffer.
fn water_colour_position(position:vec3<f32>,instance:u32)->WaterWorldColourVertex {
    let source_instance=instance-water_world_colour.controls.z;
    let index=water_colour_instances[water_world_colour.controls.x+source_instance]&0x00ffffffu;
    let model=water_colour_matrices[index];
    let p=vec4(position,1.0);
    let world=vec3(dot(p,model.x),dot(p,model.y),dot(p,model.z));
    let vp=water_world_colour.vp;
    return WaterWorldColourVertex(vec4(vec3(0.0),world.x),vec4(world.yz,vec2(0.0)),
        shadow_world_project(vec4(world,1.0),vp[0],vp[1],vp[2],vp[3]),0u);
}
@vertex fn water_world_colour_vertex_without_probes(@location(0) position:vec3<f32>,
    @builtin(instance_index) instance:u32)->WaterWorldColourVertex {
    return water_colour_position(position,instance);
}

// VS14483/14488 select ONE cube for the entire model. Its tetrahedron and
// barycentrics come from the same moving-object probe query as opaque lighting;
// there is no world-vertex reference or packed normal-W corner in this ABI.
@vertex fn water_model_colour_vertex(@location(0) position:vec3<f32>,
    @builtin(instance_index) instance:u32)->WaterWorldColourVertex {
    var output=water_colour_position(position,instance);
    let record=9u*water_world_colour.controls.w;
    let bary=vec4(water_colour_object_probes[record+2u].w,
        water_colour_object_probes[record+3u].w,
        water_colour_object_probes[record+4u].w,
        water_colour_object_probes[record+5u].w);
    let tetra=u32(water_colour_object_probes[record+6u].w);
    let base=water_colour_pnw[2]+6u*tetra;
    var weights:array<vec4<f32>,4>;
    for(var i=0u;i<4u;i++) {
        let v=water_colour_pnw[3]+4u*water_colour_pnw[base+i];
        let packed=water_colour_pnw[v+3u];
        weights[i]=vec4<f32>(vec4(packed&255u,(packed>>8u)&255u,
            (packed>>16u)&255u,packed>>24u))*bary[i];
    }
    // Preserve the installed stage's accumulation order and strict first-tie
    // selection; selecting from rounded CPU SH weights can change the winner.
    let scale=vec4(1.0/255.0);
    var w=weights[1]*scale;
    w=fma(weights[0],scale,w);
    w=fma(weights[2],scale,w);
    w=fma(weights[3],scale,w);
    var selected=0u;
    if w.y>w[selected] {selected=1u;}
    if w.z>w[selected] {selected=2u;}
    if w.w>w[selected] {selected=3u;}
    let packed=water_colour_pnw[base+4u+selected/2u];
    let primary=(packed>>(16u*(selected&1u)))&65535u;
    let reflection=water_colour_probe_info[primary].x;
    output.single_cube=water_colour_cube_indices[reflection];
    return output;
}
