#define_import_path dh2_local_receiver
#import dh2_shadow::{omni_shadow_side,spot_shadow_side,shadow_offset,sample_local_shadow}

@group(#{MATERIAL_BIND_GROUP}) @binding(126) var<storage,read> local:array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(127) var local_depth:texture_depth_2d_array;
@group(#{MATERIAL_BIND_GROUP}) @binding(128) var local_comparison:sampler_comparison;

fn local_vector(offset:u32)->vec4<f32> {
    return bitcast<vec4<f32>>(vec4(local[offset],local[offset+1u],local[offset+2u],local[offset+3u]));
}

// lightcontext::ComputeShadow uses wsNormal for local lights, m_wsToLight
// for point/half-point and m_SpotDir for spots. The caller supplies the
// material's actual compile options and HIGH/AVERAGE/LOW quality values.
fn local_visibility(light_id:u32,viewer_position:vec3<f32>,viewer_normal:vec3<f32>,route:vec4<u32>)->f32 {
    if local[0]==0u || light_id>=local[1] || route.x==3u {return 1.0;}
    let record=16u+16u*light_id;
    let first=local[record+1u];
    if first==0xffffffffu {return 1.0;}
    let kind=local[record];let count=local[record+2u];
    let position=vec3(viewer_position.x,-viewer_position.z,viewer_position.y);
    let normal=vec3(viewer_normal.x,-viewer_normal.z,viewer_normal.y);
    let to_light=local_vector(record+4u).xyz-position;
    // lightcontext::Init normalizes m_wsToLight before ComputeShadow.
    let ws_to_light=to_light*(1.0/length(to_light));
    var side=0u;var light_dir=ws_to_light;
    if kind<2u {side=omni_shadow_side(ws_to_light);}
    else {
        light_dir=local_vector(record+12u).xyz;
        if count>1u {side=spot_shadow_side(ws_to_light,local_vector(record+8u).xyz);}
    }
    let shadow=16u+16u*local[1]+36u*(first+side);
    let matrix=mat4x4<f32>(local_vector(shadow),local_vector(shadow+4u),
        local_vector(shadow+8u),local_vector(shadow+12u));
    let bias=local_vector(8u);
    let coordinates=shadow_offset(matrix,position,normal,light_dir,false,
        (route.y&1u)!=0u,(route.y&2u)!=0u,(route.y&4u)!=0u,(route.y&8u)!=0u,
        bias.y,bias.x,bias.z);
    return sample_local_shadow(local_depth,local_comparison,coordinates,
        i32(local[shadow+32u]&31u),local_vector(4u).xy,i32(route.w),i32(route.z));
}
