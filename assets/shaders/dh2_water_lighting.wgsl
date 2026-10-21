// Common water scattering and dielectric Fresnel from the installed shaders.
// Reflection/refraction resource ownership and WaterShading are separate.
fn water_inscattering(camera_height:f32,point_height:f32,distance:f32,extinction:vec3<f32>)->vec3<f32> {
    let phase=1.0/(4.0*3.1415926535897932384626433832795);
    let safe_distance=distance+0.0001;
    let g=extinction*(1.0+(point_height-camera_height)/safe_distance);
    return phase*exp(-extinction*camera_height)*(vec3(1.0)-exp(-g*distance))/max(vec3(0.001),g);
}
struct WaterFresnel {value:f32,total_internal_reflection:bool,}
fn water_fresnel(ior:f32,cosine:f32)->WaterFresnel {
    let g_squared=1.0-ior*ior*(1.0-cosine*cosine);
    if g_squared<0.0 {return WaterFresnel(1.0,true);}
    let g=sqrt(g_squared);
    let ior_cosine=ior*cosine;
    let ior_g=ior*g;
    let rs=(ior_cosine-g)/(ior_cosine+g);
    let rp=(ior_g-cosine)/(ior_g+cosine);
    return WaterFresnel((rs*rs+rp*rp)*0.5,false);
}
