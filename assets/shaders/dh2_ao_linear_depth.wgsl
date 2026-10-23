@group(1) @binding(0) var ao_hardware_depth:texture_depth_2d;
@fragment fn ao_linear_depth(input:AoVertex)->@location(0) f32 {
    let raw=textureLoad(ao_hardware_depth,vec2<i32>(input.position.xy+ao.p[7].xy),0);
    let normalized=clamp(ao.p[6].z*raw+ao.p[6].w,0.0,1.0);
    return 1.0/(normalized*ao.p[6].x+ao.p[6].y);
}
