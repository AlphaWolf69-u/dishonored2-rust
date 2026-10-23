// Camera-combine optics from the installed DOF include. Texture producers and
// view/exposure state are supplied by the owner; this code has no history.
struct CameraCombineParameters {
    vignette:vec4<f32>,
    scattering_white:vec4<f32>,
    far_range:vec4<f32>,
    distortion:vec4<f32>,
    centre_resolution:vec4<f32>,
    inverse_z_time:vec4<f32>,
    noise:vec4<f32>,
    units:vec4<f32>,
    // Six option bits, compressed HDR, host-source conversion, exposure entry.
    options:vec4<u32>,
}
fn camera_distort(uv:vec2<f32>,centre:vec2<f32>,k:f32,cubic:f32)->vec2<f32> {
    let delta=uv-centre;let r2=dot(delta,delta);
    var factor=1.0+r2*k;
    if cubic!=0.0 {factor=1.0+r2*(k+cubic*sqrt(r2));}
    return factor*(uv-vec2(0.5))+vec2(0.5);
}
fn camera_vignette(uv:vec2<f32>,centre:vec2<f32>,p:vec4<f32>)->f32 {
    let offset=p.w/p.z;
    let start=offset+p.x;let end=offset+p.y;
    return clamp((distance(uv,centre)-start)/(end-start),0.0,1.0);
}
fn camera_fuzziness(depth:f32,p:CameraCombineParameters,focus_distance:f32)->f32 {
    var fuzzy=p.far_range.x;var sharp=p.far_range.y;var far=p.far_range.w;
    if (p.options.x&16u)!=0u {
        far+=focus_distance;sharp=(sharp+focus_distance)/far;fuzzy=(fuzzy+focus_distance)/far;
    }
    let linear=-p.inverse_z_time.z/(p.inverse_z_time.x*depth+p.inverse_z_time.y);
    let normalized=clamp(linear/far,0.0,1.0);
    let k=(normalized-sharp)/(fuzzy-sharp);
    return clamp(select(0.0,1.0,normalized>sharp)*k+p.far_range.z,0.0,1.0);
}
fn camera_sharpen(colour:vec3<f32>,low:vec3<f32>,expanded:vec3<f32>,threshold:vec3<f32>,strength:f32)->vec3<f32> {
    let edge=clamp(step(low,colour)-0.75*step(threshold,expanded),vec3(0.0),vec3(1.0));
    let grey=dot(edge,vec3(0.2989,0.5870,0.1140));
    return (colour-low)*(1.0+strength*grey)+low;
}
fn camera_random(uv:vec2<f32>)->f32 {
    return fract(sin(dot(uv,vec2(12.9898,78.233)))*43758.5453);
}
fn camera_noise(colour:vec3<f32>,uv:vec2<f32>,p:vec4<f32>,grey:f32,time:f32)->vec3<f32> {
    let random=mix(camera_random(uv),camera_random(uv*time),p.w);
    let luminance=dot(colour,vec3(0.2989,0.5870,0.1140))*p.x;
    let ramp=clamp((luminance-0.2)/(0.0-0.2),0.0,1.0);
    let lum=ramp*ramp*(3.0-2.0*ramp);
    let curve=lum*grey;let square=curve*curve;
    var noise1=vec3(random*0.125)*(1.0-square*square)*luminance;
    noise1=colour+noise1*colour*(p.y*0.5);
    let original_grey=dot(colour,vec3(0.3333));
    let mapped=original_grey/(original_grey+1.0);
    let focus_amount=pow(grey,p.x);
    let noise2=mix(colour,vec3(random)*colour,p.y*focus_amount*clamp(1.0-mapped*1.8,0.0,1.0));
    return mix(noise1,noise2,p.z);
}
