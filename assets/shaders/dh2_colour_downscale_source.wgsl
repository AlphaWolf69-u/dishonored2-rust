fn downscale_engine_luminance()->f32 {return p.limits.y;}
fn downscale_source(uv:vec2<f32>,source_sampler:sampler)->vec4<f32> {
    return textureSampleLevel(image,source_sampler,uv,0.0);
}
