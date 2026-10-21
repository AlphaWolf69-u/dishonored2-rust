struct WaterColourQuery {world:vec4<f32>,normal:vec4<f32>,tangent:vec4<f32>,pixel:vec4<f32>,}
@group(0) @binding(0) var<uniform> p:WaterColourParameters;
@group(0) @binding(1) var permutation:texture_2d<f32>;
@group(0) @binding(2) var gradients:texture_2d<f32>;
@group(0) @binding(3) var<storage,read> queries:array<WaterColourQuery>;
@group(0) @binding(4) var<storage,read_write> output:array<vec4<f32>>;
@group(0) @binding(5) var point_wrap:sampler;
@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id:vec3<u32>) {
    let q=queries[id.x];
    let v=water_colour_input(p,q.world.xyz,q.pixel.xy,q.world.w!=0.0,q.normal.xyz,q.tangent.xyz,q.normal.w,
        vec3(0u),vec3(0.0),permutation,gradients,point_wrap);
    let i=id.x*9u;
    output[i]=vec4(v.position,v.water_level);output[i+1u]=vec4(v.view_position,select(0.0,1.0,v.is_ocean));
    output[i+2u]=vec4(v.normal,select(0.0,1.0,v.underwater));output[i+3u]=vec4(v.view_direction,v.specular_power);
    output[i+4u]=vec4(v.pixel,v.refraction,v.foam_fft);output[i+5u]=vec4(v.color,v.extinction);
    output[i+6u]=vec4(v.sun_direction,v.inscattering);output[i+7u]=vec4(v.sun_intensity,v.additional_shadow);
    output[i+8u]=vec4(v.foam,v.foam_depth,v.wave_amplitude,0.0);
}
