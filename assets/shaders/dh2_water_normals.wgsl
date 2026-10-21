// arkWater's distance-dependent Perlin detail. This affects shading normals;
// the placed mesh's vertex positions are not displaced by ComputeWaves.
struct WaterNormalParameters {
    // localTime.x, PerlinMaxOctaves, PerlinAntiAlias, PerlinFrequency.
    clock_octaves:vec4<f32>,
    // PerlinSpeed, PerlinPersistence, PerlinAmplitude, unused.
    waves:vec4<f32>,
}
fn water_wave_height(parameters:WaterNormalParameters,position:vec2<f32>,distance:f32,
    permutation:texture_2d<f32>,gradients:texture_2d<f32>,point_wrap:sampler)->f32 {
    let raw=max(parameters.clock_octaves.y/(1.0+distance*parameters.clock_octaves.z),1.0);
    let octaves=1+i32(floor(raw));
    let last=raw-trunc(raw);
    // The authored windSpeed vector is multiplied by zero in this program.
    let movement=vec2(0.8,0.6)*0.0*parameters.clock_octaves.x;
    let sample_position=vec3((position+movement)*parameters.clock_octaves.w,
        parameters.clock_octaves.x*parameters.waves.x);
    var height=noise_fractal_three(permutation,gradients,point_wrap,sample_position,
        octaves,last,vec3(2.054,2.042,1.3),parameters.waves.y,2);
    height*=parameters.waves.z;
    return height*0.1;
}
fn water_wave_normal(parameters:WaterNormalParameters,position:vec2<f32>,distance:f32,
    permutation:texture_2d<f32>,gradients:texture_2d<f32>,point_wrap:sampler)->vec3<f32> {
    let h=water_wave_height(parameters,position,distance,permutation,gradients,point_wrap);
    let dx=water_wave_height(parameters,position+vec2(0.01,0.0),distance,permutation,gradients,point_wrap);
    let dy=water_wave_height(parameters,position+vec2(0.0,0.01),distance,permutation,gradients,point_wrap);
    return normalize(vec3(h-dx,h-dy,0.01));
}
fn water_surface_normal(parameters:WaterNormalParameters,position:vec2<f32>,distance:f32,
    horizontal:bool,normal:vec3<f32>,tangent:vec3<f32>,bitangent_sign:f32,
    permutation:texture_2d<f32>,gradients:texture_2d<f32>,point_wrap:sampler)->vec3<f32> {
    let detail=water_wave_normal(parameters,position,distance,permutation,gradients,point_wrap);
    if horizontal {return detail;}
    let bitangent=cross(normal,tangent)*sign(bitangent_sign);
    return normalize(detail.x*tangent+detail.y*bitangent+detail.z*normal);
}
