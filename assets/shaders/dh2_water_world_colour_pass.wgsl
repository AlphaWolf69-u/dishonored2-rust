// Horizontal WORLD/IBL arkWater colour specialization, following VS14494.
@group(2) @binding(0) var<uniform> water:WorldProbeColourParameters;
@group(3) @binding(0) var permutation:texture_2d<f32>;
@group(3) @binding(1) var gradients:texture_2d<f32>;
@group(3) @binding(2) var opaque:texture_depth_2d;
@group(3) @binding(3) var reduced:texture_2d<f32>;
@group(3) @binding(4) var hdr:texture_2d<f32>;
@group(3) @binding(5) var fog:texture_2d<f32>;
@group(3) @binding(6) var caustics:texture_2d_array<f32>;
@group(3) @binding(7) var probes:texture_cube_array<f32>;
@group(3) @binding(8) var sky:texture_cube<f32>;
@group(3) @binding(9) var shadow:texture_depth_2d_array;
@group(3) @binding(10) var point_wrap:sampler;
@group(3) @binding(11) var linear_clamp:sampler;
@group(3) @binding(12) var probe_sampler:sampler;
@group(3) @binding(13) var sky_sampler:sampler;
@group(3) @binding(14) var caustic_sampler:sampler;
@group(3) @binding(15) var comparison:sampler_comparison;
struct WorldProbeColourFragment {
    @builtin(position) pixel:vec4<f32>,
    @location(0) indices_world_x:vec4<f32>,
    @location(1) world_yz_barycentrics:vec4<f32>,
    @location(2) @interpolate(flat) single_cube:u32,
}
fn water_world_colour(vertex:WorldProbeColourFragment,
    front:bool,parameters:WorldProbeColourParameters)->vec4<f32> {
    let position=vec3(vertex.indices_world_x.w,vertex.world_yz_barycentrics.xy);
    // This specialization is admitted only for horizontal water. The installed
    // horizontal branch does not fetch or use geometric normals/tangents.
    let input=water_colour_input_with_probes(parameters.colour,position,vertex.pixel.xy,
        front,vec3(0.0),vec3(0.0),0.0,parameters.options.y==0u,parameters.options.z==0u,vertex.single_cube,
        vertex.indices_world_x.xyz,vertex.world_yz_barycentrics.zw,
        permutation,gradients,point_wrap);
    let environment=WaterEnvironment(parameters.sky_radius.xyz,parameters.sky_radius.w,
        parameters.options.y==0u,parameters.options.z==0u,parameters.options.x!=0u);
    let result=water_surface(parameters.surface,parameters.reflection,parameters.refraction,parameters.shadow,environment,input,
        opaque,reduced,hdr,fog,caustics,probes,sky,shadow,
        linear_clamp,caustic_sampler,probe_sampler,sky_sampler,comparison);
    return vec4(colour_output(parameters.output,result.rgb),result.a);
}

@fragment fn water_world_colour_fragment(vertex:WorldProbeColourFragment,
    @builtin(front_facing) front:bool)->@location(0) vec4<f32> {
    return water_world_colour(vertex,front,water);
}
