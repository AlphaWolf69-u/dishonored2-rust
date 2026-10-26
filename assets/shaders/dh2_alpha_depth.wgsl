#import bevy_pbr::forward_io::VertexOutput
#import dh2_alpha_clip::clip_surface
#import dh2_surface::{surface,compiled_misc,evaluate_surface,unpack_bc5_normal_map}
#import bevy_pbr::mesh_view_bindings::view
// acos lowering read from installed PC pixel stage 8905; exact constant words.
fn glass_acos(value:f32)->f32 {
    let x=clamp(value,-1.0,1.0);let a=abs(x);
    var p=bitcast<f32>(0xbc996e30u)*a+bitcast<f32>(0x3d981627u);
    p=p*a+bitcast<f32>(0xbe593484u);
    p=(p*a+bitcast<f32>(0x3fc90da4u))*sqrt(1.0-a);
    return select(p,bitcast<f32>(0x40490fdbu)-p,x<0.0);
}
fn glass_distortion(v:VertexOutput,is_front:bool)->f32 {
    let face_factor=select(1.0,-1.0,!is_front && (compiled_misc.w&4u)!=0u);
    var normal=normalize(v.world_normal);
#ifdef VERTEX_TANGENTS
    if compiled_misc.x!=0u {
        var uv0=vec2(0.0);var uv1=vec2(0.0);
#ifdef VERTEX_UVS_A
        uv0=v.uv;
#endif
#ifdef VERTEX_UVS_B
        uv1=v.uv_b;
#endif
        var colour=vec4(1.0);
#ifdef VERTEX_COLORS
        colour=v.color;
#endif
        let authored=evaluate_surface(uv0,uv1,colour);
        let bump=unpack_bc5_normal_map(authored.normal_xy);
        let t=v.world_tangent;
#ifdef DH2_AUTHORED_BASIS
        let b=v.world_bitangent;
#else
        let b=cross(v.world_normal,t.xyz)*t.w;
#endif
        normal=normalize(bump.x*t.xyz+bump.y*b+bump.z*v.world_normal);
    }
#endif
    if face_factor<0.0 {return 0.0;}
    let delta=view.world_position-v.world_position.xyz;
    let length_v=length(delta);
    let eye=delta*select(0.0,1.0/length_v,length_v>0.0);
    let ray=refract(-eye,normal,surface.glass.x);
    let view_ray=(view.view_from_world*vec4(ray,0.0)).xyz;
    let angle=glass_acos(-view_ray.z);
    var strength=surface.glass.y*(0.5+0.5*(1.0-v.position.z));
    if compiled_misc.x!=0u {
        let mask=clamp(surface.glass.z/dot(normal,normalize(v.world_normal)),0.0,1.0);
        strength*=mask*mask*(3.0-2.0*mask);
    }
    let unit=fract(angle*bitcast<f32>(0x3e22f983u));
    let packed=u32(unit*4095.0+0.5)|(u32(round(strength))<<12u);
    return unpack2x16float(packed).x;
}
fn clip_vertex(vertex_output:VertexOutput) {
    var uv0=vec2(0.0);var uv1=vec2(0.0);
#ifdef VERTEX_UVS_A
    uv0=vertex_output.uv;
#endif
#ifdef VERTEX_UVS_B
    uv1=vertex_output.uv_b;
#endif
    clip_surface(uv0,uv1,vertex_output.world_position);
}

// Nonrefractive alpha slots disable colour writes; the value is unobservable.
@fragment fn full(vertex_output:VertexOutput,@builtin(front_facing) is_front:bool)->@location(0) vec2<f32> {
    clip_vertex(vertex_output);
#ifdef DH2_REFRACTION
    return vec2(0.0,glass_distortion(vertex_output,is_front));
#else
    return vec2(0.0);
#endif
}
@fragment fn reduced(vertex_output:VertexOutput) {clip_vertex(vertex_output);}
