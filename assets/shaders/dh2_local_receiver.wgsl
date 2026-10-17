#define_import_path dh2_local_receiver
#import dh2_shadow::{native_omni_shadow_side,native_spot_shadow_side,native_shadow_offset,native_sample_local_shadow}

@group(#{MATERIAL_BIND_GROUP}) @binding(126) var<storage,read> native_local:array<u32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(127) var native_local_depth:texture_depth_2d_array;
@group(#{MATERIAL_BIND_GROUP}) @binding(128) var native_local_comparison:sampler_comparison;

fn native_local_vector(offset:u32)->vec4<f32> {
    return bitcast<vec4<f32>>(vec4(native_local[offset],native_local[offset+1u],native_local[offset+2u],native_local[offset+3u]));
}

// lightcontext::ComputeShadow uses wsNormal for local lights, m_wsToLight
// for point/half-point and m_SpotDir for spots. The caller supplies the
// material's actual compile options and HIGH/AVERAGE/LOW quality values.
fn native_local_visibility(light_id:u32,viewer_position:vec3<f32>,viewer_normal:vec3<f32>,route:vec4<u32>)->f32 {
    if native_local[0]==0u || light_id>=native_local[1] || route.x==3u {return 1.0;}
    let record=16u+16u*light_id;
    let first=native_local[record+1u];
    if first==0xffffffffu {return 1.0;}
    let kind=native_local[record];let count=native_local[record+2u];
    let position=vec3(viewer_position.x,-viewer_position.z,viewer_position.y);
    let normal=vec3(viewer_normal.x,-viewer_normal.z,viewer_normal.y);
    let to_light=native_local_vector(record+4u).xyz-position;
    // lightcontext::Init normalizes m_wsToLight before ComputeShadow.
    let ws_to_light=to_light*(1.0/length(to_light));
    var side=0u;var light_dir=ws_to_light;
    if kind<2u {side=native_omni_shadow_side(ws_to_light);}
    else {
        light_dir=native_local_vector(record+12u).xyz;
        if count>1u {side=native_spot_shadow_side(ws_to_light,native_local_vector(record+8u).xyz);}
    }
    let shadow=16u+16u*native_local[1]+36u*(first+side);
    let matrix=mat4x4<f32>(native_local_vector(shadow),native_local_vector(shadow+4u),
        native_local_vector(shadow+8u),native_local_vector(shadow+12u));
    let bias=native_local_vector(8u);
    let coordinates=native_shadow_offset(matrix,position,normal,light_dir,false,
        (route.y&1u)!=0u,(route.y&2u)!=0u,(route.y&4u)!=0u,(route.y&8u)!=0u,
        bias.y,bias.x,bias.z);
    return native_sample_local_shadow(native_local_depth,native_local_comparison,coordinates,
        i32(native_local[shadow+32u]&31u),native_local_vector(4u).xy,i32(route.w),i32(route.z));
}
