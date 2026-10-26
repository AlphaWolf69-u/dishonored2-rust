// particles::ComputeIBL calls IBL::LQ_ComputeIBL, not the HQ Ward integral.
// Shared reflection_direction/radiance and ward_fresnel retain their scene units.
fn particle_lq_reflection(position:vec3<f32>,normal:vec3<f32>,view_direction:vec3<f32>,
    roughness:f32,ior:vec3<f32>,fresnel_strength:f32,cube:f32,gain:f32)->vec3<f32> {
    let light=normalize(mix(reflect(-view_direction,normal),normal,roughness));
    let ln=clamp(dot(light,normal),0.0,1.0);
    let vn=clamp(dot(view_direction,normal),0.0,1.0);
    let aperture=0.0003474660443456835+roughness*(1.3331290497744692-roughness*0.5040552688878546);
    let mip=6.0*sqrt(aperture/0.834);
    let count=reflection_planes[0];
    let index=select(count-1u,u32(max(0.0,cube)),cube>=0.0 && cube<f32(count));
    let luminance_ior=dot(vec3(0.2126,0.7152,0.0722),ior);
    let ratio=(luminance_ior-1.0)/(luminance_ior+1.0);
    let bounce=mix(gain,1.0,ratio*ratio);
    return ward_fresnel(ior,vn,fresnel_strength)*
        (reflection_radiance(position,light,index,mip)*sqrt(ln)*bounce);
}
