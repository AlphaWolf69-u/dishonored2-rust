// Host primary RGB is in camera-pre-exposed radiance units. Read the current
// GPU adaptation directly and encode each fetched colour into native HDR units.
// The original main target's packed storage quantization is not emulated here.
@group(0) @binding(4) var<storage,read> downscale_adaptation:array<vec4<f32>,2>;
fn downscale_engine_luminance()->f32 {return downscale_adaptation[0].x;}
fn downscale_source(uv:vec2<f32>,source_sampler:sampler)->vec4<f32> {
    let raw=textureSampleLevel(image,source_sampler,uv,0.0);
    return vec4(hdr_range_compress(raw.rgb/p.limits.w,p.options.y!=0u,
        downscale_engine_luminance(),p.limits.z),raw.a);
}
