
struct Params {surface:OceanSurfaceParameters,colour:OceanColourParameters,view_from_world:mat4x4<f32>,refraction:vec4<f32>,}
struct Query {world:vec4<f32>,legacy:vec4<f32>,pixel:vec4<f32>,}
@group(0) @binding(0) var<uniform> p:Params;
@group(0) @binding(1) var permutation:texture_2d<f32>;
@group(0) @binding(2) var gradients:texture_2d<f32>;
@group(0) @binding(3) var shore:texture_2d<f32>;
@group(0) @binding(4) var displacement:texture_2d<f32>;
@group(0) @binding(5) var previous:texture_2d<f32>;
@group(0) @binding(6) var collision:texture_2d<f32>;
@group(0) @binding(7) var fft_foam:texture_2d<f32>;
@group(0) @binding(8) var amplitude:texture_2d<f32>;
@group(0) @binding(9) var projected_height:texture_2d<f32>;
@group(0) @binding(10) var foam_texture:texture_2d<f32>;
@group(0) @binding(11) var foam_depth:texture_2d<f32>;
@group(0) @binding(12) var<storage,read> queries:array<Query>;
@group(0) @binding(13) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(14) var point_wrap:sampler;
@group(0) @binding(15) var bilinear_wrap:sampler;
@group(0) @binding(16) var bilinear_clamp:sampler;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];
    let foam_uv=q.legacy.xy*0.04+vec2(sin(p.surface.clock_view.x)*0.01);
    let foam=textureSampleLevel(foam_texture,bilinear_wrap,foam_uv,0.0).r;
    let uv=q.pixel.xy*p.colour.pixel_to_texture.zw;
    let depth=textureSampleLevel(foam_depth,bilinear_wrap,uv,0.0).r;
    let v=ocean_colour_input(p.colour,p.surface,p.view_from_world,q.world.xyz,q.legacy,q.pixel.xy,
        q.pixel.z!=0.0,p.refraction.x,foam,depth,permutation,gradients,point_wrap,bilinear_wrap,
        bilinear_clamp,shore,displacement,collision,fft_foam,amplitude,projected_height);
    let i=id.x*9u;
    output[i]=vec4(v.position,v.water_level);output[i+1u]=vec4(v.view_position,select(0.0,1.0,v.is_ocean));
    output[i+2u]=vec4(v.normal,select(0.0,1.0,v.underwater));output[i+3u]=vec4(v.view_direction,v.specular_power);
    output[i+4u]=vec4(v.pixel,v.refraction,v.foam_fft);output[i+5u]=vec4(v.color,v.extinction);
    output[i+6u]=vec4(v.sun_direction,v.inscattering);output[i+7u]=vec4(v.sun_intensity,v.additional_shadow);
    output[i+8u]=vec4(v.foam,v.foam_depth,v.wave_amplitude,0.0);
}
