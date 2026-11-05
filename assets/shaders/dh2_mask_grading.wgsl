// postfx/colorgrading: the invert option deliberately selects the fetched mask.
@group(0) @binding(0) var<uniform> mask_grade:vec4<f32>;
@group(0) @binding(1) var mask_grade_hdr:texture_2d<f32>;
@group(0) @binding(2) var mask_grade_mask:texture_2d<f32>;
@group(0) @binding(3) var mask_grade_linear:sampler;
@group(0) @binding(4) var<storage,read> mask_grade_adaptation:array<vec4<f32>,2>;
@vertex fn mask_grade_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn mask_grade_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let uv=pixel.xy/vec2<f32>(textureDimensions(mask_grade_hdr));
    let original=textureSampleLevel(mask_grade_hdr,mask_grade_linear,uv,0.0);
    let luminance=mask_grade_adaptation[0].x;
    let colour=hdr_range_expand(original.rgb,mask_grade.z!=0.0,luminance,mask_grade.y);
    let fetched=textureSampleLevel(mask_grade_mask,mask_grade_linear,uv,0.0).r;
    let mask=select(1.0-fetched,fetched,mask_grade.w!=0.0);
    let grey=vec3(max(colour.r,max(colour.g,colour.b)));
    let graded=mix(colour,grey,mask*mask_grade.x);
    return vec4(hdr_range_compress(graded,mask_grade.z!=0.0,luminance,mask_grade.y),original.a);
}
