// D3D11 point/MIRROR addressing: reduce tiling, snap scaled coordinates to
// 16.8 fixed point (nearest/even), then mirror the selected integer texel.
// D3D11 functional spec 7.18.6..9; source-API fixtures include odd sizes/seams.
fn point_scaled_fixed(value:f32,size:u32)->i32 {
    let bits=bitcast<u32>(value);
    let exponent=(bits>>23u)&255u;
    // Subnormals cannot reach half a fixed-point step at supported extents.
    if exponent==0u {return 0;}
    let shift=142u-exponent; // mantissa * extent * 2^(exponent-127-23+8)
    if shift>=64u {return 0;}
    let mantissa=(bits&0x7fffffu)|0x800000u;
    // Three byte products are exact for all D3D11 texture extents (<=2^17).
    // Carry into a second word before nearest/even fixed-point rounding.
    let p0=(mantissa&255u)*size;
    let p1=((mantissa>>8u)&255u)*size;
    let p2=(mantissa>>16u)*size;
    let middle=p0+(p1<<8u);
    let low=middle+(p2<<16u);
    let high=(p1>>24u)+(p2>>16u)+u32(middle<p0)+u32(low<middle);
    var whole=0u;var above=false;var equal=false;
    if shift<32u {
        whole=(low>>shift)|(high<<(32u-shift));
        let remainder=low&((1u<<shift)-1u);
        let half=1u<<(shift-1u);
        above=remainder>half;equal=remainder==half;
    } else if shift==32u {
        whole=high;above=low>0x80000000u;equal=low==0x80000000u;
    } else {
        let upper_shift=shift-32u;
        whole=high>>upper_shift;
        let remainder=high&((1u<<upper_shift)-1u);
        let half=1u<<(upper_shift-1u);
        above=(remainder>half)||((remainder==half)&&(low!=0u));
        equal=(remainder==half)&&(low==0u);
    }
    if above||(equal&&((whole&1u)!=0u)) {whole+=1u;}
    return select(i32(whole),-i32(whole),(bits&0x80000000u)!=0u);
}
fn point_mirror_texel(uv:vec2<f32>,extent:vec2<u32>)->vec2<i32> {
    let half_uv=uv*0.5;
    let reduced=(half_uv-trunc(half_uv))*2.0;
    // Keep every product bit until nearest/even 16.8 rounding. Floating MAD
    // compensation is subject to DX12 compiler reassociation at half steps.
    var texel=vec2(point_scaled_fixed(reduced.x,extent.x),point_scaled_fixed(reduced.y,extent.y))>>vec2(8);
    texel=select(texel,-texel-vec2(1),texel<vec2(0));
    let size=vec2<i32>(extent);
    let odd=((texel/size)&vec2(1))!=vec2(0);
    let wrapped=texel%size;
    return select(wrapped,size-wrapped-vec2(1),odd);
}
fn point_mirror_sample(image:texture_2d<f32>,uv:vec2<f32>)->vec4<f32> {
    return textureLoad(image,point_mirror_texel(uv,textureDimensions(image)),0);
}
fn point_clamp_texel(uv:vec2<f32>,extent:vec2<u32>)->vec2<i32> {
    let reduced=clamp(uv,vec2(0.0),vec2(1.0));
    let texel=vec2(point_scaled_fixed(reduced.x,extent.x),point_scaled_fixed(reduced.y,extent.y))>>vec2(8);
    return clamp(texel,vec2(0),vec2<i32>(extent)-vec2(1));
}
fn point_clamp_sample(image:texture_2d<f32>,uv:vec2<f32>)->vec4<f32> {
    return textureLoad(image,point_clamp_texel(uv,textureDimensions(image)),0);
}
fn point_clamp_depth(image:texture_depth_2d,uv:vec2<f32>)->f32 {
    return textureLoad(image,point_clamp_texel(uv,textureDimensions(image)),0);
}
