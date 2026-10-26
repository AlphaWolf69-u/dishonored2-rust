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
    to_light:vec3<f32>,attenuation:f32)->f32 {
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
        local_vector(4u).xy,0,i32(particle_material.shadow_options.y)-1);
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
            let visibility=particle_local_shadow(record,kind,position,normal,to_light,attenuation);
            let colour=record.color*visibility*attenuation;
            let w=particle_basis_weights(basis,to_light);
            colours+=mat3x3(colour*w.x,colour*w.y,colour*w.z);
        }
    }
    return colours;
}
