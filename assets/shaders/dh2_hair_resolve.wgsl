// PostFX_CompositeWeightedBlendedOIT's hair term, resolved before the
// particle (alpha) list's resolve. With blend ONE SRC_ALPHA both resolves
// compose to the original's single composite:
//   hair * alphaOpacity * (1 - hairOpacity) + alpha * (1 - alphaOpacity)
//   + dst * alphaOpacity * hairOpacity.
// Accumulation is in the view target's units (the OIT weight is evaluated
// in game units by the hair program), so no exposure conversion is needed.
@group(0) @binding(0) var hair_accumulation:texture_2d<f32>;
@group(0) @binding(1) var hair_revealage:texture_2d<f32>;
@vertex fn resolve_vertex(@builtin(vertex_index) i:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((i<<1u)&2u),f32(i&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn resolve_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let position=vec2<i32>(pixel.xy);
    let opacity=textureLoad(hair_revealage,position,0).x;
    let accumulated=textureLoad(hair_accumulation,position,0);
    var colour=accumulated.rgb;
    if accumulated.a>0.0 {colour*=1.0/accumulated.a;}
    return vec4(max(colour*(1.0-opacity),vec3(0.0)),opacity);
}
