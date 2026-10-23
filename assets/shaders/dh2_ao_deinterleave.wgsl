@group(1) @binding(0) var ao_linear:texture_2d<f32>;
struct AoDepthSlices {
    @location(0) z0:f32,@location(1) z1:f32,@location(2) z2:f32,@location(3) z3:f32,
    @location(4) z4:f32,@location(5) z5:f32,@location(6) z6:f32,@location(7) z7:f32,
}
fn ao_load_depth(p:vec2<i32>)->f32 {
    return textureLoad(ao_linear,clamp(p,vec2(0),vec2<i32>(textureDimensions(ao_linear))-vec2(1)),0).r;
}
fn ao_deinterleave(position:vec2<f32>,row:i32)->AoDepthSlices {
    let base=vec2<i32>(position)*4+vec2(0,row);var out:AoDepthSlices;
    out.z0=ao_load_depth(base);out.z1=ao_load_depth(base+vec2(1,0));
    out.z2=ao_load_depth(base+vec2(2,0));out.z3=ao_load_depth(base+vec2(3,0));
    out.z4=ao_load_depth(base+vec2(0,1));out.z5=ao_load_depth(base+vec2(1,1));
    out.z6=ao_load_depth(base+vec2(2,1));out.z7=ao_load_depth(base+vec2(3,1));return out;
}
@fragment fn ao_deinterleave_top(input:AoVertex)->AoDepthSlices {return ao_deinterleave(input.position.xy,0);}
@fragment fn ao_deinterleave_bottom(input:AoVertex)->AoDepthSlices {return ao_deinterleave(input.position.xy,2);}
