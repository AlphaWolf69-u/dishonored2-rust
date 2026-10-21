// PostFX/LightScatteing. Concatenate after dh2_shadow/dh2_light_scattering.
// The owner supplies the SCATTERING heads, shared node pool and ready atlas.
// Head texels use the existing TLF compute backend's linear storage transport.
override SCATTER_TEMPORAL:bool=false;
override SCATTER_BW_EXCLUDED:bool=false;
struct LightScatterView {
    // Game-coordinate inverse view projection (clip Z=0 reconstructs far).
    world_from_clip:mat4x4<f32>,
    camera_far:vec4<f32>,
    position_to_view_texture:vec4<f32>,
    // Original global resolutionScale.xy and jitterVectors.xy.
    canvas_scale_jitter:vec4<f32>,
    // Lighting shift, tile count X/Y, atm/worldFog/resolutionScale.
    grid:vec4<u32>,
    // blackWhiteModeColor RGB and blackWhiteModeRuntime.
    black_white:vec4<f32>,
}
@group(0) @binding(0) var<uniform> scatter_view:LightScatterView;
@group(0) @binding(1) var<storage,read> scatter_lights:array<TiledLightRecord>;
@group(0) @binding(2) var<storage,read> scatter_pool:array<u32>;
@group(0) @binding(3) var<storage,read> scatter_heads:array<u32>;
@group(0) @binding(4) var<storage,read> scatter_shadows:array<ShadowInfo>;
@group(0) @binding(5) var scatter_atlas:texture_depth_2d_array;
@group(0) @binding(6) var scatter_comparison:sampler_comparison;
@group(0) @binding(7) var scatter_depth:texture_2d<f32>;
@group(0) @binding(8) var scatter_first_canvas:texture_2d<f32>;
@group(0) @binding(9) var scatter_second_canvas:texture_2d<f32>;
@group(0) @binding(10) var scatter_bilinear:sampler;

// ComputeShadowTerm reads the selected point side on EVERY march step. Spots
// use shadow_setup directly; its split-surface-shadow flag is not used here.
fn scatter_shadow_term(kind:u32,pixel:vec2<u32>,record:TiledLightRecord,
    ray:vec3<f32>,minimum:f32,length:f32,quality:u32)->f32 {
    var taps=8u;
    if quality==2u {taps=16u;}
    else if quality==0u {taps=3u;}
    let step=length/f32(taps);
    let ray_step=ray*step;
    var offset=0.0;
    if quality>0u {
        var interleaved=pixel%vec2(8u);
        if SCATTER_TEMPORAL {
            interleaved=vec2<u32>(vec2<f32>(interleaved)+vec2(0.5)+scatter_view.canvas_scale_jitter.zw);
        }
        offset=f32(screen_b8(interleaved))*step*(1.0/64.0);
    }
    let origin=scatter_view.camera_far.xyz;
    var sample_position=fma(ray,vec3(minimum+offset),origin);
    var integral_position=fma(ray,vec3(minimum),origin);
    let light_position=fma(-record.direction,vec3(record.scatter_offset),record.position);
    var accumulated=0.0;
    for(var tap=0u;tap<taps;tap+=1u) {
        var index=record.shadow_setup;
        if kind<2u {index+=omni_shadow_side(record.position-sample_position);}
        let shadow=scatter_shadows[index];
        let homogeneous=shadow_position(shadow.world_to_shadow,sample_position,false);
        let position=homogeneous.xyz/homogeneous.w;
        let visibility=textureSampleCompareLevel(scatter_atlas,scatter_comparison,
            position.xy,shadow_slice(shadow.info),shadow_depth_reference(position.z));
        accumulated+=scatter_integral(integral_position,ray,light_position,step)*visibility;
        sample_position+=ray_step;
        integral_position+=ray_step;
    }
    return accumulated;
}

fn scatter_opaque_light(kind:u32,pixel:vec2<u32>,ray:vec3<f32>,distance:f32,
    record:TiledLightRecord)->vec4<f32> {
    let origin=scatter_view.camera_far.xyz;
    let light=scatter_light_from_record(record);
    let segment=scatter_light_interval(kind,origin,ray,distance,light);
    var scatter=0.0;
    if segment.y>0.0 {
        if (record.flags&2u)!=0u && record.shadow_setup!=0xffffffffu {
            scatter=scatter_shadow_term(kind,pixel,record,ray,segment.x,segment.y,1u);
        } else {
            let position=fma(-light.direction,vec3(light.offset),light.position);
            scatter=scatter_integral(fma(ray,vec3(segment.x),origin),ray,position,segment.y);
        }
        let uv=vec2<f32>(pixel)*scatter_view.position_to_view_texture.zw
            *scatter_view.canvas_scale_jitter.xy;
        let first=textureSampleLevel(scatter_first_canvas,scatter_bilinear,uv,0.0).x;
        let second=textureSampleLevel(scatter_second_canvas,scatter_bilinear,uv,0.0).x;
        scatter*=scatter_turbulence(first,second,light);
        scatter*=scatter_attenuation(kind,origin,ray,segment.x,segment.y,light);
    }
    return vec4(light.color*scatter,scatter);
}

@vertex fn light_scatter_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn light_scatter_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    let uv=pixel.xy*scatter_view.position_to_view_texture.zw;
    let ndc=uv*vec2(2.0,-2.0)+vec2(-1.0,1.0);
    let far_h=scatter_view.world_from_clip*vec4(ndc,0.0,1.0);
    let far_position=far_h.xyz/far_h.w;
    let depth=textureLoad(scatter_depth,vec2<i32>(pixel.xy),0).y;
    let delta=(far_position-scatter_view.camera_far.xyz)*(depth/scatter_view.camera_far.w);
    let distance=length(delta);
    let ray=delta*select(0.0,1.0/distance,distance>1e-8);
    let address=vec2<u32>(pixel.xy*f32(scatter_view.grid.w))>>vec2(scatter_view.grid.x);
    var sums:array<vec4<f32>,3>;
    for(var kind=0u;kind<3u;kind+=1u) {
        let texel=address+vec2(kind*scatter_view.grid.y,0u);
        var head=scatter_heads[texel.y*scatter_view.grid.y*3u+texel.x];
        var count=0u;
        while head!=0x7fffffu && count<32u {
            let node=scatter_pool[head];
            let light=scatter_lights[node&511u];
            head=node>>9u;
            count+=1u;
            sums[kind]=scatter_add(sums[kind],scatter_opaque_light(kind,vec2<u32>(pixel.xy),ray,distance,light));
        }
    }
    var result=scatter_resolve(sums[0],sums[1],sums[2]);
    result=vec4(black_white_overlay(result.xyz,scatter_view.black_white.xyz,
        scatter_view.black_white.w!=0.0,SCATTER_BW_EXCLUDED),result.w);
    return result;
}
