// PostFX/ToneMapping::Rotate2dScaled, LensStreak and helper::rand(float).
fn lens_random(n:f32)->f32 {return fract(sin(n)*43758.5453123);}
fn lens_rotate_scaled(p:vec2<f32>,angle:f32)->vec2<f32> {
    let s=sin(angle);let c=cos(angle);
    return vec2(c*p.x-s*0.5*p.y,s*0.5*p.x+c*p.y);
}
// Installed-HLSL DXBC atan2 expansion and phase MAD. The angular noise makes
// ordinary atan2 approximation/rounding differences visible after 4096 slices.
fn lens_slice_phase(p:vec2<f32>)->vec2<f32> {
    var angle=shader_atan2(p.x,p.y);
    angle+=bitcast<f32>(0x40490fdbu);
    let slice=floor(angle*bitcast<f32>(0x4422f983u));
    return vec2(slice,fma(angle,bitcast<f32>(0x4422f983u),-slice));
}
// camera_axis is the source viewMatrix[0].z + viewMatrix[2].y.
// settings: radius, random radius offset, width, opacity.
fn lens_streak(uv:vec2<f32>,camera_axis:f32,settings:vec4<f32>)->f32 {
    let p=lens_rotate_scaled(uv-vec2(0.5),camera_axis);
    let radius_squared=dot(p,p);
    let phase=lens_slice_phase(p);
    let first=lens_random(phase.x);
    let random=fma(phase.y,lens_random(phase.x+1.0)-first,first);
    let radius=fma(random,settings.y,settings.x);
    let distance=abs(fma(-radius,radius,radius_squared));
    // The source deliberately reverses the smoothstep edges.
    let t=clamp((distance-settings.z)*(1.0/(-settings.z)),0.0,1.0);
    return t*t*fma(-2.0,t,3.0)*settings.w;
}
