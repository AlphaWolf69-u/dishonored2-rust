#define_import_path dh2_ward
#import dh2_fresnel::ward_fresnel

// Pure standard BRDF kernels from ward, orenNayar, fresnel, colorimetry and
// BRDF_Ward. Light integration supplies the authored surface and light inputs.
// BRDF_vegetation's translucency inputs; enabled 0 for every other surface.
struct WardTranslucency {
    vertex_normal:vec3<f32>, weight:f32,
    transmittance:vec3<f32>, enabled:f32,
    // translucency/intensity, distortion, power, min
    parameters:vec4<f32>,
    // vegetation/transmittanceParms.z: smoothstep(-z, 0.1, -L.N_vertex)
    onset:f32,
}
struct WardSurface {
    normal:vec3<f32>, roughness:f32,
    view:vec3<f32>, fresnel_strength:f32,
    diffuse:vec3<f32>, sheen_importance:f32,
    ior:vec3<f32>, inverse_roughness_squared:f32,
    sheen_color:vec3<f32>,
    translucency:WardTranslucency,
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
    return WardSurface(normal,roughness,view,fresnel_strength,diffuse,sheen_importance,ior,inverse*inverse,ward_sheen_color(diffuse,sheen),
        WardTranslucency(vec3(0.0),0.0,vec3(0.0),0.0,vec4(0.0),0.0));
}

// colorimetry RGB2HSV: hue in [0,1[, value = max, saturation = delta/max.
fn rgb_to_hsv(rgb:vec3<f32>)->vec3<f32> {
    let maximum=max(max(rgb.x,rgb.y),rgb.z);
    if maximum<=0.0001 {return vec3(0.0);}
    let minimum=min(min(rgb.x,rgb.y),rgb.z);
    let delta=maximum-minimum;
    let inverse=1.0/max(1e-4,delta);
    let d=(0.5*delta+(1.0/6.0)*(vec3(maximum)-rgb))*inverse;
    let not_red=clamp(1e3*abs(rgb.x-maximum),0.0,1.0);
    let not_green=clamp(1e3*abs(rgb.y-maximum),0.0,1.0);
    let hue=mix(d.z-d.y,mix((1.0/3.0)+d.x-d.z,(2.0/3.0)+d.y-d.x,not_green),not_red);
    return vec3(fract(hue),delta/maximum,maximum);
}
// colorimetry HSV2RGB: V(1-S) + VS * RGB(hue).
fn hsv_to_rgb(hsv:vec3<f32>)->vec3<f32> {
    let rgb=clamp(3.0*abs(2.0*fract(vec3(hsv.x,hsv.x+(2.0/3.0),hsv.x+(1.0/3.0)))-1.0)-1.0,vec3(0.0),vec3(1.0));
    return hsv.z*(1.0-hsv.y)+(hsv.z*hsv.y)*rgb;
}

// BRDF_vegetation::ComputeTranslucencyNoRamp: a bent-normal translucency lobe.
fn ward_translucency(t:WardTranslucency,weight:f32,light:vec3<f32>,view:vec3<f32>)->f32 {
    let light_vector=light+t.parameters.y*t.vertex_normal;
    let ldotv=exp2(clamp(-dot(view,light_vector),0.0,1.0)*t.parameters.z-t.parameters.z);
    return t.parameters.x*(t.parameters.w+weight*ldotv);
}

// The per-light translucency weight: smoothstep(-transmittanceParms.z, 0.1,
// -dot(L, vertexNormal)) * surf.translucency.
fn ward_translucency_factor(t:WardTranslucency,light:vec3<f32>)->f32 {
    return smoothstep(-t.onset,0.1,-dot(light,t.vertex_normal))*t.weight;
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
    if s.translucency.enabled!=0.0 {
        // BRDF_vegetation::ComputeLighting: transmitted reflectance through the leaf.
        let t=s.translucency;
        let ldotn=-dot(l.direction,t.vertex_normal);
        let fresnel=ward_fresnel(s.ior,clamp(ldotn,0.0,1.0),s.fresnel_strength);
        let rho=vec3(1.0)-(vec3(1.0)-fresnel)*diffuse_color;
        let amount=ward_translucency(t,ward_translucency_factor(t,l.direction),l.direction,s.view);
        result.diffuse+=(1.0/3.14159265358979323846)*rho*t.transmittance*irradiance*amount;
    }
    return result;
}
fn ward_direct(s:WardSurface,l:WardLight)->DirectLighting {
    let term=(1.0/3.14159265358979323846)*ward_oren_nayar(s.normal,l.direction,s.view,s.roughness);
    return ward_direct_term(s,l,vec3(term));
}
fn ward_direct_lookup(s:WardSurface,l:WardLight,lookup:vec3<f32>)->DirectLighting {
    return ward_direct_term(s,l,(1.0/3.14159265358979323846)*lookup);
}

