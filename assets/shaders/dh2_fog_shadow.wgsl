// Complete postFX_fogshadow_generate::ComputeShadowTerm.
// World/camera coordinates supplied here are game Z-up, not backend coordinates.
// ParallelSamplingUniform: source rows at0..19, view row20, fars21, flags25.
@group(0) @binding(15) var<storage,read> fog_parallel:array<vec4<f32>>;
struct FogShadowParameters {
    viewport:vec4<f32>,
    // resolutionScale.xy, ocean/ProjectedShadowHeightMapExtent, unused.
    resolution_projected:vec4<f32>,
    // jitterVectors.xy, unused.
    jitter:vec4<f32>,
    // useProjectedShadowHeightMap, useTemporal, quality (0 low/1 average/2 high).
    options:vec4<u32>,
}
fn fog_shadow_parallel(position:vec4<f32>,
    atlas:texture_depth_2d_array,comparison:sampler_comparison)->f32 {
    if bitcast<u32>(fog_parallel[25].x)==0u {return 1.0;}
    let masks=step(fog_parallel[21],vec4(-dot(position,fog_parallel[20])));
    let slice=u32(masks.x+masks.y+masks.z+masks.w);
    let i=slice*4u;
    let coordinates=vec3(dot(fog_parallel[i],position),dot(fog_parallel[i+1u],position),dot(fog_parallel[i+2u],position));
    return textureSampleCompareLevel(atlas,comparison,coordinates.xy,i32(slice),shadow_depth_reference(coordinates.z));
}
fn fog_shadow_term(parameters:FogShadowParameters,world_from_clip:mat4x4<f32>,eye:vec3<f32>,
    pixel:vec2<u32>,uv:vec2<f32>,depth:texture_depth_2d,projected_height:texture_2d<f32>,
    linear_clamp:sampler,
    atlas:texture_depth_2d_array,comparison:sampler_comparison)->f32 {
    var taps=8u;
    if parameters.options.z==2u {taps=16u;}
    else if parameters.options.z==0u {taps=3u;}
    let projected=parameters.options.x!=0u;
    let raw=textureSampleLevel(depth,linear_clamp,uv*parameters.resolution_projected.xy,0);
    var clamped=raw;
    if !projected {clamped=shadow_saturate(raw+0.001);}
    let ndc=vec2(1.0,-1.0)*((vec2<f32>(pixel)-parameters.viewport.xy-parameters.viewport.zw*0.5)/(parameters.viewport.zw*0.5));
    let homogeneous=world_from_clip*vec4(ndc,clamped,1.0);
    let position=homogeneous.xyz/homogeneous.w;
    let ray=position-eye;
    let distance=length(ray);
    let direction=ray/distance;
    let fars=fog_parallel[21];
    let maximum=select(dot(fars,vec4(1.0)),7000.0,projected);
    let step_size=min(maximum,distance)/f32(taps);
    let ray_step=direction*step_size;
    var interleaved=pixel%vec2(8u);
    if parameters.options.y!=0u {
        interleaved=vec2<u32>(vec2<f32>(interleaved)+vec2(0.5)+parameters.jitter.xy);
    }
    let offset=f32(screen_b8(interleaved))*step_size*(1.0/64.0);
    var ray_position=vec4(eye+direction*offset,1.0);
    var sum=0.0;
    for(var i=0u;i<taps;i++) {
        let visibility=fog_shadow_parallel(ray_position,atlas,comparison);
        if projected {
            let shadow_uv=vec2(0.5)+ray_position.xy/parameters.resolution_projected.z;
            let height=textureSampleLevel(projected_height,linear_clamp,shadow_uv,0.0).r;
            let t=shadow_saturate(ray_position.z-height);
            let projected_shadow=t*t*(3.0-2.0*t);
            // The installed shader adds the two visibilities. Its R8 target
            // performs storage conversion afterwards; do not multiply/clamp here.
            sum+=visibility+projected_shadow;
        } else {sum+=visibility;}
        ray_position=vec4(ray_position.xyz+ray_step,1.0);
    }
    return sum/f32(taps);
}
