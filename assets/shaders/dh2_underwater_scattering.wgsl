// underWaterPostFX::ComputeUnderWaterScattering, into the fog radiance buffer.
struct UnderwaterParameters {
    water:WaterRefractionParameters,
    shadow:WaterShadowParameters,
    world_from_clip:mat4x4<f32>,
    camera_far:vec4<f32>,
    optics:vec4<f32>, // Extinction, unused, unused, Inscattering.
    sun_boost:vec4<f32>,
    // Logical fog viewport extent; sampled depth-pyramid UV scale.
    viewport:vec4<f32>,
    rays:vec4<f32>, // intensity, shadowing, chromatic option, temporal option.
    ray_jitter:vec4<f32>, // temporal XY, visibility target extent.
}
@group(0) @binding(0) var<uniform> underwater:UnderwaterParameters;
@group(0) @binding(1) var underwater_hdr:texture_2d<f32>;
@group(0) @binding(2) var underwater_opaque:texture_depth_2d;
@group(0) @binding(3) var underwater_reduced:texture_2d<f32>;
@group(0) @binding(4) var underwater_caustics:texture_2d_array<f32>;
@group(0) @binding(5) var underwater_shadows:texture_depth_2d_array;
@group(0) @binding(6) var underwater_mask:texture_2d<u32>;
@group(0) @binding(7) var underwater_clamp:sampler;
@group(0) @binding(8) var underwater_wrap:sampler;
@group(0) @binding(9) var underwater_compare:sampler_comparison;
@group(0) @binding(10) var<storage,read> underwater_adaptation:array<vec4<f32>,2>;
@group(0) @binding(11) var underwater_ray_depth:texture_depth_2d;
@group(0) @binding(12) var underwater_ray_shadow:texture_2d<f32>;
@vertex fn underwater_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
fn underwater_position(ndc:vec2<f32>,linear_z:f32)->vec3<f32> {
    let far=underwater.world_from_clip*vec4(ndc,0.0,1.0);
    return underwater.camera_far.xyz+(far.xyz/far.w-underwater.camera_far.xyz)
        *(linear_z/underwater.camera_far.w);
}
@fragment fn underwater_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let uv=pixel.xy/underwater.viewport.xy;
    // Host integration: read the same 0x40 stencil test explicitly. This keeps
    // classification in camera coordinates when fog and stencil sizes differ.
    let mask_position=point_clamp_texel(uv,textureDimensions(underwater_mask));
    if (textureLoad(underwater_mask,mask_position,0).x&64u)==0u {discard;}
    let p=underwater.water;
    var radiance=textureSampleLevel(underwater_hdr,underwater_clamp,uv,0.0).rgb;
    radiance=hdr_range_expand(radiance,p.extinction.w!=0.0,underwater_adaptation[0].x,p.hdr.y);
    let raw=textureSampleLevel(underwater_opaque,underwater_clamp,uv,0);
    let opaque_z=p.unproject.x/(p.unproject.y*raw+p.unproject.z);
    let water_z=textureSampleLevel(underwater_reduced,underwater_clamp,uv*underwater.viewport.zw,0.0).y;
    let ndc=uv*vec2(2.0,-2.0)+vec2(-1.0,1.0);
    let opaque=underwater_position(ndc,opaque_z);
    let water=underwater_position(ndc,water_z);
    let shadow=water_parallel_shadow(underwater.shadow,water,underwater_shadows,underwater_compare);
    let time=(p.clock_caustics.x*p.clock_caustics.y)%32.0;
    let caustic_uv=(opaque.xy+opaque.z*p.light_level.xy)*p.clock_caustics.z;
    let chromatic=vec2(p.clock_caustics.w);
    let layer=texture_array_layer(time,textureNumLayers(underwater_caustics));
    let caustics=vec3(
        textureSampleLevel(underwater_caustics,underwater_wrap,caustic_uv-chromatic,layer,0.0).r,
        textureSampleLevel(underwater_caustics,underwater_wrap,caustic_uv,layer,0.0).r,
        textureSampleLevel(underwater_caustics,underwater_wrap,caustic_uv+chromatic,layer,0.0).r);
    let camera_height=max(p.light_level.w-underwater.camera_far.z,0.0);
    let distance=length(underwater.camera_far.xyz-water);
    let point_height=max(p.light_level.w-water.z,0.0);
    let scattering=water_inscattering(camera_height,point_height,distance,p.extinction.xyz)
        *(underwater.optics.w/underwater.sun_boost.w);
    radiance+=underwater.sun_boost.xyz*caustics*p.light_level.z;
    radiance*=exp(-p.extinction.xyz*point_height)*max(0.25,shadow);
    radiance*=exp(-p.extinction.xyz*distance);
    radiance+=scattering*underwater.sun_boost.xyz;
    let extinction=max(p.extinction.x,max(p.extinction.y,p.extinction.z));
    let alpha_scattering=max(scattering.x,max(scattering.y,scattering.z));
    let dispersion=mix(1.0,shadow,smoothstep(p.light_level.w-underwater.optics.x,
        p.light_level.w,water.z));
    let alpha=1.0-(exp(-extinction*point_height)*dispersion*exp(-extinction*distance)+alpha_scattering);
    return vec4(radiance,alpha);
}

// water/ComputeGodRays, AVERAGE_QUALITY, followed by CopyWaterStencil.
@fragment fn underwater_rays_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let uv=pixel.xy/underwater.viewport.xy;
    if (textureLoad(underwater_mask,point_clamp_texel(uv,textureDimensions(underwater_mask)),0).x&64u)==0u {discard;}
    let depth=clamp(textureSampleLevel(underwater_ray_depth,underwater_clamp,uv,0)+0.001,0.0,1.0);
    let ndc=vec2<f32>(vec2<u32>(pixel.xy))/underwater.viewport.xy*vec2(2.0,-2.0)+vec2(-1.0,1.0);
    let position=underwater.world_from_clip*vec4(ndc,depth,1.0);
    let ray=position.xyz/position.w-underwater.camera_far.xyz;
    let direction=ray/length(ray);
    let distance=min(dot(underwater.shadow.slice_fars,vec4(1.0)),length(ray));
    let step=distance/8.0;
    var interleave=vec2<u32>(pixel.xy)%vec2(8u);
    if underwater.rays.w!=0.0 {interleave=vec2<u32>(vec2<f32>(interleave)+vec2(0.5)+underwater.ray_jitter.xy);}
    let offset=f32(screen_b8(interleave))*step/64.0;
    var point=underwater.camera_far.xyz+direction*offset;
    // Unlike scattering, the source rays do not divide by visibility boost.
    let extinction=underwater.water.extinction.xyz*underwater.sun_boost.w;
    let per_step=vec3(1.0)-exp(-extinction*step);
    let time=(underwater.water.clock_caustics.x*underwater.water.clock_caustics.y)%32.0;
    let layer=texture_array_layer(time,textureNumLayers(underwater_caustics));
    var visibility=1.0;
    if underwater.rays.y!=0.0 {visibility=textureSampleLevel(underwater_ray_shadow,underwater_clamp,uv,0.0).x;}
    var colour=vec3(0.0);
    for(var i=0u;i<8u;i++) {
        let vertical=max(underwater.water.light_level.w-point.z,0.0);
        let caustic_uv=(point.xy+point.z*underwater.water.light_level.xy)*underwater.water.clock_caustics.z;
        let chromatic=vec2(underwater.water.clock_caustics.w);
        let green=textureSampleLevel(underwater_caustics,underwater_wrap,caustic_uv,layer,0.0).r;
        var caustic=vec3(green);
        if underwater.rays.z!=0.0 {
            caustic=vec3(textureSampleLevel(underwater_caustics,underwater_wrap,caustic_uv-chromatic,layer,0.0).r,
                green,textureSampleLevel(underwater_caustics,underwater_wrap,caustic_uv+chromatic,layer,0.0).r);
        }
        colour+=per_step*exp(-extinction*vertical)*caustic*underwater.rays.x*visibility;
        point+=direction*step;
    }
    return vec4(colour,0.0);
}

@fragment fn underwater_visibility_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) f32 {
    let scale=textureDimensions(underwater_mask)/vec2<u32>(underwater.ray_jitter.zw);
    let value=textureLoad(underwater_mask,vec2<i32>(vec2<u32>(pixel.xy)*scale),0).x;
    let extinction=underwater.water.extinction.y*underwater.sun_boost.w;
    let depth=max(underwater.water.light_level.w-underwater.camera_far.z,0.0);
    return select(0.0,0.1*exp(-extinction*depth*depth),(value&64u)!=0u);
}
