// PostFX_uberMotionVector: five authored FSdisto contributions to the shared
// motion field. Compiled wobble/camera PS checksum 935452d7cb0fe0552975f05940d5e5c3.
struct FullscreenDistortionParameters {
    inverse_current:mat4x4<f32>,
    previous:mat4x4<f32>,
    current:mat4x4<f32>,
    camera_forward:vec4<f32>,
    // Vector-field extent, view time (group1), localTime.
    extent_time:vec4<f32>,
    inverse_z:vec4<f32>,
    // Low bound, high bound, speed, strength.
    wobble:vec4<f32>,
    // Radial blur dead zone, blur strength, distortion strength, lens strength.
    radial_lens:vec4<f32>,
    // Sand dead zone, start depth, end depth, unused.
    sand_depth:vec4<f32>,
    sand_start:vec4<f32>,
    sand_end:vec4<f32>,
    wind_camera_strength:vec4<f32>,
    camera_depth:vec4<f32>,
    // radial, lens, wobble, sand/camera bits.
    options:vec4<u32>,
}
@group(0) @binding(0) var<uniform> fs:FullscreenDistortionParameters;
@group(0) @binding(1) var fs_depth:texture_depth_2d;
@group(0) @binding(2) var fs_linear_clamp:sampler;

@vertex fn fullscreen_distortion_vertex(@builtin(vertex_index) index:u32)->@builtin(position) vec4<f32> {
    let uv=vec2(f32((index<<1u)&2u),f32(index&2u));
    return vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0);
}
@fragment fn fullscreen_distortion_fragment(@builtin(position) pixel:vec4<f32>)->@location(0) vec4<f32> {
    // bevy_backend: viewport scratch is tightly allocated (resolutionScale=1),
    // retaining original vector-field pixel coordinates and full-depth UVs.
    let size=fs.extent_time.xy;
    let texel=1.0/size;
    let center=size*0.5;
    let uv=pixel.xy*texel;
    let ratio=(16.0/9.0)/(size.x/size.y);
    let resolution_factor=max(texel.x*ratio,texel.y*ratio);
    let pixel_factor=450.0*resolution_factor;
    let delta=pixel.xy-center;
    let radial_uv=delta*texel;
    var result=vec4(0.0);
    var projected_z=0.0;
    var linear_z=0.0;
    if fs.options.w!=0u {
        projected_z=textureSampleLevel(fs_depth,fs_linear_clamp,uv,0.0);
        linear_z=-fs.inverse_z.z/(fs.inverse_z.x*projected_z+fs.inverse_z.y);
    }
    if (fs.options.w&1u)!=0u {
        let wind=fs.wind_camera_strength.xyz;
        let wind_length=length(wind);
        var direction=wind;
        if wind_length>0.0 {direction=wind/wind_length;}
        let projected=fs.current*vec4(direction+fs.camera_forward.xyz*0.01,0.0);
        let direction_uv=projected.xy/vec2(projected.w,-projected.w);
        let storm_center=(0.5*direction_uv+vec2(0.5))/texel;
        var storm_vector=(pixel.xy-storm_center)*texel;
        storm_vector=resolution_factor*(-sign(dot(direction,fs.camera_forward.xyz)))*normalize(storm_vector);
        let dead_zone=fs.sand_depth.x/pixel_factor;
        storm_vector*=smoothstep(0.0,dead_zone,length(pixel.xy-storm_center));
        let depth_factor=clamp((linear_z-fs.sand_depth.y)/(fs.sand_depth.z-fs.sand_depth.y),0.0,1.0);
        let amount=mix(fs.sand_start,fs.sand_end,vec4(depth_factor));
        let noise=abs(fract(sin(dot(uv+vec2(fract(fs.extent_time.w)),vec2(12.9898,78.233)))*43758.5453));
        result+=amount*(storm_vector*resolution_factor*noise).xyxy;
    }
    if (fs.options.w&2u)!=0u {
        let current_position=vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),projected_z,1.0);
        var world=fs.inverse_current*current_position;
        world/=world.w;
        var previous=fs.previous*world;
        previous/=abs(previous.w);
        var velocity=clamp((current_position.xy-previous.xy)*vec2(-0.5,0.5),vec2(-1.0),vec2(1.0));
        let depth_factor=clamp((linear_z-fs.camera_depth.x)/(fs.camera_depth.y-fs.camera_depth.x),0.0,1.0);
        velocity*=texel*fs.wind_camera_strength.w*depth_factor;
        result+=vec4(velocity,0.0,0.0);
    }
    if fs.options.x!=0u || fs.options.z!=0u {
        let radial=-normalize(radial_uv)*resolution_factor;
        if fs.options.x!=0u {
            let dead_zone=fs.radial_lens.x/pixel_factor;
            let distortion=radial*clamp(length(delta)-dead_zone,0.0,1.0/resolution_factor)*resolution_factor;
            result+=vec4(fs.radial_lens.y*distortion,fs.radial_lens.z*distortion);
        }
        if fs.options.z!=0u {
            let phase=fs.wobble.w*mix(fs.wobble.x,fs.wobble.y,sin(fs.extent_time.z*fs.wobble.z)*0.5+0.5);
            // wobbleDeadZone is not read by the installed shader.
            var dead_zone=clamp(length(radial_uv)-0.15,0.0,sqrt(0.5))/sqrt(0.5);
            dead_zone*=dead_zone;
            let distortion=radial*phase*dead_zone/resolution_factor;
            result+=vec4(0.0,0.0,distortion);
        }
    }
    if fs.options.y!=0u {
        let radius=250.0/pixel_factor;
        let transition=100.0/pixel_factor;
        let main_width=40.0/pixel_factor;
        let secondary_width=8.0/pixel_factor;
        let to_center=center-pixel.xy;
        let distance=length(to_center);
        var lens=vec4(0.0);
        if distance<radius {
            let factor=(cos(clamp(distance-(radius-transition),0.0,transition)/transition*3.141592)+1.0)/2.0;
            lens+=vec4(0.0,0.0,to_center*texel*factor*(0.05*fs.radial_lens.w));
        }
        let ripple_direction=vec2(to_center.y,-to_center.x);
        if distance>radius-main_width && distance<radius {
            let factor=(-cos(clamp(distance-(radius-main_width),0.0,main_width)/main_width*2.0*3.141592)+1.0)/2.0;
            let ripple=normalize(ripple_direction)*factor*(0.005*fs.radial_lens.w);
            lens+=ripple.xyxy;
        }
        if distance>radius-secondary_width && distance<radius {
            let factor=(-cos(clamp(distance-(radius-secondary_width),0.0,secondary_width)/secondary_width*2.0*3.141592)+1.0)/2.0;
            let ripple=normalize(ripple_direction)*factor*(-0.02*fs.radial_lens.w);
            lens+=ripple.xyxy;
        }
        result+=lens;
    }
    if abs(dot(result,vec4(1.0)))==0.0 {discard;}
    return result;
}
