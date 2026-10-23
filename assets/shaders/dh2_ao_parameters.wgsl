// HBAO+ 3 translations, Copyright (c) 2008-2021 NVIDIA Corporation.
// See vendor/hbao/LICENSE.txt. Verified against the installed game's shaders.
struct HbaoParameters { p:array<vec4<f32>,14>, }
@group(0) @binding(0) var<uniform> ao:HbaoParameters;
struct AoVertex { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32>, }
@vertex fn ao_vertex(@builtin(vertex_index) i:u32)->AoVertex {
    let uv=vec2(f32((i<<1u)&2u),f32(i&2u));
    var out:AoVertex;out.position=vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);out.uv=uv;return out;
}
fn ao_view_position(uv:vec2<f32>,depth:f32)->vec3<f32> {
    return vec3((ao.p[2].xy*uv+ao.p[2].zw)*depth,depth);
}
