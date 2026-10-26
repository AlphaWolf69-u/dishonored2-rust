// Host integration: apply the existing NOT_EQUAL 0x40 stencil test while fog
// resolution and the shared camera stencil attachment have different extents.
@group(0) @binding(7) var ray_stencil:texture_2d<u32>;
@fragment fn fog_ray_scatter_stencil(in:RayVertex)->@location(0) vec4<f32> {
    let extent=textureDimensions(ray_stencil);
    let p=clamp(vec2<i32>(in.uv*vec2<f32>(extent)),vec2(0),vec2<i32>(extent)-vec2(1));
    if (textureLoad(ray_stencil,p,0).x&64u)!=0u {discard;}
    return fog_ray_scatter(in);
}
