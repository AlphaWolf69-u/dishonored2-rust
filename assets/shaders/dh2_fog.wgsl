// Concatenate after dh2_shadow and dh2_fog_math. Positions supplied to the
// interior pass are game coordinates; camera matrices use the viewer basis.
struct FogView {
    clip_from_world:mat4x4<f32>,
    world_from_view:mat4x4<f32>,
    world_from_clip:mat4x4<f32>,
    ray_scale:vec4<f32>, // inverse projection X/Y, far clip, native nearest threshold
    unproject:vec4<f32>, // -inverseProjection[2].w, inverseProjection[3].z/w, exposure
    extent:vec4<f32>, // full width/height, linear depth width/height
    reduced_extent:vec4<f32>, // linear depth width/height, fog width/height
    eye:vec4<f32>, // game XYZ, current jitter phase
    jitter:vec4<f32>, // subpixel offset XY, temporal jitter XY
}
struct FogCanvas {
    s:vec4<f32>,
    t:vec4<f32>,
    octaves:array<vec4<f32>,10>,
}
@group(0) @binding(0) var<uniform> fog_view:FogView;
@group(0) @binding(1) var<uniform> fog_parameters:WorldFogParameters;
@group(0) @binding(2) var fog_reduced_depth:texture_2d<f32>;
@group(0) @binding(3) var fog_depth:texture_depth_2d;
@group(0) @binding(4) var fog_interior:texture_2d<f32>;
@group(0) @binding(5) var fog_sky_mask:texture_2d<f32>;
@group(0) @binding(6) var fog_curve:texture_2d<f32>;
@group(0) @binding(7) var fog_clamp:sampler;
@group(0) @binding(8) var fog_noise:texture_2d<f32>;
@group(0) @binding(9) var fog_shadow:texture_2d<f32>;
@group(0) @binding(10) var<uniform> fog_canvas:FogCanvas;
@group(0) @binding(11) var fog_wrap:sampler;
@group(0) @binding(12) var fog_source:texture_2d<f32>;
@group(0) @binding(13) var fog_point:sampler;
@group(0) @binding(14) var fog_atlas:texture_depth_2d_array;
@group(0) @binding(16) var fog_comparison:sampler_comparison;
@group(0) @binding(17) var fog_projected_height:texture_2d<f32>;
@group(0) @binding(18) var<uniform> fog_shadow_parameters:FogShadowParameters;
@group(1) @binding(0) var<storage,read> fog_models:array<mat4x4<f32>>;

struct FogVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
}
@vertex fn fog_vertex(@builtin(vertex_index) index:u32)->FogVertex {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return FogVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
fn fog_game(v:vec3<f32>)->vec3<f32>{return vec3(v.x,-v.z,v.y);}
fn fog_linear_z(raw:f32)->f32 {
    return fog_view.unproject.x/(fog_view.unproject.y*raw+fog_view.unproject.z);
}
// waterDepthReduction gathers one 2x2 footprint at each destination UV,
// including quarter-resolution quality; it is not a recursive depth pyramid.
@fragment fn fog_depth_fragment(in:FogVertex)->@location(0) vec2<f32> {
    let raw=textureGather(fog_depth,fog_point,in.uv);
    let linear=vec4(fog_view.unproject.x)/(fog_view.unproject.y*raw+vec4(fog_view.unproject.z));
    let lo=min(linear.xy,linear.zw);
    let hi=max(linear.xy,linear.zw);
    return vec2(min(lo.x,lo.y),max(hi.x,hi.y));
}
fn fog_half_uv(uv:vec2<f32>)->vec2<f32> {
    return uv*fog_view.extent.zw/fog_view.reduced_extent.xy;
}
@vertex fn fog_interior_vertex(@location(0) position:vec3<f32>)->@builtin(position) vec4<f32> {
    return fog_view.clip_from_world*vec4(position.x,position.z,-position.y,1.0);
}
// Scene meshes already use the viewer basis. Keep the shared allocator's
// vertex/index data and apply each current instance transform on the GPU.
@vertex fn fog_interior_mesh_vertex(@location(0) position:vec3<f32>,@builtin(instance_index) instance:u32)->@builtin(position) vec4<f32> {
    return fog_view.clip_from_world*fog_models[instance]*vec4(position,1.0);
}
@fragment fn fog_interior_fragment(@builtin(position) position:vec4<f32>,@builtin(front_facing) front:bool)->@location(0) f32 {
    let uv=position.xy/fog_view.reduced_extent.zw;
    let scene=fog_linear_z(textureSampleLevel(fog_depth,fog_clamp,uv,0));
    let volume=fog_linear_z(position.z);
    return select(min(scene,volume),-clamp(volume,0.0,scene),front);
}
@fragment fn fog_canvas_fragment(in:FogVertex)->@location(0) f32 {
    var uv=(in.uv*vec2(2.0,-2.0)+vec2(-1.0,1.0)+fog_view.jitter.xy)
        *vec2(fog_view.extent.x/fog_view.extent.y,1.0);
    uv=vec2(dot(vec4(uv,0.0,1.0),fog_canvas.s),dot(vec4(uv,0.0,1.0),fog_canvas.t));
    var value=0.0;
    for(var octave=0u;octave<10u;octave++) {
        let o=fog_canvas.octaves[octave];
        value+=textureSampleLevel(fog_noise,fog_wrap,uv+o.xy,0.0).r*o.z;
        uv*=2.0;
    }
    return value;
}
@fragment fn fog_shadow_fragment(in:FogVertex)->@location(0) f32 {
    let m=fog_view.world_from_clip;
    let game_from_clip=mat4x4<f32>(vec4(fog_game(m[0].xyz),m[0].w),
        vec4(fog_game(m[1].xyz),m[1].w),vec4(fog_game(m[2].xyz),m[2].w),vec4(fog_game(m[3].xyz),m[3].w));
    return fog_shadow_term(fog_shadow_parameters,game_from_clip,fog_view.eye.xyz,
        vec2<u32>(in.position.xy),in.uv,fog_depth,fog_projected_height,fog_clamp,fog_atlas,fog_comparison);
}
@fragment fn fog_world_fragment(in:FogVertex)->@location(0) vec4<f32> {
    let linear=textureSampleLevel(fog_reduced_depth,fog_clamp,fog_half_uv(in.uv),0.0).y;
    let interior=textureSampleLevel(fog_interior,fog_clamp,in.uv,0.0).r;
    let cs_ray=vec4((in.uv*vec2(2.0,-2.0)+vec2(-1.0,1.0))*fog_view.ray_scale.xy,-1.0,0.0);
    let ray=fog_game((fog_view.world_from_view*cs_ray).xyz);
    let distance=length(ray)*(linear-interior);
    let normalized=fog_normalized_distance(fog_parameters,distance);
    let curve=textureSampleLevel(fog_curve,fog_clamp,vec2(normalized,0.5),0.0).r;
    let mask=textureGather(0,fog_sky_mask,fog_clamp,in.uv);
    let shadow=textureSampleLevel(fog_shadow,fog_clamp,in.uv,0.0).r;
    let noise=textureSampleLevel(fog_source,fog_clamp,in.uv,0.0).r;
    return fog_world(fog_parameters,WorldFogSample(ray,fog_view.eye.z,linear,interior,curve,shadow,mask,noise));
}
@fragment fn fog_upscale_fragment(in:FogVertex)->@location(0) vec4<f32> {
    let native_size=fog_view.extent.zw;
    let texel=1.0/native_size;
    let reduction=native_size.y/fog_view.extent.y;
    let uv=in.uv*(texel*reduction*fog_view.extent.xy);
    let padded_uv=uv*native_size/fog_view.reduced_extent.xy;
    let depths=textureGather(1,fog_reduced_depth,fog_clamp,padded_uv);
    let full=fog_linear_z(textureSampleLevel(fog_depth,fog_point,in.uv,0));
    let values=array<f32,4>(depths.w,depths.z,depths.x,depths.y);
    let uv00=uv-0.5*texel;
    let positions=array<vec2<f32>,4>(uv00,uv00+vec2(texel.x,0.0),uv00+vec2(0.0,texel.y),uv00+texel);
    var nearest=uv00;var minimum=fog_view.ray_scale.z;var flat=true;
    for(var i=0u;i<4u;i++) {
        let distance=abs(values[i]-full);
        if distance<minimum {minimum=distance;nearest=positions[i];}
        flat=flat && distance<fog_view.ray_scale.w;
    }
    var color=textureSampleLevel(fog_source,fog_point,nearest,0.0);
    if flat {color=textureSampleLevel(fog_source,fog_clamp,uv,0.0);}
    return vec4(color.rgb*139.26*fog_view.unproject.w,fog_saturate(color.a));
}
// renderUpscale: curvature steers a point fetch in the reduced fog image.
// 30bb70 derives the bounds from TexDepth (depthWater), not the fog image.
@fragment fn fog_upscale_slope_fragment(in:FogVertex)->@location(0) vec4<f32> {
    let uv=in.uv;
    let offset=vec2(2.0)/fog_view.extent.xy;
    let center=fog_linear_z(textureSampleLevel(fog_depth,fog_point,uv,0));
    let left=fog_linear_z(textureSampleLevel(fog_depth,fog_point,uv-vec2(offset.x,0.0),0));
    let right=fog_linear_z(textureSampleLevel(fog_depth,fog_point,uv+vec2(offset.x,0.0),0));
    let top=fog_linear_z(textureSampleLevel(fog_depth,fog_point,uv-vec2(0.0,offset.y),0));
    let bottom=fog_linear_z(textureSampleLevel(fog_depth,fog_point,uv+vec2(0.0,offset.y),0));
    let bound=(sqrt(2.0)*2.0)/vec2<f32>(textureDimensions(fog_depth));
    let sign=vec2(right-left,bottom-top)*bound;
    let slope=clamp(vec2((center-right)+(center-left),(center-top)+(center-bottom))*sign,-bound,bound)*0.5;
    let color=textureSampleLevel(fog_source,fog_point,uv+slope,0.0);
    return vec4(color.rgb*139.26*fog_view.unproject.w,fog_saturate(color.a));
}
