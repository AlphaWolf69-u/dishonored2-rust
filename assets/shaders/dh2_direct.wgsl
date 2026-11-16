#define_import_path dh2_direct
#import dh2_fresnel::ward_fresnel
#ifdef DH2_SKIN
#import dh2_material_samplers::linear_clamp
@group(#{MATERIAL_BIND_GROUP}) @binding(171) var skin_diffuse_lookup:texture_2d<f32>;
#endif
#ifdef DH2_HAIR
// arkHair reuses image roles 34..36: hairFlowMap (170), hairSpecularMask
// (171), hairSpecularNoiseMask (172).
@group(#{MATERIAL_BIND_GROUP}) @binding(171) var hair_specular_mask:texture_2d<f32>;
#endif

#import dh2_ward::{WardSurface,WardLight,DirectLighting,ward_sheen_color,ward_surface,ward_oren_nayar,ward_distribution,ward_direct_term,ward_direct,ward_direct_lookup}

// BRDF_skin (arkSSSRender). Diffuse albedo is 1 and sheen 0, so the sheen
// colour is white and the diffuse colour term is 1. Fresnel uses V.N, not
// H.N, and up to three Ward lobes share the normal with SSS/tweakSpecularIntensity
// weights. Diffuse leaves through `skin_light_diffuse` (the light loop
// applies the same shadow it applies to the returned specular) because
// arkSSSRender writes it to sssBuffer instead of hdrBuffer.
// `isTranslucent` only admits shadowed lights for the translucency_hq
// transmittance term (not ported); without it a zero shadow contributes zero.
struct SkinSurface {
    normal:vec3<f32>, roughness:f32,
    view:vec3<f32>, secondary:u32,
    inverse_roughness_squared:vec3<f32>, fuzz_enable:u32,
    tweak:vec3<f32>, diffuse_lookup:u32,
    ior:vec3<f32>, fuzz_intensity:f32,
    fuzz_fresnel:f32, fuzz_mask:f32,
}
var<private> skin_surface:SkinSurface;
var<private> skin_light_diffuse:vec3<f32>;
var<private> skin_direct_diffuse:vec3<f32>;
// ward::CreateWardContext.
fn ward_inverse_roughness_squared(roughness:f32)->f32 {
    let inverse=1.0/(1e-5+max(0.01,roughness*roughness));
    return inverse*inverse;
}
fn ward_term(n:vec3<f32>,l:vec3<f32>,v:vec3<f32>,inverse_roughness_squared:f32)->f32 {
    let h=l+v;let hn=max(1e-4,dot(h,n));
    let inverse_n2=1.0/(hn*hn);let inverse_n4=inverse_n2*inverse_n2;
    let hh=dot(h,h);
    return (1.0/3.14159265358979323846)*inverse_roughness_squared
        *exp(inverse_roughness_squared*(1.0-inverse_n2*hh))*hh*inverse_n4;
}
fn skin_direct(l:WardLight)->vec3<f32> {
    skin_light_diffuse=vec3(0.0);
    if l.attenuation<=0.0 || l.shadow<=0.0 {return vec3(0.0);}
    let s=skin_surface;
    let ln=clamp(dot(l.direction,s.normal),0.0,1.0);
    let vn=clamp(dot(s.view,s.normal),0.0,1.0);
    let irradiance=l.shadow*(l.attenuation*l.color);
    let specular=ward_fresnel(s.ior,vn,1.0)*l.specular_attenuation;
    let oren_nayar=ward_oren_nayar(s.normal,l.direction,s.view,s.roughness);
    var term=vec3((1.0/3.14159265358979323846)*oren_nayar);
#ifdef DH2_SKIN
    if s.diffuse_lookup!=0u {
        term=(1.0/3.14159265358979323846)*textureSampleLevel(skin_diffuse_lookup,linear_clamp,
            vec2(oren_nayar,0.5),0.0).rgb;
    }
#endif
    skin_light_diffuse=(vec3(1.0)-specular)*irradiance*term;
    var lobes=ward_term(s.normal,l.direction,s.view,s.inverse_roughness_squared.x)*s.tweak.x;
    if s.secondary!=0u {
        lobes+=ward_term(s.normal,l.direction,s.view,s.inverse_roughness_squared.y)*s.tweak.y
            +ward_term(s.normal,l.direction,s.view,s.inverse_roughness_squared.z)*s.tweak.z;
    }
    var result=specular*irradiance*(lobes*ln);
    if s.fuzz_enable!=0u {
        result+=irradiance*(pow(1.0-vn,s.fuzz_fresnel)*s.fuzz_intensity*s.fuzz_mask)*ln;
    }
    return result;
}

// BRDF_hair (arkHair). Light shadow is applied by the light loop to the
// returned sum, which equals the original's Irradiance_shadowed use in every
// term ported here. One-sided translucency (shadow-map depth through
// GetShadowMapZandPos, unshadowed irradiance) is not ported; two-sided
// translucency is.
struct HairSurface {
    normal:vec3<f32>, roughness:f32,
    view:vec3<f32>, sheen_importance:f32,
    vertex_normal:vec3<f32>, fresnel_strength:f32,
    // m_Kajiya.wsTangent: cos(angle) * tangent + sin(angle) * bitangent.
    tangent:vec3<f32>, luminance_hack:f32,
    albedo:vec3<f32>, noise_shift:f32,
    sheen_color:vec3<f32>, noise_mask:f32,
    // userData.specularMask, already INVPI-scaled.
    specular_mask:vec3<f32>, translucency_flags:u32,
    transmittance:vec3<f32>, primary_power:f32,
    // primary intensity, shift; secondary power, intensity, shift.
    lobes:vec4<f32>, secondary_shift:f32,
    // translucency/intensity, distortion, power, min.
    translucency:vec4<f32>, translucency_onset:f32,
}
var<private> hair_surface:HairSurface;
// ShiftTangent.
fn hair_shift(tangent:vec3<f32>,normal:vec3<f32>,shift:f32)->vec3<f32> {
    return normalize(tangent+shift*normal);
}
// BRDF::SpecularTerm (Kajiya-Kay).
fn hair_kajiya(tangent:vec3<f32>,light:vec3<f32>,view:vec3<f32>,exponent:f32)->f32 {
    let h=normalize(light+view);let th=dot(tangent,h);
    let sin_th=sqrt(max(0.0,1.0-th*th));
    return smoothstep(-1.0,0.0,th)*pow(sin_th,exponent);
}
// fresnel FresnelSchlick(u).
fn hair_schlick(u:f32)->f32 {
    let m=clamp(1.0-u,0.0,1.0);let m2=m*m;return m2*m2*m;
}
// fresnel FresnelDisney (MAX_DIFFUSE_ROUGHNESS 4, sheen-importance exponent).
fn hair_fresnel_disney(ln:f32,vn:f32,lh:f32,sheen:vec3<f32>,importance:f32)->vec3<f32> {
    let comp_l=1.0-clamp(ln,0.0,1.0);let comp_v=1.0-clamp(vn,0.0,1.0);
    let fd90_minus_1=vec3(0.5)+(sheen*4.0)*lh*lh-vec3(1.0);
    let exponent=exp2(mix(3.3219280948873623478703194294894,1.0,importance));
    return (vec3(1.0)+fd90_minus_1*pow(comp_l,exponent))*(vec3(1.0)+fd90_minus_1*pow(comp_v,exponent));
}
const HAIR_LUMINANCE=vec3(0.2126,0.7152,0.0722);
fn hair_direct(l:WardLight)->vec3<f32> {
    if l.attenuation<=0.0 {return vec3(0.0);}
    let s=hair_surface;
    let half_vector=normalize(s.view+l.direction);
    let ln=clamp(dot(l.direction,s.normal),0.0,1.0);
    let vn=clamp(dot(s.view,s.normal),0.0,1.0);
    let lh=dot(l.direction,half_vector);
    // Diffuse: hairLuminanceHack * sheen-mixed colour * Oren-Nayar.
    let sheen=s.roughness*pow(hair_schlick(vn),s.sheen_importance);
    let diffuse_color=mix(s.albedo,s.sheen_color,sheen);
    let diffuse_term=s.luminance_hack*diffuse_color*ward_oren_nayar(s.normal,l.direction,s.view,s.roughness);
    let irradiance=l.attenuation*l.color;
    let fresnel_specular=hair_fresnel_disney(ln,vn,lh,s.sheen_color,s.sheen_importance);
    // IOR 1.55 [Stamm et al. 1977].
    let fresnel_diffuse=fresnel_specular*(vec3(1.0)-ward_fresnel(vec3(1.55),vn,s.fresnel_strength));
    var result=fresnel_diffuse*irradiance*diffuse_term;
    // Two shifted Kajiya-Kay lobes.
    let light_luminance=dot(l.color,HAIR_LUMINANCE);
    let mask_luminance=dot(s.specular_mask,HAIR_LUMINANCE);
    let luminance_irradiance=l.attenuation*light_luminance;
    let primary_tangent=hair_shift(s.tangent,s.normal,s.noise_shift+s.lobes.y);
    let primary=hair_kajiya(primary_tangent,l.direction,s.view,s.primary_power);
    let primary_color=fresnel_specular*(mask_luminance*irradiance)*primary;
    let secondary_tangent=hair_shift(s.tangent,s.normal,s.noise_shift+s.secondary_shift);
    let secondary=hair_kajiya(secondary_tangent,l.direction,s.view,s.lobes.z);
    let secondary_color=fresnel_specular*(s.specular_mask*luminance_irradiance)*secondary*s.noise_mask;
    result+=(s.lobes.x*primary_color+s.lobes.w*secondary_color)*l.specular_attenuation;
    // Two-sided translucency (translucency/enable with twoSidedLighting).
    if (s.translucency_flags&3u)==3u {
        let ldotn=-dot(l.direction,s.vertex_normal);
        let factor=smoothstep(-s.translucency_onset,0.1,ldotn);
        let light_vector=l.direction+s.translucency.y*s.vertex_normal;
        let ldotv=exp2(clamp(-dot(s.view,light_vector),0.0,1.0)*s.translucency.z-s.translucency.z);
        result+=s.transmittance*irradiance*(s.translucency.x*(s.translucency.w+factor*ldotv));
    }
    return result;
}
