#define_import_path dh2_ward
#import dh2_fresnel::ward_fresnel

// Pure standard BRDF kernels from ward, orenNayar, fresnel, colorimetry and
// BRDF_Ward. Light integration supplies the authored surface and light inputs.
struct WardSurface {
    normal:vec3<f32>, roughness:f32,
    view:vec3<f32>, fresnel_strength:f32,
    diffuse:vec3<f32>, sheen_importance:f32,
    ior:vec3<f32>, inverse_roughness_squared:f32,
    sheen_color:vec3<f32>,
}
struct WardLight {
    direction:vec3<f32>, attenuation:f32,
    color:vec3<f32>, shadow:f32,
    specular_attenuation:f32,
}
struct DirectLighting { diffuse:vec3<f32>, specular:vec3<f32>, }
fn ward_sheen_color(color:vec3<f32>,sheen:f32)->vec3<f32> {
    let desaturation=clamp(sheen+1.0,0.0,1.0)*0.5+0.5;
    let amount=clamp(sheen,0.0,1.0);
    let unsaturated=mix(vec3(1.0-desaturation),color,desaturation);
    let saturation=clamp(color*(1.0+amount*2.0)-(color.gbr+color.brg)*amount,vec3(0.0),vec3(1.0));
    let saturated=clamp((1.0+amount)*saturation,vec3(0.0),vec3(1.0));
    return mix(unsaturated,saturated,sheen*0.5+0.5);
}
fn ward_surface(normal:vec3<f32>,view:vec3<f32>,roughness:f32,diffuse:vec3<f32>,ior:vec3<f32>,
    sheen:f32,sheen_importance:f32,fresnel_strength:f32)->WardSurface {
    let inverse=1.0/(1e-5+max(0.01,roughness*roughness));
    return WardSurface(normal,roughness,view,fresnel_strength,diffuse,sheen_importance,ior,inverse*inverse,ward_sheen_color(diffuse,sheen));
}
fn ward_oren_nayar(n:vec3<f32>,l:vec3<f32>,v:vec3<f32>,roughness:f32)->f32 {
    let gamma=dot(v-n*dot(v,n),l-n*dot(l,n));
    let r2=roughness*roughness;
    let a=1.0-0.5*(r2/(r2+0.57));let b=0.45*(r2/(r2+0.09));
    let vn=dot(v,n);let ln=dot(l,n);let alpha=min(vn,ln);let beta=max(vn,ln);
    let sin_alpha=clamp(1.0-alpha*alpha,0.0,1.0);let sin_beta=clamp(1.0-beta*beta,0.0,1.0);
    let c=sqrt(sin_alpha*sin_beta)/(1e-6+beta);
    return clamp(dot(n,l),0.0,1.0)*(a+b*max(0.0,gamma)*c);
}
fn ward_distribution(s:WardSurface,l:vec3<f32>)->f32 {
    // Ward's half vector is intentionally unnormalized.
    let h=l+s.view;let hn=max(1e-4,dot(h,s.normal));
    let inverse_n2=1.0/(hn*hn);let inverse_n4=inverse_n2*inverse_n2;
    let hh=dot(h,h);let exponent=s.inverse_roughness_squared*(1.0-inverse_n2*hh);
    return (1.0/3.14159265358979323846)*s.inverse_roughness_squared*exp(exponent)*hh*inverse_n4;
}

fn ward_direct_term(s:WardSurface,l:WardLight,diffuse_term:vec3<f32>)->DirectLighting {
    var result=DirectLighting(vec3(0.0),vec3(0.0));
    if l.attenuation<=0.0 || l.shadow<=0.0 {return result;}
    let h=normalize(s.view+l.direction);let hn=clamp(dot(h,s.normal),0.0,1.0);
    let ln=clamp(dot(l.direction,s.normal),0.0,1.0);
    let irradiance=l.shadow*(l.attenuation*l.color);
    let specular=ward_fresnel(s.ior,hn,s.fresnel_strength)*l.specular_attenuation;
    let m=clamp(1.0-hn,0.0,1.0);let m2=m*m;let schlick=m2*m2*m;
    let diffuse_color=mix(s.diffuse,s.sheen_color,s.roughness*pow(schlick,s.sheen_importance));
    result.diffuse=(vec3(1.0)-specular)*irradiance*(diffuse_color*diffuse_term);
    result.specular=specular*irradiance*(ward_distribution(s,l.direction)*ln);
    return result;
}
fn ward_direct(s:WardSurface,l:WardLight)->DirectLighting {
    let term=(1.0/3.14159265358979323846)*ward_oren_nayar(s.normal,l.direction,s.view,s.roughness);
    return ward_direct_term(s,l,vec3(term));
}
fn ward_direct_lookup(s:WardSurface,l:WardLight,lookup:vec3<f32>)->DirectLighting {
    return ward_direct_term(s,l,(1.0/3.14159265358979323846)*lookup);
}

