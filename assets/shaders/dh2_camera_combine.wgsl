@group(0) @binding(0) var<uniform> camera_p:CameraCombineParameters;
@group(0) @binding(1) var camera_full:texture_2d<f32>;
@group(0) @binding(2) var camera_depth:texture_2d<f32>;
@group(0) @binding(3) var camera_far:texture_2d<f32>;
@group(0) @binding(4) var camera_near_alpha:texture_2d<f32>;
@group(0) @binding(5) var camera_near:texture_2d<f32>;
@group(0) @binding(6) var camera_diffuse:texture_2d<f32>;
@group(0) @binding(7) var camera_focus:texture_2d<f32>;
@group(0) @binding(8) var camera_linear:sampler;
@group(0) @binding(9) var camera_point:sampler;
@group(0) @binding(10) var<storage,read> camera_adaptation:array<vec4<f32>>;
struct CameraVertex {@builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>}
@vertex fn camera_vertex(@builtin(vertex_index)i:u32)->CameraVertex {
    let uv=vec2(f32((i<<1u)&2u),f32(i&2u));
    return CameraVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv*camera_p.centre_resolution.zw);
}
@fragment fn camera_fragment(in:CameraVertex)->@location(0) vec4<f32> {
    let p=camera_p;var uv=in.uv;var outside=false;
    if (p.options.x&1u)!=0u {
        let distorted=camera_distort(uv/p.centre_resolution.zw,p.centre_resolution.xy,p.distortion.x,p.distortion.y);
        outside=any(distorted<vec2(0.0))||any(distorted>vec2(1.0));
        uv=clamp(distorted,vec2(0.0),vec2(1.0))*p.centre_resolution.zw;
    }
    let luminance=camera_adaptation[p.options.w*2u].x;
    let expansion=select(1.0,luminance*p.scattering_white.w,p.options.y!=0u);
    var colour=point_clamp_sample(camera_full,uv).rgb;
    if p.options.z!=0u {colour=colour/(p.units.x*expansion);}
    if outside {colour=vec3(0.0);}
    let depth=point_clamp_sample(camera_depth,uv).x;
    var focus=0.0;
    if (p.options.x&16u)!=0u {focus=point_clamp_sample(camera_focus,vec2(0.5)).x;}
    let far_weight=clamp(camera_fuzziness(depth,p,focus)*2.0,0.0,1.0);
    if (p.options.x&2u)!=0u {
        var low=textureSampleLevel(camera_diffuse,camera_linear,uv,0.0).rgb;
        if outside {low=vec3(0.0);}
        colour=camera_sharpen(colour,low,colour*expansion,p.scattering_white.xyz,p.distortion.z);
    }
    if (p.options.x&32u)!=0u && !outside {
        let near=textureSampleLevel(camera_near,camera_linear,uv,0.0).rgb;
        let alpha=textureSampleLevel(camera_near_alpha,camera_linear,uv,0.0).x;
        let square=alpha*alpha;colour=mix(colour,near,square*square);
    }
    var far=textureSampleLevel(camera_far,camera_linear,uv,0.0).rgb;
    if outside {far=vec3(0.0);}
    colour=mix(colour,far,far_weight);
    if (p.options.x&8u)!=0u {
        let grey=clamp(dot(colour,vec3(0.2989,0.5870,0.1140)),0.0,1.0);
        colour=camera_noise(colour,uv/p.centre_resolution.zw,p.noise,grey,p.inverse_z_time.w);
    }
    if (p.options.x&4u)!=0u {colour*=camera_vignette(in.uv/p.centre_resolution.zw,p.centre_resolution.xy,p.vignette);}
    if p.options.z!=0u {colour*=p.units.x*expansion;}
    return vec4(colour,1.0);
}
