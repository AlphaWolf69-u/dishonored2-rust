// water/FoamDepth: the authored four gathers (the other eight offsets are unused).
struct Parameters {
    inverse_view_projection:mat4x4<f32>,
    camera:vec4<f32>,
    // -inverseProjection[2].w, inverseProjection[3][2:3], camera forward Z.
    linear_depth:vec4<f32>,
    // globalViewInfos.zw, unused padding.
    near_far:vec4<f32>,
}
@group(0) @binding(0) var<uniform> p:Parameters;
@group(0) @binding(1) var opaque:texture_depth_2d;
@group(0) @binding(2) var water:texture_depth_2d;
@group(0) @binding(3) var point_clamp:sampler;
struct Vertex { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32>, }
@vertex fn vertex(@builtin(vertex_index) index:u32)->Vertex {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return Vertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
fn linear_z(raw:f32)->f32 {return p.linear_depth.x/(p.linear_depth.y*raw+p.linear_depth.z);}
fn linear_z4(raw:vec4<f32>)->vec4<f32> {return p.linear_depth.x/(p.linear_depth.y*raw+vec4(p.linear_depth.z));}
fn world_position(ndc:vec2<f32>,z:f32)->vec3<f32> {
    let far_h=p.inverse_view_projection*vec4(ndc,0.0,1.0);
    let far=far_h.xyz/far_h.w;
    return p.camera.xyz+(far-p.camera.xyz)*(z/p.near_far.y);
}
fn reverse_smooth(high:f32,x:f32)->f32 {
    // HLSL smoothstep(high,0,x); WGSL smoothstep requires ordered edges.
    let t=clamp((x-high)/(0.0-high),0.0,1.0);
    return t*t*(3.0-2.0*t);
}
@fragment fn fragment(in:Vertex)->@location(0) f32 {
    var max_opaque=p.near_far.x;
    var min_opaque=p.near_far.y;
    var min_water=p.near_far.y;
    let ndc=vec2(in.uv.x*2.0-1.0,1.0-in.uv.y*2.0);
    let center_opaque=linear_z(textureSampleLevel(opaque,point_clamp,in.uv,0));
    let center_water=linear_z(textureSampleLevel(water,point_clamp,in.uv,0));
    let ws_opaque=world_position(ndc,center_opaque);
    let ws_water=world_position(ndc,center_water);
    // Gather offsets must be compile-time constants in WGSL.
    let o=array<vec4<f32>,4>(
        linear_z4(textureGather(opaque,point_clamp,in.uv,vec2<i32>(0,-2))),
        linear_z4(textureGather(opaque,point_clamp,in.uv,vec2<i32>(1,0))),
        linear_z4(textureGather(opaque,point_clamp,in.uv,vec2<i32>(-1,1))),
        linear_z4(textureGather(opaque,point_clamp,in.uv,vec2<i32>(-2,1))));
    let w=array<vec4<f32>,4>(
        linear_z4(textureGather(water,point_clamp,in.uv,vec2<i32>(0,-2))),
        linear_z4(textureGather(water,point_clamp,in.uv,vec2<i32>(1,0))),
        linear_z4(textureGather(water,point_clamp,in.uv,vec2<i32>(-1,1))),
        linear_z4(textureGather(water,point_clamp,in.uv,vec2<i32>(-2,1))));
    for(var i=0u;i<4u;i++) {
        max_opaque=max(max_opaque,max(max(o[i].x,o[i].y),max(o[i].z,o[i].w)));
        min_opaque=min(min_opaque,min(min(o[i].x,o[i].y),min(o[i].z,o[i].w)));
        min_water=min(min_water,min(min(w[i].x,w[i].y),min(w[i].z,w[i].w)));
    }
    let ws_min_opaque=world_position(ndc,min_opaque);
    let ws_max_opaque=world_position(ndc,max_opaque);
    let ws_min_water=world_position(ndc,min_water);
    let dir_min=ws_min_opaque-ws_min_water;
    let dir_max=ws_max_opaque-ws_min_water;
    let high=clamp((min_water-1.0)/49.0,0.0,1.0);
    let low=mix(high,0.1,high);
    let pitch=mix(high,low,p.linear_depth.w*p.linear_depth.w);
    let threshold=max(0.005,(0.19*center_water-0.19)*pitch);
    var front=0.0;
    if dot(dir_min,dir_min)>0.001 {front=reverse_smooth(threshold,dot(dir_max,dir_max));}
    let to_min=ws_min_opaque-ws_water;
    let to_center=ws_opaque-ws_water;
    let sqr_min=dot(to_min,to_min);
    var back=0.0;
    if sqr_min>0.001 && min_opaque<center_water && dot(to_center,to_center)>0.001 {
        back=reverse_smooth(0.05,sqr_min);
    }
    return max(front,back)*clamp((center_water-250.0)/(-250.0),0.0,1.0);
}
