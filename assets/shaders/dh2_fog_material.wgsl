// Registered after the shared fog math and alpha helpers in dh2_material_fog.
#import bevy_pbr::mesh_view_bindings::view
#import dh2_material_samplers::linear_clamp

struct MaterialFogParameters {
    control:vec4<f32>,
    world:WorldFogParameters,
    world_type:vec4<f32>,
    unproject:vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(161) var<storage,read> material_fog:MaterialFogParameters;
@group(#{MATERIAL_BIND_GROUP}) @binding(162) var material_fog_world:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(163) var material_fog_canvas:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(164) var material_fog_curve:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(165) var material_fog_depth:texture_depth_2d;
@group(#{MATERIAL_BIND_GROUP}) @binding(166) var material_fog_wrap:sampler;
struct MaterialFogObject {
    volume:AlphaVolumeParameters,
    interval:vec4<f32>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(167) var<storage,read> material_fog_objects:array<MaterialFogObject>;

fn material_fog_ray(world_position:vec3<f32>)->vec3<f32> {
    let ray=world_position-view.world_position;
    return vec3(ray.x,-ray.z,ray.y);
}
fn material_fog_world_sample(ray:vec3<f32>,distance:f32,uv:vec2<f32>)->vec4<f32> {
    let normalized=fog_normalized_distance(material_fog.world,distance);
    let curve=textureSampleLevel(material_fog_curve,linear_clamp,vec2(normalized,0.5),0.0).r;
    let canvas=textureSampleLevel(material_fog_canvas,material_fog_wrap,uv,0.0).r;
    return fog_alpha_world(material_fog.world,material_fog.world_type,
        AlphaWorldSample(ray,distance,material_fog.control.z,curve,canvas));
}
fn material_fog_volume_sample(tag:u32,ray:vec3<f32>,distance:f32)->vec4<f32> {
    if material_fog.control.w<0.0 {return vec4(0.0);}
    var index=tag;
    if (tag&0x80000000u)!=0u {
        index=u32(material_fog.control.w)+(tag&0x7fffffffu);
    }
    if index>=arrayLength(&material_fog_objects) {return vec4(0.0);}
    let object=material_fog_objects[index];
    return fog_alpha_volume(object.volume,ray,distance,material_fog.control.z,object.interval.xy);
}
fn material_fog_composite(color:vec3<f32>,fog:vec4<f32>)->vec3<f32> {
    return fog.rgb*(139.26*view.exposure)+color*(1.0-fog.a);
}
fn material_fog_pixel(color:vec3<f32>,position:vec4<f32>,world_position:vec3<f32>,tag:u32)->vec3<f32> {
    if material_fog.control.x==0.0 {return color;}
    let uv=position.xy/vec2<f32>(textureDimensions(material_fog_depth));
    let ray=material_fog_ray(world_position);
    let ray_length=length(ray);
    let direction=ray*select(0.0,1.0/ray_length,ray_length>0.00000001);
    // Actual cached pixel stages609/621/648 consume SV_POSITION.w directly.
    let world=material_fog_world_sample(direction,position.w,uv);
    let volume=material_fog_volume_sample(tag,direction,position.w);
    let precomputed=textureSampleLevel(material_fog_world,linear_clamp,uv,0.0);
    let opaque=textureSampleLevel(material_fog_depth,linear_clamp,uv,0);
    let ratio=(1.0-position.z)/(1.0001-opaque);
    // Installed lighting.decl explicitly assigns accumScattering=0.0;
    // its INTEGRATE_SCATTER invocation is commented out.
    return material_fog_composite(color,fog_alpha_mixed(world,volume,vec3(0.0),precomputed,ratio));
}
fn material_fog_vertex(position:vec4<f32>,world_position:vec3<f32>,use_mesh:bool,tag:u32)->vec4<f32> {
    if material_fog.control.x==0.0 {return vec4(0.0);}
    let uv=(position.xy/position.w)*vec2(0.5,-0.5)+vec2(0.5);
    let world=material_fog_world_sample(material_fog_ray(world_position),position.w,uv);
    let volume=material_fog_volume_sample(tag,material_fog_ray(world_position),position.w);
    let precomputed=textureSampleLevel(material_fog_world,linear_clamp,uv,0.0);
    let opaque=textureSampleLevel(material_fog_depth,linear_clamp,uv,0);
    var ratio:f32;
    if use_mesh {
        ratio=fog_saturate((1.0-position.z)/(1.0001-opaque));
    } else {
        let linear=material_fog.unproject.x/(material_fog.unproject.y*opaque+material_fog.unproject.z);
        ratio=fog_saturate(position.w/(material_fog.control.y+linear));
    }
    return fog_alpha_mixed(world,volume,vec3(0.0),precomputed,ratio);
}
