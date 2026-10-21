// Inputs are the interpolated clip-position varyings, not SV_Position pixels.
fn screen_motion_vector(current:vec4<f32>,previous:vec4<f32>,jitter:vec2<f32>)->vec4<f32> {
    let motion=(current.xy/current.w-previous.xy/previous.w-jitter)*0.5;
    return vec4(motion.x,-motion.y,1.0,0.0);
}
