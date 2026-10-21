// PostFX/ToneMapping/common and the standard final colour transform.
// Bloom/lens radiance is supplied before this transform. The display owner
// supplies the actual exposure and a prepared linear RGBA8 colour cube.
struct ToneMappingParameters {
    junctions:vec4<f32>,
    toe:vec4<f32>,
    shoulder:vec4<f32>,
    display:vec4<f32>, // gamma, brightness, exposure, unused
    options:vec4<u32>, // tone curve, colour cube, unused, unused
}
fn tone_curve_component(p:ToneMappingParameters,x:f32)->f32 {
    let c=select(p.shoulder,p.toe,x<p.junctions.x);
    let fraction=c.xy*x+c.zw;
    return fraction.x/fraction.y;
}
fn tone_curve_colour(p:ToneMappingParameters,colour:vec3<f32>)->vec3<f32> {
    return vec3(tone_curve_component(p,colour.r),tone_curve_component(p,colour.g),tone_curve_component(p,colour.b));
}
fn tone_grade_colour(colour:vec3<f32>,cube:texture_3d<f32>,linear_clamp:sampler)->vec3<f32> {
    return textureSampleLevel(cube,linear_clamp,(colour*31.0+vec3(0.5))*(1.0/32.0),0.0).rgb;
}
fn tone_display_colour(p:ToneMappingParameters,colour:vec3<f32>)->vec3<f32> {
    let value=colour*p.display.y;
    let clipped=select(vec3(0.0),min(value,vec3(1.0)),value>vec3(0.0));
    return pow(clipped,vec3(p.display.x));
}
fn standard_display_colour(p:ToneMappingParameters,radiance:vec3<f32>,cube:texture_3d<f32>,linear_clamp:sampler)->vec3<f32> {
    var colour=max(vec3(0.0),radiance)*p.display.z;
    if p.options.x!=0u {colour=tone_curve_colour(p,colour);}
    if p.options.y!=0u {colour=tone_grade_colour(colour,cube,linear_clamp);}
    return tone_display_colour(p,colour);
}
