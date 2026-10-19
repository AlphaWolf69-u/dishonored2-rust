#define_import_path dh2_vista

// arkVista's baked-lightmap BRDF, including its distinct gloss remapping.
// Keep independent of material bindings so synthetic D3D11 oracle cases can
// exercise this production implementation directly on the GPU.
fn vista_saturate(x:f32)->f32 {
    // D3D saturate maps NaN to zero, including equal smoothstep endpoints.
    if !(x > 0.0) { return 0.0; }
    return min(x,1.0);
}
fn vista_smoothstep(a:f32,b:f32,x:f32)->f32 {
    // Equal thresholds produce NaN at the threshold in HLSL; D3D saturate
    // maps that value to zero. Avoid FXC optimizing away the NaN check.
    if a==b {return select(0.0,1.0,x>a);}
    let t=vista_saturate((x-a)/(b-a));
    return t*t*(3.0-2.0*t);
}
fn vista_lighting(albedo:vec3<f32>, baked:vec4<f32>, input_gloss:f32,
    dielectric:vec3<f32>, metal_diffuse:vec3<f32>, maximum_metal:f32,
    parameters:array<vec4<f32>,4>, V:vec3<f32>, L:vec3<f32>, sun:vec3<f32>,
    N:vec3<f32>, T:vec3<f32>, B:vec3<f32>, geometric_normal:vec3<f32>,
    emission:vec3<f32>)->vec3<f32> {
    let inv_pi=0.3183098861837907;
    let ldotn=dot(L,N);
    let p=parameters;
    let boost=vista_smoothstep(p[1].x,p[1].y,ldotn)
        *vista_smoothstep(p[1].z,p[1].w,input_gloss)
        *vista_smoothstep(p[2].x,p[2].y,baked.a);
    let H=mix(V+L,N,boost);
    let gloss=mix(input_gloss,p[2].z,boost);
    let metal=maximum_metal*vista_smoothstep(p[2].w,p[3].x,boost);
    let metallic=metal*metal;
    let mapped_gloss=mix(vista_smoothstep(0.0,1.07,gloss),
        vista_smoothstep(0.05,1.0,gloss),gloss);
    let f0=mix(dielectric,albedo,metallic)*vista_saturate((mapped_gloss+metallic)/0.05);
    let diffuse=mix(albedo,metal_diffuse,metallic);
    let root=sqrt(f0);
    let ior=(vec3(1.0)+root)/(vec3(1.00001)-root);
    let strength=vista_smoothstep(0.1,1.0,gloss);
    let c=mix(1.0,vista_saturate(dot(V,N)),strength);
    let g=sqrt(max(vec3(0.0001),ior*ior-vec3(1.0)+vec3(c*c)));
    let a=(g-vec3(c))/(g+vec3(c));
    let b=(c*(g+vec3(c))-vec3(1.0))/(c*(g-vec3(c))+vec3(1.0));
    let fresnel=0.5*a*a*(vec3(1.0)+b*b);
    let shadowed=baked.a*sun;
    let direct=(vec3(1.0)-fresnel)*shadowed*inv_pi*diffuse*vista_saturate(ldotn);
    // Game Z up is viewer Y up; all vectors have already been transformed.
    let rdotup=vista_saturate(0.5-reflect(V,geometric_normal).y*0.5);
    let vdotgeo=pow(vista_saturate(dot(V,geometric_normal)),p[3].z);
    let ambient=diffuse*baked.rgb*p[0].x+p[3].y*sun*gloss*vdotgeo*rdotup;
    let roughness=(1.0-gloss)*(1.0-gloss);
    let inverse_roughness=1.0/roughness;
    let hn=dot(H,N);
    let inv_hn2=1.0/max(0.01,hn*hn);
    let ht=dot(H,T*inverse_roughness);
    let hb=dot(H,B*inverse_roughness);
    let ward=inv_pi*inverse_roughness*inverse_roughness
        *exp(-inv_hn2*(ht*ht+hb*hb))*dot(H,H)*inv_hn2*inv_hn2*vista_saturate(ldotn);
    return min(vec3(32768.0),direct+fresnel*shadowed*ward+ambient+emission);
}
