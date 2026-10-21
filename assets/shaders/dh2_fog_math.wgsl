// atm/distanceFog/renderWorld and fogHelpers; all positions/directions are Z-up.
struct WorldFogParameters {
    distance:vec4<f32>,
    distance_start:vec4<f32>,
    distance_end:vec4<f32>,
    vertical:vec4<f32>,
    vertical_start:vec4<f32>,
    vertical_end:vec4<f32>,
    height:vec4<f32>, // start, end, falloff, unused
    scattering_direction_phase:vec4<f32>,
    scattering_color_intensity:vec4<f32>,
    effects:vec4<f32>, // far clip, scattering depth start, GodRayShadow, unused
    turbulence:vec4<f32>, // start, end, unused, unused
    turbulence_color:vec4<f32>,
    flags:vec4<u32>,
}
struct WorldFogSample {
    ray:vec3<f32>,
    camera_height:f32,
    scene_depth:f32,
    interior_depth:f32,
    distance_curve:f32,
    shadow:f32,
    sky_cloud_density:vec4<f32>,
    turbulence:f32,
}
fn fog_saturate(x:f32)->f32 {
    // The installed min/max path saturates NaN to zero.
    return select(0.0,min(x,1.0),x>0.0);
}
fn fog_normalized_distance(p:WorldFogParameters,distance:f32)->f32 {
    return fog_saturate((distance-p.distance.x)*p.distance.y);
}
fn fog_distance(p:WorldFogParameters,normalized:f32,curve:f32)->vec4<f32> {
    return vec4(mix(p.distance_start,p.distance_end,normalized).rgb,1.0-exp(-p.distance.z*curve));
}
fn fog_vertical(p:WorldFogParameters,view_z:f32,camera_z:f32,distance:f32,normalized:f32)->vec4<f32> {
    let altitude=camera_z-p.height.x;
    let dy=view_z+select(-0.00001,0.00001,view_z>0.0);
    let hit=altitude+distance*dy;
    let density=(exp(-p.height.z*altitude)-exp(-p.height.z*hit))/(p.height.z*dy);
    let exponential=exp(-density);
    // DXBC min(exp(NaN),1) returns 1; retain zero fog at zero falloff.
    let extinction=1.0-select(1.0,min(exponential,1.0),exponential==exponential);
    let height=fog_saturate((hit-p.height.x)/(p.height.y-p.height.x));
    let color=extinction*mix(p.vertical_start,p.vertical_end,height).rgb;
    return vec4(color,(1.0-exp(-p.vertical.z*normalized))*extinction);
}
fn fog_mie_phase(cosine:f32,g:f32)->f32 {
    return (1.5/12.566370614359172)*(1.0-g*g)
        *pow(max(0.00001,1.0+g*g-2.0*g*cosine),-1.5)
        *(1.0+cosine*cosine)/(2.0+g*g);
}
fn fog_world(p:WorldFogParameters,s:WorldFogSample)->vec4<f32> {
    let mask=1.0-max(max(s.sky_cloud_density.x,s.sky_cloud_density.y),max(s.sky_cloud_density.z,s.sky_cloud_density.w));
    if mask<=0.0 {return vec4(0.0);}
    let linear_z=s.scene_depth-s.interior_depth;
    let ray_length=length(s.ray);
    let direction=s.ray/ray_length;
    let distance=linear_z*ray_length;
    var color=vec4(0.0);
    if (p.flags.x&8u)!=0u {color=fog_distance(p,fog_normalized_distance(p,distance),s.distance_curve);}
    var vertical=vec4(0.0);
    if (p.flags.x&16u)!=0u {
        vertical=fog_vertical(p,direction.z,s.camera_height,distance,fog_saturate((distance-p.vertical.x)*p.vertical.y));
    }
    color=mix(color,vertical,vertical.a);
    let shadow=select(1.0,s.shadow,(p.flags.x&64u)!=0u);
    if (p.flags.x&32u)!=0u {
        var scattering=p.scattering_color_intensity.rgb*(p.scattering_color_intensity.a
            *fog_mie_phase(dot(p.scattering_direction_phase.xyz,direction),p.scattering_direction_phase.w));
        let depth_mask=fog_saturate((linear_z-p.effects.y)/(p.effects.x-p.effects.y));
        scattering*=min(1.0,max(scattering.r,max(scattering.g,scattering.b)))*depth_mask;
        color=vec4(color.rgb*mix(1.0,shadow,p.effects.z)+shadow*scattering,color.a);
    }
    if (p.flags.x&128u)!=0u {
        let noise=p.turbulence_color.rgb*mix(1.0,shadow,p.effects.z);
        let factor=fog_saturate((linear_z-p.turbulence.x)/(p.turbulence.y-p.turbulence.x));
        color=vec4(mix(color.rgb,mix(color.rgb,noise,s.turbulence),factor),color.a);
    }
    return vec4(color.rgb,color.a*mask);
}
