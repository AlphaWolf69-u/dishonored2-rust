// arkOcean colour inputs. Compose after ocean surface and water helpers.
struct OceanColourParameters {
    // ProjectedShadowHeightMapExtent, start/end height, WaterLevel.
    projected:vec4<f32>,
    // WaterColor.xyz, Extinction.
    water:vec4<f32>,
    // parallelLightDir.xyz, Inscattering.
    light:vec4<f32>,
    // parallelLightColor.rgb, SpecularPower.
    sun:vec4<f32>,
    // positionToViewTexture; screenPosToTexcoord uses only scale.zw.
    pixel_to_texture:vec4<f32>,
}
// Implicit-derivative foam/foam-depth sampling belongs to the actual fragment
// entry. Keeping those two samples explicit also permits numerical stage tests.
fn ocean_colour_input(parameters:OceanColourParameters,surface:OceanSurfaceParameters,
    view_from_world:mat4x4<f32>,position:vec3<f32>,legacy:vec4<f32>,pixel:vec2<f32>,
    front_face:bool,refraction:f32,foam_texture:f32,foam_depth:f32,
    permutation:texture_2d<f32>,gradients:texture_2d<f32>,
    point_wrap:sampler,bilinear_wrap:sampler,bilinear_clamp:sampler,
    shore_distance:texture_2d<f32>,displacement:texture_2d<f32>,
    collision_height:texture_2d<f32>,fft_foam:texture_2d<f32>,
    amplitude_mask:texture_2d<f32>,projected_height:texture_2d<f32>)->WaterSurfaceInput {
    var global_uv=legacy.xy/surface.spatial.w+vec2(0.5);
    global_uv.x=1.0-global_uv.x;
    let amplitude=textureSampleLevel(amplitude_mask,bilinear_clamp,global_uv,0.0).r;
    let shadow_uv=vec2(0.5)+legacy.xy/parameters.projected.x;
    let height=textureSampleLevel(projected_height,bilinear_clamp,shadow_uv,0.0).r;
    let visibility=screen_ray_smooth(parameters.projected.y,parameters.projected.z,position.z-height);
    let view_position=(view_from_world*vec4(position,1.0)).xyz;
    let to_camera=surface.camera.xyz-position;
    let view_direction=to_camera/length(to_camera);
    let input=OceanSurfaceInput(legacy.xy,legacy.w,legacy.z,amplitude,false,foam_texture);
    let normal=ocean_surface_normal(surface,input,permutation,gradients,point_wrap,bilinear_wrap,
        bilinear_clamp,shore_distance,displacement,collision_height,fft_foam);
    // Source ocean passes unused IBLInfo. The caller must use the authored
    // ocean sky/uniform path rather than infer localized probe indices.
    return WaterSurfaceInput(true,!front_face,parameters.projected.w,pixel,position,view_position,
        normal.normal,view_direction,parameters.water.xyz,parameters.water.w,parameters.light.w,
        parameters.light.xyz,parameters.sun.xyz,parameters.sun.w,visibility,refraction,
        normal.foam,foam_texture,screen_ray_saturate(foam_depth),amplitude,vec3(0u),vec3(0.0));
}
