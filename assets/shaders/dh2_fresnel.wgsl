#define_import_path dh2_fresnel

fn ward_fresnel(ior:vec3<f32>,cosine:f32,strength:f32)->vec3<f32> {
    let c=mix(1.0,cosine,strength);
    let g=sqrt(max(vec3(0.0001),ior*ior-vec3(1.0)+vec3(c*c)));
    let ratio=(g-vec3(c))/(g+vec3(c));let a=ratio*ratio;
    let b=(c*(g+vec3(c))-vec3(1.0))/(c*(g-vec3(c))+vec3(1.0));
    return 0.5*a*(vec3(1.0)+b*b);
}
