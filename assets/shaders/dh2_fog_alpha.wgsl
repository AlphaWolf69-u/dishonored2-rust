// fogHelpers' alpha branch. Concatenate after dh2_fog_math.
struct AlphaWorldSample {
    ray:vec3<f32>,
    distance:f32,
    camera_height:f32,
    curve:f32,
    canvas:f32,
}
fn fog_alpha_world(p:WorldFogParameters,world_type:vec4<f32>,s:AlphaWorldSample)->vec4<f32> {
    if dot(world_type,vec4(1.0))<=0.0 {return vec4(0.0);}
    let direction=s.ray/length(s.ray);
    var distance=vec4(0.0);
    if world_type.x>0.0 {distance=fog_distance(p,fog_normalized_distance(p,s.distance),s.curve);}
    var vertical=vec4(0.0);
    if world_type.z>0.0 {
        vertical=fog_vertical(p,direction.z,s.camera_height,s.distance,fog_saturate((s.distance-p.vertical.x)*p.vertical.y));
    }
    var color=mix(distance,vertical,vertical.a);
    // ComputeWorldFog declares the scattering switch but has no scattering
    // calculation. Turbulence uses the distance end colour in this branch.
    if (p.flags.x&128u)!=0u {
        let factor=fog_saturate((s.distance-p.turbulence.x)/(p.turbulence.y-p.turbulence.x));
        color=vec4(mix(color.rgb,mix(color.rgb,p.distance_end.rgb,s.canvas),factor),color.a);
    }
    return color;
}
struct AlphaVolumeParameters {
    distance:vec4<f32>,
    color_start:vec4<f32>,
    color_end:vec4<f32>,
    density:vec4<f32>, // density factor, falloff, unused, unused
    scattering_direction_phase:vec4<f32>,
    scattering_color_intensity:vec4<f32>,
    volume_type:vec4<f32>,
}
fn fog_alpha_extinction(density:f32)->f32 {
    let exponential=exp(-density);
    return 1.0-select(1.0,min(exponential,1.0),exponential==exponential);
}
fn fog_alpha_volume(p:AlphaVolumeParameters,ray:vec3<f32>,distance:f32,camera_height:f32,interval:vec2<f32>)->vec4<f32> {
    if p.volume_type.x<=0.0 {return vec4(0.0);}
    let depth=min(distance,interval.y)-max(0.0,interval.x);
    var normalized=fog_saturate((depth-p.distance.x)*p.distance.y);
    var color=mix(p.color_start.rgb,p.color_end.rgb,normalized);
    var ray_length=0.0;
    var direction=vec3(0.0);
    if p.volume_type.y>0.0 || p.volume_type.z>0.0 {
        ray_length=length(ray);
        direction=ray/ray_length;
    }
    var extinction:f32;
    if p.volume_type.z>0.0 {
        let start=select(0.0,interval.x,interval.x<interval.y);
        let altitude=camera_height+(ray_length*start)*direction.z;
        let dy=direction.z+select(-0.00001,0.00001,direction.z>0.0);
        normalized*=p.density.x;
        let density=p.distance.z*(exp(-p.density.y*altitude)-exp(-p.density.y*(altitude+dy*normalized)))/(p.density.y*dy);
        extinction=fog_alpha_extinction(density);
    } else {
        extinction=fog_alpha_extinction(p.distance.z*normalized);
    }
    if p.volume_type.y>0.0 {
        let phase=fog_mie_phase(dot(p.scattering_direction_phase.xyz,direction),p.scattering_direction_phase.w);
        color+=p.scattering_color_intensity.rgb*(p.scattering_color_intensity.a*phase);
    }
    return vec4(color,extinction);
}
fn fog_alpha_mixed(world:vec4<f32>,volume:vec4<f32>,scattering:vec3<f32>,precomputed:vec4<f32>,depth_ratio:f32)->vec4<f32> {
    let rgb=volume.rgb*volume.a+world.rgb*world.a*(1.0-volume.a)+scattering;
    let alpha=fog_saturate(volume.a+world.a*(1.0-volume.a));
    return mix(vec4(rgb,alpha),precomputed,depth_ratio);
}
fn fog_alpha_composite(color:vec3<f32>,fog:vec4<f32>)->vec3<f32> {
    return fog.rgb+color*(1.0-fog.a);
}
