#define_import_path dh2_area_light_opaque
#import dh2_ward::{WardSurface,DirectLighting,ward_translucency,ward_translucency_factor}
#import dh2_fresnel::ward_fresnel
#import dh2_reflection::integrated_ward
#import dh2_shadow::shadow_saturate
#import bevy_pbr::mesh_view_bindings::{area_light_records,area_light_textures,area_light_sampler}

// Decode the scene's seven-vector TLF packet into the area evaluator's context.
struct TiledLightRecord {
    position:vec3<f32>, falloff:f32,
    color:vec3<f32>, cutoff:f32,
    direction:vec3<f32>, decay:f32,
    specular:f32, scatter_value:f32, scatter_scale:f32, scatter_offset:f32,
    spot:vec3<f32>, scatter_turbulence:f32,
    axis:vec3<f32>, origin_offset:f32,
    shadow_setup:u32, flags:u32, padding:vec2<u32>,
}
fn opaque_area_lighting(index:u32,host_surface:WardSurface,host_position:vec3<f32>)->DirectLighting {
#ifdef DH2_TRACE_LIGHT_COMPONENT
    // Profiling attribution: 10 omits every opaque area light.
    if #{DH2_TRACE_LIGHT_COMPONENT} == 10u {return DirectLighting(vec3(0.0),vec3(0.0));}
#endif
    if index>=arrayLength(&area_light_records)/7u {return DirectLighting(vec3(0.0),vec3(0.0));}
    let base=index*7u;
    let a=bitcast<vec4<f32>>(area_light_records[base]);
    let b=bitcast<vec4<f32>>(area_light_records[base+1u]);
    let c=bitcast<vec4<f32>>(area_light_records[base+2u]);
    let d=bitcast<vec4<f32>>(area_light_records[base+3u]);
    let e=bitcast<vec4<f32>>(area_light_records[base+4u]);
    let f=bitcast<vec4<f32>>(area_light_records[base+5u]);
    let g=area_light_records[base+6u];
    let record=TiledLightRecord(a.xyz,a.w,b.xyz,b.w,c.xyz,c.w,d.x,d.y,d.z,d.w,e.xyz,e.w,f.xyz,f.w,g.x,g.y,g.zw);
    var surface=host_surface;
    surface.normal=vec3(host_surface.normal.x,-host_surface.normal.z,host_surface.normal.y);
    surface.view=vec3(host_surface.view.x,-host_surface.view.z,host_surface.view.y);
    let position=vec3(host_position.x,-host_position.z,host_position.y);
    let area=area_ward(surface,position,record,1.0,area_light_textures,area_light_sampler);
    var lighting=area.lighting;
    if surface.translucency.enabled!=0.0 {
        // BRDF_vegetation::ComputeLightingArea: Rho_t = INVPI*(1-(1-F))*albedo,
        // unlike the point-light complement (see docs/original-defects.md).
        var t=surface.translucency;
        t.vertex_normal=vec3(t.vertex_normal.x,-t.vertex_normal.z,t.vertex_normal.y);
        let ldotn=-dot(area.direction,t.vertex_normal);
        let fresnel=ward_fresnel(surface.ior,clamp(ldotn,0.0,1.0),surface.fresnel_strength);
        let rho=(1.0/3.14159265358979323846)*fresnel*surface.diffuse;
        let amount=ward_translucency(t,ward_translucency_factor(t,area.direction),area.direction,surface.view);
        lighting.diffuse+=rho*t.transmittance*area.irradiance*amount;
    }
    return lighting;
}
