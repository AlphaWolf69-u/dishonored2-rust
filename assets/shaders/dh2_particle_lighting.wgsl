// particles::ComputeIBL calls IBL::LQ_ComputeIBL, not the HQ Ward integral.
// Shared reflection_direction/radiance and ward_fresnel retain their scene units.
fn particle_lq_reflection(position:vec3<f32>,normal:vec3<f32>,view_direction:vec3<f32>,
    roughness:f32,ior:vec3<f32>,fresnel_strength:f32,cube:f32,gain:f32)->vec3<f32> {
    let light=normalize(mix(reflect(-view_direction,normal),normal,roughness));
    let ln=shadow_saturate(dot(light,normal));
    let vn=shadow_saturate(dot(view_direction,normal));
    let aperture=0.0003474660443456835+roughness*(1.3331290497744692-roughness*0.5040552688878546);
    let mip=6.0*sqrt(aperture/0.834);
    let count=reflection_planes[0];
    let index=select(count-1u,u32(max(0.0,cube)),cube>=0.0 && cube<f32(count));
    let luminance_ior=dot(vec3(0.2126,0.7152,0.0722),ior);
    let ratio=(luminance_ior-1.0)/(luminance_ior+1.0);
    let bounce=mix(gain,1.0,ratio*ratio);
    return ward_fresnel(ior,vn,fresnel_strength)*
        (reflection_radiance(position,light,index,mip)*sqrt(ln)*bounce);
}

@group(3) @binding(129) var<storage,read> particle_lights:array<TiledLightRecord>;
@group(3) @binding(130) var<storage,read> particle_light_pool:array<u32>;
@group(3) @binding(131) var<storage,read> particle_light_heads:array<u32>;
@group(3) @binding(132) var<storage,read> particle_light_shadows:array<ShadowInfo>;
@group(3) @binding(133) var particle_area_pool:texture_2d_array<f32>;

fn particle_hl2_basis(tangent:vec3<f32>,bitangent:vec3<f32>,normal:vec3<f32>)->mat3x3<f32> {
    return mat3x3(sqrt(2.0/3.0)*tangent+sqrt(1.0/3.0)*normal,
        -sqrt(1.0/6.0)*tangent+sqrt(1.0/2.0)*bitangent+sqrt(1.0/3.0)*normal,
        -sqrt(1.0/6.0)*tangent-sqrt(1.0/2.0)*bitangent+sqrt(1.0/3.0)*normal);
}
fn particle_basis_weights(basis:mat3x3<f32>,direction:vec3<f32>)->vec3<f32> {
    return vec3(shadow_saturate(dot(direction,basis[0])),shadow_saturate(dot(direction,basis[1])),shadow_saturate(dot(direction,basis[2])));
}
fn particle_parallel_shadow(position:vec3<f32>)->f32 {
    if parallel.config.x==0u {return 1.0;}
    let slice=parallel_shadow_slice(position,parallel.view_depth_row,parallel.cascade_fars);
    let coordinates=shadow_position(parallel.world_to_shadow[slice],position,true);
    return textureSampleCompareLevel(parallel_depth,parallel_comparison,coordinates.xy,
        i32(slice),shadow_depth_reference(coordinates.z));
}
fn particle_local_shadow(record:TiledLightRecord,kind:u32,position:vec3<f32>,normal:vec3<f32>,
    to_light:vec3<f32>,attenuation:f32,quality:i32)->f32 {
    if record.shadow_setup==0xffffffffu {return 1.0;}
    if attenuation<=0.0 && record.scatter_value<=0.0 {return 0.0;}
    var side=0u;
    var light_direction=to_light;
    if kind<2u {side=omni_shadow_side(to_light);}
    else {
        light_direction=record.direction;
        if (record.flags&128u)!=0u {side=spot_shadow_side(to_light,record.axis);}
    }
    let shadow=particle_light_shadows[record.shadow_setup+side];
    let bias=local_vector(8u);
    let flags=particle_material.shadow_options.x;
    let coordinates=shadow_offset(shadow.world_to_shadow,position,normal,light_direction,false,
        (flags&1u)!=0u,(flags&2u)!=0u,(flags&4u)!=0u,(flags&8u)!=0u,bias.y,bias.x,bias.z);
    return sample_local_shadow(local_depth,local_comparison,coordinates,shadow_slice(shadow.info),
        local_vector(4u).xy,quality,i32(particle_material.shadow_options.y)-1);
}
fn particle_vertex_lights(position:vec3<f32>,normal:vec3<f32>,basis:mat3x3<f32>,clip:vec4<f32>)->mat3x3<f32> {
    let sunlight=particle_view.sun_colour.rgb*particle_parallel_shadow(position);
    let weights=particle_basis_weights(basis,particle_view.sun_direction.xyz);
    var colours=mat3x3(sunlight*weights.x,sunlight*weights.y,sunlight*weights.z);
    let coordinates=vec2(0.5+clip.x/clip.w*0.5,0.5-clip.y/clip.w*0.5)*particle_view.exposure_extent.zw;
    let pixel=vec2<u32>(trunc(coordinates));
    let kinds=array(1u,0u,2u);
    for (var type_index=0u;type_index<3u;type_index++) {
        let kind=kinds[type_index];
        let head=light_head_coordinate(pixel,kind,particle_view.scene_grid.x,particle_view.scene_grid.y);
        let head_index=head.y*(particle_view.scene_grid.y*4u)+head.x;
        var list=begin_light_list(particle_light_heads[head_index]);
        while has_light(list,8u) {
            let index=advance_light(&list,particle_light_pool[list.head]);
            let record=particle_lights[index];
            let delta=record.position-position;
            let distance=length(delta);
            let to_light=delta/distance;
            let decay=mix(1.0,1.0/distance,record.decay);
            var attenuation=decay*decay*smoothstep(record.cutoff,record.falloff,distance);
            if kind==2u {attenuation*=1.0-shadow_saturate(dot(to_light,record.direction)*record.spot.x+record.spot.y);}
            else if kind==1u {
                let angle=dot(to_light,-record.direction);
                if angle<=0.0 {attenuation=0.0;}
                else {attenuation*=1.0-shadow_saturate(angle*record.spot.x+record.spot.y);}
            }
            let visibility=particle_local_shadow(record,kind,position,normal,to_light,attenuation,0);
            let colour=record.color*visibility*attenuation;
            let w=particle_basis_weights(basis,to_light);
            colours+=mat3x3(colour*w.x,colour*w.y,colour*w.z);
        }
    }
    return colours;
}

struct ParticleSelfShadow {
    // Native HQ_ComputeAO swizzles the texture's alpha before its RGB weights.
    occlusion:vec4<f32>,
    basis:mat3x3<f32>,
    normal:vec3<f32>,
}
fn particle_directional_occlusion(direction:vec3<f32>,surface:ParticleSelfShadow)->f32 {
    if particle_material.pixel_parameters.z==0.0 {return 1.0;}
    let weights=particle_basis_weights(surface.basis,direction);
    let occlusion=dot(surface.occlusion.yzw,weights);
    let verticality=pow(shadow_saturate(dot(direction,surface.normal)),mix(1.0,8.0,surface.occlusion.x*surface.occlusion.x));
    // ComputeDirectionalOcclusion deliberately does not saturate this sum.
    return mix(occlusion,1.0,verticality);
}
fn particle_hq_reflection(position:vec3<f32>,normal:vec3<f32>,view_direction:vec3<f32>,
    roughness:f32,f0:vec3<f32>,cube:f32,gain:f32,self_shadow:ParticleSelfShadow)->vec3<f32> {
    let light=normalize(mix(reflect(-view_direction,normal),normal,roughness));
    let aperture=0.0003474660443456835+roughness*(1.3331290497744692-roughness*0.5040552688878546);
    let mip=6.0*sqrt(aperture/0.834);
    let count=reflection_planes[0];
    let index=select(count-1u,u32(max(0.0,cube)),cube>=0.0 && cube<f32(count));
    let brdf=integrated_ward(shadow_saturate(dot(view_direction,normal)),roughness);
    let bounce=mix(gain,1.0,dot(vec3(0.2126,0.7152,0.0722),f0));
    return reflection_radiance(position,light,index,mip)*particle_directional_occlusion(light,self_shadow)*bounce*(f0*brdf.x+brdf.y*(1.0-roughness*roughness));
}
fn particle_translucency_term(weight:f32,irradiance:vec3<f32>,normal:vec3<f32>,light:vec3<f32>,view:vec3<f32>)->vec3<f32> {
    let parameters=particle_material.translucency_phase;
    let bent=light+parameters.x*normal;
    let phase=exp2(shadow_saturate(-dot(view,bent))*parameters.y-parameters.y);
    let intensity=parameters.z*weight*irradiance;
    let maximum=max(max(intensity.x,intensity.y),intensity.z);
    let t=parameters.w+particle_material.translucency_ramp[0].w*maximum*phase;
    let outer=mix(particle_material.translucency_ramp[2].rgb,vec3(1.0),shadow_saturate((t-0.9)/0.1));
    let medium=mix(particle_material.translucency_ramp[1].rgb,outer,shadow_saturate((t-0.7)/0.2));
    let inner=mix(particle_material.translucency_ramp[0].rgb,medium,shadow_saturate((t-0.4)/0.3));
    return intensity*mix(vec3(0.0),inner,shadow_saturate(t/0.4));
}
fn particle_pixel_ward(surface:WardSurface,light:WardLight,geometric_normal:vec3<f32>,translucency:f32,fuzz_mask:f32)->DirectLighting {
    var result:DirectLighting;
    if particle_material.pixel_options.x!=0u {
        let lookup=textureSampleLevel(particle_diffuse_lookup,linear_clamp,
            vec2(shadow_saturate(dot(light.direction,surface.normal)),0.0),0.0).rgb;
        result=ward_direct_lookup(surface,light,lookup);
    } else {result=ward_direct(surface,light);}
    if light.attenuation>0.0 {
        let irradiance=light.shadow*(light.attenuation*light.color);
        if particle_material.pixel_effects.x!=0u {
            let vn=shadow_saturate(dot(surface.view,surface.normal));
            let ln=shadow_saturate(dot(light.direction,surface.normal));
            let fuzz=pow(1.0-vn,particle_material.fuzz.x)*particle_material.fuzz.y*fuzz_mask;
            result.specular+=irradiance*fuzz*ln;
        }
        if particle_material.pixel_effects.z!=0u {
            let cosine=-dot(light.direction,geometric_normal);
            let weight=smoothstep(-0.2,0.1,cosine)*translucency;
            let transmitted=ward_fresnel(surface.ior,shadow_saturate(cosine),surface.fresnel_strength);
            let halfway=normalize(surface.view+light.direction);
            let hn=shadow_saturate(dot(halfway,surface.normal));
            let m=shadow_saturate(1.0-hn);let m2=m*m;
            let diffuse=mix(surface.diffuse,surface.sheen_color,surface.roughness*pow(m2*m2*m,surface.sheen_importance));
            let rho=vec3(1.0)-(vec3(1.0)-transmitted)*diffuse;
            result.diffuse+=(1.0/3.14159265358979323846)*rho*
                particle_translucency_term(weight,irradiance,geometric_normal,light.direction,surface.view);
        }
    }
    return result;
}
fn particle_pixel_lights(surface:WardSurface,position:vec3<f32>,geometric_normal:vec3<f32>,pixel:vec4<f32>,translucency:f32,fuzz_mask:f32,self_shadow:ParticleSelfShadow)->DirectLighting {
    // Reconstruct wsPositionHacked from the actual projected fragment, as
    // particles::Lighting does before IntegrateLighting(HIGH_QUALITY).
    let ndc=vec2(2.0*pixel.x/particle_view.exposure_extent.z-1.0,
        1.0-2.0*pixel.y/particle_view.exposure_extent.w);
    let homogeneous=particle_view.game_world_from_clip*vec4(ndc,pixel.z,1.0);
    let hacked=homogeneous.xyz/homogeneous.w;
    let host_hacked=vec3(hacked.x,hacked.z,-hacked.y);
    let host_normal=vec3(geometric_normal.x,geometric_normal.z,-geometric_normal.y);
    let route=vec4(particle_material.shadow_options.z,particle_material.shadow_options.x,
        particle_material.shadow_options.y,2u);
    let directional=parallel_depth_visibility(host_hacked,host_normal,pixel.xy,route);
    var result=particle_pixel_ward(surface,WardLight(particle_view.sun_direction.xyz,1.0,
        particle_view.sun_colour.rgb,directional,1.0),geometric_normal,translucency,fuzz_mask);
    let kinds=array(1u,0u,2u,3u);
    for(var type_index=0u;type_index<4u;type_index++) {
        let kind=kinds[type_index];
        let head=light_head_coordinate(vec2<u32>(pixel.xy),kind,particle_view.scene_grid.x,particle_view.scene_grid.y);
        var list=begin_light_list(particle_light_heads[head.y*(particle_view.scene_grid.y*4u)+head.x]);
        while has_light(list,32u) {
            let index=advance_light(&list,particle_light_pool[list.head]);
            let record=particle_lights[index];
            if kind==3u {
                // The world-view collector331cc0 excludes area shadow sides.
                // Its prepared packets carry the unshadowed FFFFFFFF setup.
                let visibility=particle_directional_occlusion(normalize(record.position-position),self_shadow);
                let area=area_ward(surface,position,record,visibility,particle_area_pool,particle_trilinear);
                result.diffuse+=area.lighting.diffuse;result.specular+=area.lighting.specular;
                // BRDF_Ward's area method adds translucency, but no fuzz lobe.
                if particle_material.pixel_effects.z!=0u {
                    let cosine=-dot(area.direction,geometric_normal);
                    let weight=smoothstep(-0.2,0.1,cosine)*translucency;
                    let transmitted=ward_fresnel(surface.ior,shadow_saturate(cosine),surface.fresnel_strength);
                    let rho=vec3(1.0)-(vec3(1.0)-transmitted)*surface.diffuse;
                    result.diffuse+=(1.0/3.14159265358979323846)*rho*
                        particle_translucency_term(weight,area.irradiance,geometric_normal,area.direction,surface.view);
                }
            } else {
                let delta=record.position-position;
                let distance=length(delta);
                let to_light=delta/distance;
                let decay=mix(1.0,1.0/distance,record.decay);
                var attenuation=decay*decay*smoothstep(record.cutoff,record.falloff,distance);
                if kind==2u {attenuation*=1.0-shadow_saturate(dot(to_light,record.direction)*record.spot.x+record.spot.y);}
                else if kind==1u {
                    let angle=dot(to_light,-record.direction);
                    if angle<=0.0 {attenuation=0.0;}
                    else {attenuation*=1.0-shadow_saturate(angle*record.spot.x+record.spot.y);}
                }
                let visibility=particle_local_shadow(record,kind,hacked,surface.normal,to_light,attenuation,2)*
                    particle_directional_occlusion(to_light,self_shadow);
                let lighting=particle_pixel_ward(surface,WardLight(to_light,attenuation,record.color,visibility,record.specular),
                    geometric_normal,translucency,fuzz_mask);
                result.diffuse+=lighting.diffuse;result.specular+=lighting.specular;
            }
        }
    }
    return result;
}
