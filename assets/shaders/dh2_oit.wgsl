#define_import_path dh2_oit
// renderprog/oit's writePixel for the weighted-blended alpha lists
// (14031dce0): NEW_writePixel for r_OIT 1 (RETAIL), OLD_writePixel for
// r_OIT 2. The colour is in the game's lighting units; weights are
// OIT/Weights and parameters OIT/Params as 325130 binds them per surface.
struct OitPixel {
    colour:vec4<f32>,
    opacity:vec4<f32>,
}
fn oit_write_pixel(colour:vec3<f32>,albedo:vec3<f32>,alpha:f32,position:vec4<f32>,
    weights:vec4<f32>,parameters:vec4<f32>,mode:u32)->OitPixel {
    if mode==2u {
        let lum=mix(1.0,clamp(colour.g,0.0,1.0),parameters.y);
        let alpha_lum=alpha*lum;
        var weight=pow(1e-2+alpha_lum,weights.x)*clamp(parameters.z/
            (1e-5+pow(position.w/weights.y,weights.z)),1e-2,3e3);
        weight*=weights.w;
        return OitPixel(min(vec4(6000.0),vec4(colour,parameters.w)*(weight*alpha_lum)),
            vec4(alpha_lum*parameters.w,0.0,0.0,0.0));
    }
    let albedo_luminance=dot(albedo,vec3(0.2126,0.7152,0.0722));
    var additive_luminance=1.0;
    if parameters.w<1.0 && albedo_luminance<alpha {
        additive_luminance=smoothstep(0.0,parameters.w,albedo_luminance);
    }
    let opacity=parameters.w*additive_luminance;
    let weighted=vec4(colour,alpha);
    let colour_length=length(weighted);
    var colour_normal=1.0;
    if colour_length>0.0 {colour_normal=1.0-dot(weighted/colour_length,vec4(0.25));}
    colour_normal*=mix(1.0,parameters.z,alpha);
    colour_normal*=mix(1.0,alpha,parameters.y);
    let a=pow(1e-2+colour_normal,weights.x);
    let b=pow(min(0.02245,position.z)*weights.y,weights.z);
    let weight=clamp(a*b,1e-2,3e3)*weights.w;
    return OitPixel(vec4(colour,opacity)*(weight*alpha),vec4(alpha*opacity,0.0,0.0,0.0));
}
