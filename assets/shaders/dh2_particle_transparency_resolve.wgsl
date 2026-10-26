// PostFX_CompositeWeightedBlendedOIT's alpha contribution. This owner has no
// hair draw list; that list's clear contribution is zero with revealage one.
struct ParticleResolveParameters {limits:vec4<f32>,}
@group(0) @binding(0) var particle_accumulation:texture_2d<f32>;
@group(0) @binding(1) var particle_revealage:texture_2d<f32>;
@group(0) @binding(2) var<uniform> particle_resolve:ParticleResolveParameters;
@group(0) @binding(3) var<storage,read> resolve_adaptation:array<vec4<f32>,2>;
@vertex fn resolve_vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((i<<1u)&2u),f32(i&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn resolve_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let position=vec2<i32>(pixel.xy);
    let opacity=textureLoad(particle_revealage,position,0).x;
    let accumulated=textureLoad(particle_accumulation,position,0);
    var colour=accumulated.rgb;
    if accumulated.a>0.0 {colour*=1.0/accumulated.a;}
    colour=clamp(colour*(1.0-opacity),vec3(0.0),vec3(65000.0));
    return vec4(hdr_range_compress(colour,particle_resolve.limits.y!=0.0,
        resolve_adaptation[0].x,particle_resolve.limits.x),opacity);
}
