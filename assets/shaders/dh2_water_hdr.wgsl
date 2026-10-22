// Host integration around the game's pre-water CopyResource and colour draws.
// The selected native-format attachments perform the real storage rounding.
struct WaterHdrParameters {limits:vec4<f32>,options:vec4<u32>,}
@group(0) @binding(0) var<uniform> water_hdr:WaterHdrParameters;
@group(0) @binding(1) var water_hdr_source:texture_2d<f32>;
@group(0) @binding(2) var<storage,read> water_hdr_adaptation:array<vec4<f32>,2>;

@vertex fn water_hdr_vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((i<<1u)&2u),f32(i&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
struct WaterHdrCopies {@location(0) source:vec4<f32>,@location(1) colour:vec4<f32>,}
@fragment fn water_hdr_encode(@builtin(position) pixel:vec4<f32>)->WaterHdrCopies {
    let raw=textureLoad(water_hdr_source,vec2<i32>(pixel.xy),0);
    let encoded=hdr_range_compress(raw.rgb/water_hdr.limits.y,water_hdr.options.x!=0u,
        water_hdr_adaptation[0].x,water_hdr.limits.x);
    let value=vec4(encoded,raw.a);
    // Both copies receive the same native-format store. Subsequent water draws
    // mutate only colour; source remains the immutable pre-water image.
    return WaterHdrCopies(value,value);
}
@fragment fn water_hdr_decode(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let raw=textureLoad(water_hdr_source,vec2<i32>(pixel.xy),0).rgb;
    let expanded=hdr_range_expand(raw,water_hdr.options.x!=0u,
        water_hdr_adaptation[0].x,water_hdr.limits.x);
    // The pipeline writes RGB only, preserving the host target's existing alpha.
    return vec4(expanded*water_hdr.limits.y,0.0);
}
