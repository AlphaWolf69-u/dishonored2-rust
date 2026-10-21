// Concatenated before dh2_sky.wgsl, sharing the existing sky colour program.
struct SkyView {
    clip_from_world: mat4x4<f32>,
    time: f32,
    exposure: f32,
    padding: vec2<f32>,
}
struct SkySurface {
    sky: array<vec4<f32>,7>,
    world_from_local: mat4x4<f32>,
    fog: vec4<f32>, // depth offset, environment masking factor
    depth_uv: vec4<f32>,
    flags: vec4<u32>, // clouds, project to far plane
}
@group(0) @binding(0) var<uniform> globals: SkyView;
@group(0) @binding(1) var<uniform> surface: SkySurface;
@group(1) @binding(0) var diffuse0: texture_2d<f32>;
@group(1) @binding(1) var cloud_opacity: texture_2d<f32>;
@group(1) @binding(2) var cloud_density: texture_2d<f32>;
@group(1) @binding(3) var cloud_back_density: texture_2d<f32>;
@group(1) @binding(4) var sky_depth: texture_2d<f32>;
@group(1) @binding(5) var surface_sampler: sampler;

struct SkyVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) colour: vec4<f32>,
}
@vertex fn sky_mask_vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) colour: vec4<f32>,
) -> SkyVertex {
    let world=surface.world_from_local*vec4(position,1.0);
    var clip=globals.clip_from_world*world;
    if surface.flags.y!=0u {clip.z=0.0;}
    return SkyVertex(clip,uv,world.xyz,
        normalize((surface.world_from_local*vec4(normal,0.0)).xyz),colour);
}
struct SkyTargets {
    @location(0) colour: vec4<f32>,
    @location(1) mask: vec4<f32>,
}
@fragment fn sky_mask_fragment(in: SkyVertex) -> SkyTargets {
    let cloud=surface.flags.x!=0u;
    let colour=sky_colour(in.uv,in.world,in.normal,in.colour,cloud);
    var uv=in.uv*surface.depth_uv.xy+surface.depth_uv.zw;
    if cloud {uv=in.uv+vec2(globals.time*surface.sky[2].y,0.0);}
    let depth=textureSampleGrad(sky_depth,surface_sampler,uv,dpdx(uv),dpdy(uv)).r;
    var mask=clamp(1.0-pow(1.0-depth,surface.fog.x),0.0,1.0)*surface.fog.y;
    if cloud {mask*=colour.a;}
    return SkyTargets(vec4(colour.rgb*139.26*globals.exposure,colour.a),vec4(mask,0.0,0.0,0.0));
}
