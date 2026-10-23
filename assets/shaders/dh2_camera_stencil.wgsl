// Camera blur stencil classifiers; caller supplies the actual fullscreen mesh.
struct CameraStencilParameters {projection:mat4x4<f32>,clip:vec4<f32>,}
@group(0) @binding(0) var<uniform> camera_stencil:CameraStencilParameters;
@group(0) @binding(1) var stencil_mask:texture_2d<f32>;
@group(0) @binding(2) var stencil_linear:sampler;
@group(0) @binding(3) var stencil_focus:texture_2d<f32>;
struct StencilVertex {@builtin(position)position:vec4<f32>,@location(0)uv:vec2<f32>,}
@vertex fn stencil_near_vertex(@location(0)p:vec3<f32>,@location(1)uv:vec2<f32>)->StencilVertex {
    return StencilVertex(vec4(p,1.0),uv);
}
fn stencil_projected_depth(focus:f32)->f32 {
    let distance=max(camera_stencil.clip.x,focus+camera_stencil.clip.y-0.01);
    let clip=camera_stencil.projection*vec4(0.0,0.0,-distance,1.0);
    return clip.z/clip.w;
}
@vertex fn stencil_far_vertex(@location(0)p:vec3<f32>,@location(1)uv:vec2<f32>)->StencilVertex {
    return StencilVertex(vec4(p.xy,stencil_projected_depth(0.0),1.0),uv);
}
@vertex fn stencil_autofocus_vertex(@location(0)p:vec3<f32>,@location(1)uv:vec2<f32>)->StencilVertex {
    return StencilVertex(vec4(p.xy,stencil_projected_depth(textureLoad(stencil_focus,vec2(0),0).x),1.0),uv);
}
@vertex fn stencil_constant_vertex(@location(0)p:vec3<f32>,@location(1)uv:vec2<f32>)->StencilVertex {
    return StencilVertex(vec4(p.xy,camera_stencil.clip.y,1.0),uv);
}
@fragment fn stencil_near_fragment(input:StencilVertex) {
    if textureSampleLevel(stencil_mask,stencil_linear,input.uv,0.0).r<=0.05 {discard;}
}
@fragment fn stencil_far_fragment(input:StencilVertex) {
    if point_clamp_sample(stencil_mask,input.uv).g==0.0 {discard;}
}
@fragment fn stencil_constant_fragment() {}
