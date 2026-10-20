#define_import_path dh2_filling
// Installed PanAndReadDistUV and filing_container colour equations.
fn filling_pan_uv(uv:vec2<f32>,time:f32,rotation:f32,pan:vec2<f32>,tiling:vec2<f32>)->vec2<f32> {
    let c=cos(time*rotation);let s=sin(time*rotation);let q=uv-vec2(0.5);
    return (vec2(q.x*c+q.y*s,-q.x*s+q.y*c)+vec2(0.5))*tiling+time*pan;
}
fn filling_step(a:f32,b:f32,x:f32)->f32 {
    let v=(x-a)/(b-a);var t=0.0;if v>0.0 {t=min(v,1.0);}return t*t*(3.0-2.0*t);
}
// ViewDependentFade uses DirectX's pixel SV_Position.w (clip W).
// Installed pixel stage 177 retains v5.w in the reciprocal-interval multiply.
fn filling_view_fade(clip_w:f32,start:f32,end:f32)->f32 {
    let v=(clip_w-start)*(1.0/(end-start));
    var t=0.0;if v>0.0 {t=min(v,1.0);}
    return t*t*(3.0-2.0*t);
}
fn filling_lookup_coordinate(y:f32,distortion:f32,control:f32,p:vec4<f32>)->f32 {
    let bottom=filling_step(0.0,p.y,pow(y,8.0));
    let top=filling_step(0.0,p.y,pow(1.0-y+control,8.0));
    return filling_step(p.x,p.x+p.z,distortion+bottom+top);
}
fn filling_transfer_color(base:vec4<f32>,lookup:vec4<f32>,amount:f32,y:f32,control:f32,
    lerp_colour:f32,liquid:vec4<f32>,adjust:vec4<f32>,legacy:f32)->vec4<f32> {
    let color=mix(base,lookup,amount)*mix(vec4(1.0),liquid,lerp_colour);
    return vec4(color.rgb*legacy*adjust.rgb*adjust.a,step(control,y)*legacy);
}
