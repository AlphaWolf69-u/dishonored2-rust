#import bevy_pbr::mesh_view_bindings::view
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> position_size:vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var<uniform> color:vec4<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var image:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var image_sampler:sampler;
struct Sprite { @builtin(position) position:vec4<f32>, @location(0) uv:vec2<f32> }
@vertex fn vertex(@location(0) position:vec3<f32>,@location(2) uv:vec2<f32>)->Sprite {
    let center=view.clip_from_world*vec4(position_size.xyz,1.0);
    let aspect=view.viewport.z/view.viewport.w;
    var out:Sprite;
    out.position=vec4(center.xy/center.w+position.xy*position_size.w*vec2(1.0,aspect),0.0,1.0);
    if center.z/center.w<=0.0 || center.w<=0.0 {out.position=vec4(2.0,2.0,0.0,1.0);}
    out.uv=uv;
    return out;
}
@fragment fn fragment(input:Sprite)->@location(0) vec4<f32> {
    let sampled=textureSampleLevel(image,image_sampler,input.uv,0.0)*color;
    return vec4(sampled.rgb*139.26*view.exposure,sampled.a);
}
