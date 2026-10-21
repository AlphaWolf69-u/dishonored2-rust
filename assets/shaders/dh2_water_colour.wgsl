// Per-material/view inputs to the placed arkWater fragment program.
struct WaterColourParameters {
    normals:WaterNormalParameters,
    view_from_world:mat4x4<f32>,
    camera_refraction:vec4<f32>,
    water:vec4<f32>, // WaterColor.xyz, Extinction
    light:vec4<f32>, // parallelLightDir.xyz, Inscattering
    sun:vec4<f32>, // parallelLightColor.rgb, SpecularPower
    options:vec4<u32>, // IsHorizontalWaterSurface, unused
}
fn water_colour_input(parameters:WaterColourParameters,position:vec3<f32>,pixel:vec2<f32>,
    front_face:bool,normal:vec3<f32>,tangent:vec3<f32>,bitangent_sign:f32,
    probe_indices:vec3<u32>,barycentrics:vec3<f32>,
    permutation:texture_2d<f32>,gradients:texture_2d<f32>,point_wrap:sampler)->WaterSurfaceInput {
    let view_position=(parameters.view_from_world*vec4(position,1.0)).xyz;
    let to_camera=parameters.camera_refraction.xyz-position;
    let distance=length(to_camera);
    let direction=to_camera/distance;
    let surface_normal=water_surface_normal(parameters.normals,position.xy,distance,
        parameters.options.x!=0u,normal,tangent,bitangent_sign,permutation,gradients,point_wrap);
    // The placed program uses the actual surface Z, no extra shadow, no foam
    // and a full wave-amplitude mask; these are constants in its source call.
    return WaterSurfaceInput(false,!front_face,position.z,pixel,position,view_position,
        surface_normal,direction,parameters.water.xyz,parameters.water.w,parameters.light.w,
        parameters.light.xyz,parameters.sun.xyz,parameters.sun.w,1.0,parameters.camera_refraction.w,
        0.0,0.0,0.0,1.0,probe_indices,barycentrics);
}

fn water_colour_input_with_probes(parameters:WaterColourParameters,position:vec3<f32>,pixel:vec2<f32>,
    front_face:bool,normal:vec3<f32>,tangent:vec3<f32>,bitangent_sign:f32,
    use_probes:bool,per_vertex_probes:bool,single_cube:u32,
    weighted_indices:vec3<f32>,barycentrics_xy:vec2<f32>,
    permutation:texture_2d<f32>,gradients:texture_2d<f32>,point_wrap:sampler)->WaterSurfaceInput {
    let probes=water_probe_pixel(use_probes,per_vertex_probes,single_cube,weighted_indices,barycentrics_xy);
    return water_colour_input(parameters,position,pixel,front_face,normal,tangent,bitangent_sign,
        probes.indices,probes.barycentrics,permutation,gradients,point_wrap);
}
