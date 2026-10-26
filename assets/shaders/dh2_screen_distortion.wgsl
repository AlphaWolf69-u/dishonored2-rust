// PostFX/screenDisto, screenProgs::ComputeRatioAdaptedTexcoord and
// particles::PanAndReadDistUV. Each material adds to the shared vector field.
struct DistortionLayer {
    tiling_speed:vec4<f32>,
    strength:vec4<f32>,
    rotation_seed:vec4<f32>,
}
struct ScreenDistortionParameters {
    layers:array<DistortionLayer,3>,
    options:vec4<u32>,
    viewport_time_fade:vec4<f32>,
    clamp_colour:vec4<f32>,
    depth:vec4<f32>,
    unproject_alpha:vec4<f32>,
}
@group(0) @binding(0) var<uniform> p:ScreenDistortionParameters;
@group(0) @binding(1) var distortion_one:texture_2d<f32>;
@group(0) @binding(2) var distortion_two:texture_2d<f32>;
@group(0) @binding(3) var opacity:texture_2d<f32>;
@group(0) @binding(4) var trilinear_wrap:sampler;
@group(0) @binding(5) var scene_depth:texture_depth_2d;
@group(0) @binding(6) var linear_clamp:sampler;

@vertex fn screen_distortion_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
fn screen_ratio_uv(uv:vec2<f32>)->vec2<f32> {
    let position=uv*vec2(2.0,-2.0)+vec2(-1.0,1.0);
    let ratio=p.viewport_time_fade.x/p.viewport_time_fade.y;
    let invratio=p.viewport_time_fade.y/p.viewport_time_fade.x;
    var coord=position*vec2(1.0,-1.0);
    switch p.options.y {
        case 1u: {coord=vec2(position.x*ratio*(9.0/16.0),-position.y);}
        case 2u: {coord=vec2(position.x,-position.y*invratio*(16.0/9.0));}
        case 3u: {
            if ratio>16.0/9.0 {coord=vec2(position.x*ratio*(9.0/16.0),-position.y);}
            else {coord=vec2(position.x,-position.y*invratio*(16.0/9.0));}
        }
        case 4u: {
            if ratio>16.0/9.0 {coord=vec2(position.x,-position.y*invratio*(16.0/9.0));}
            else {coord=vec2(position.x*ratio*(9.0/16.0),-position.y);}
        }
        case 5u: {
            if ratio<1.0 {coord=vec2(-position.y*invratio*(9.0/16.0),position.x);}
            else {coord=vec2(position.x*ratio*(9.0/16.0),-position.y);}
        }
        case 6u: {
            if ratio<1.0 {coord=vec2(-position.y,position.x*ratio*(16.0/9.0));}
            else {coord=vec2(position.x,-position.y*invratio*(16.0/9.0));}
        }
        default: {}
    }
    return 0.5*coord+vec2(0.5);
}
fn screen_pan(uv:vec2<f32>,layer:DistortionLayer)->vec2<f32> {
    // Screen geometry sets texcoords.zw to zero, so material_seed contributes
    // zero to local_time here, exactly as in PanAndReadDistUV.
    let time=p.viewport_time_fade.z;
    let angle=time*layer.rotation_seed.x;
    let c=cos(angle); let s=sin(angle);
    let rotated=vec2((uv.x-0.5)*c+(uv.y-0.5)*s+0.5,
        -(uv.x-0.5)*s+(uv.y-0.5)*c+0.5);
    return rotated*layer.tiling_speed.xy+time*layer.tiling_speed.zw;
}
@fragment fn screen_distortion_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let screen_uv=pixel.xy/p.viewport_time_fade.xy;
    let uv=screen_ratio_uv(screen_uv);
    let one=textureSample(distortion_one,trilinear_wrap,screen_pan(uv,p.layers[0])).xy;
    var blurred=one*p.layers[0].strength.zw;
    var raw=one*p.layers[0].strength.xy;
    if p.options.x!=0u {
        let two=textureSample(distortion_two,trilinear_wrap,screen_pan(uv,p.layers[1])).xy;
        blurred=(blurred+two*p.layers[1].strength.zw)/2.0;
        raw=(raw+two*p.layers[1].strength.xy)/2.0;
    }
    var attenuation=textureSample(opacity,trilinear_wrap,screen_pan(uv,p.layers[2])).x;
    if p.options.w!=0u {
        let z=textureSampleLevel(scene_depth,linear_clamp,screen_uv,0);
        let linear=-p.unproject_alpha.z/(p.unproject_alpha.x*z+p.unproject_alpha.y);
        let amount=clamp((linear-p.depth.x)/(p.depth.y-p.depth.x),0.0,1.0);
        attenuation*=mix(p.depth.z,p.depth.w,amount);
    }
    if attenuation<max(p.unproject_alpha.w,0.0001) {discard;}
    let output=vec4(blurred,raw);
    if p.options.z!=0u && (any(uv>vec2(1.0))||any(uv<vec2(0.0))) {
        return attenuation*p.clamp_colour*p.viewport_time_fade.w;
    }
    return attenuation*output*p.viewport_time_fade.w;
}
