// PostFX_DOF_CombineAlpha, submitted by 313730.
@group(0) @binding(0) var<uniform> position_to_texture:vec4<f32>;
@group(0) @binding(1) var refraction:texture_2d<f32>;
@group(0) @binding(2) var blurred:texture_2d<f32>;
@group(0) @binding(3) var near_alpha:texture_2d<f32>;
@group(0) @binding(4) var bilinear_clamp:sampler;
struct AlphaVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
}
@vertex fn alpha_vertex(@location(0) position:vec3<f32>,@location(1) uv:vec2<f32>)->AlphaVertex {
    return AlphaVertex(vec4(position,1.0),uv);
}
@fragment fn alpha_fragment(in:AlphaVertex)->@location(0) vec4<f32> {
    let distortion=dof_glass(textureSampleLevel(refraction,bilinear_clamp,in.uv,0.0).y);
    let do_blur=select(0.0,1.0,distortion.y>0.0);
    var offset=vec2(0.0);
    if distortion.y==1.0 {
        // GetRefractionOffset() is the installed constant 10, doubled here.
        offset=vec2(cos(distortion.x),sin(distortion.x))*20.0*position_to_texture.zw;
    }
    let alpha=clamp(textureSampleLevel(near_alpha,bilinear_clamp,in.uv+offset,0.0).x+do_blur,0.0,1.0);
    if alpha<1.0/255.0 {discard;}
    let colour=textureSampleLevel(blurred,bilinear_clamp,in.uv+offset,0.0).rgb;
    return vec4(colour,alpha);
}
