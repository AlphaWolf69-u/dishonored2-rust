// particle/add_default's unlit sprite path. Shared material fog is appended by
// the backend after replacing its material-group binding placeholder.
struct ParticleViewParameters {
    clip_from_game_world:mat4x4<f32>,
    camera_time:vec4<f32>,
    inverse_z:vec4<f32>,
    exposure_extent:vec4<f32>,
}
struct ParticleFogView {world_position:vec3<f32>,exposure:f32,}
struct ParticleInstance {
    center:vec4<f32>,
    axis_y:vec4<f32>,
    axis_z:vec4<f32>,
    parameters:vec4<u32>,
    clip_plane:vec4<f32>,
    clip_fade:vec4<f32>,
}
struct ParticleGridVertex {position:vec4<f32>,uv:vec4<f32>,}
@group(0) @binding(0) var<uniform> particle_view:ParticleViewParameters;
@group(0) @binding(1) var<storage,read> particle_instances:array<ParticleInstance>;
@group(0) @binding(2) var<storage,read> particle_grid:array<ParticleGridVertex>;
@group(0) @binding(3) var<storage,read> particle_adaptation:array<vec4<f32>,2>;
// Shared fog expects host coordinates for this camera transport.
@group(0) @binding(4) var<uniform> view:ParticleFogView;

struct ParticleTextureLayer {tiling_pan:vec4<f32>,strength:vec4<f32>,rotation_seed:vec4<f32>,}
struct ParticleMaterialParameters {
    layers:array<ParticleTextureLayer,3>,
    colour_adjust:vec4<f32>,
    depth_fade_lookup:vec4<f32>,
    radial_exposure_fog:vec4<f32>,
    distort_options:vec4<u32>,
    colour_options:vec4<u32>,
    fade_options:vec4<u32>,
}
@group(1) @binding(0) var<uniform> particle_material:ParticleMaterialParameters;
@group(1) @binding(1) var particle_opacity:texture_2d<f32>;
@group(1) @binding(2) var particle_distortion_one:texture_2d<f32>;
@group(1) @binding(3) var particle_distortion_two:texture_2d<f32>;
@group(1) @binding(4) var particle_diffuse:texture_2d<f32>;
@group(1) @binding(5) var particle_lookup:texture_2d<f32>;
@group(1) @binding(6) var particle_trilinear:sampler;
@group(1) @binding(7) var particle_surface_sampler:sampler;
@group(1) @binding(8) var linear_clamp:sampler;

struct ParticleFragment {
    @builtin(position) position:vec4<f32>,
    @location(0) texture_coordinates:vec4<f32>,
    @location(1) colour:vec4<f32>,
    @location(2) world_position:vec3<f32>,
    @location(3) fog:vec4<f32>,
    @location(4) @interpolate(flat) clip_plane:vec4<f32>,
    @location(5) @interpolate(flat) fade_radius:vec2<f32>,
}
@vertex fn particle_vertex(@builtin(vertex_index) vertex:u32,
    @builtin(instance_index) instance_index:u32)->ParticleFragment {
    let instance=particle_instances[instance_index];
    let grid=particle_grid[vertex];
    let position=instance.center.xyz+instance.axis_y.xyz*grid.position.y+instance.axis_z.xyz*grid.position.z;
    let clip=particle_view.clip_from_game_world*vec4(position,1.0);
    let word=instance.parameters.y;
    var uv=grid.uv.xy;
    uv.x=uv.x*select(1.0,-1.0,(word&(1u<<30u))!=0u)+select(0.0,1.0,(word&(1u<<30u))!=0u);
    uv.y=uv.y*select(1.0,-1.0,(word&(1u<<31u))!=0u)+select(0.0,1.0,(word&(1u<<31u))!=0u);
    var output:ParticleFragment;
    output.position=clip;
    output.texture_coordinates=vec4(uv,unpack2x16float(word&0xffffu).x,
        f32((word>>16u)&0x3fffu)*(1.0/16383.0));
    output.colour=unpack4x8unorm(instance.parameters.x);
    output.world_position=position;
    output.fog=material_fog_vertex(clip,vec3(position.x,position.z,-position.y),false,instance.parameters.w);
    output.clip_plane=instance.clip_plane;
    output.fade_radius=vec2(instance.clip_fade.x,unpack2x16float(instance.parameters.z).x);
    return output;
}
fn particle_pan(coordinates:vec4<f32>,layer:ParticleTextureLayer)->vec2<f32> {
    let time=coordinates.w*layer.rotation_seed.y+particle_view.camera_time.w;
    let angle=time*layer.rotation_seed.x;
    let c=cos(angle);let s=sin(angle);
    let uv=vec2((coordinates.x-0.5)*c+(coordinates.y-0.5)*s+0.5,
        -(coordinates.x-0.5)*s+(coordinates.y-0.5)*c+0.5);
    return uv*layer.tiling_pan.xy+time*layer.tiling_pan.zw;
}
@fragment fn particle_fragment(input:ParticleFragment)->@location(0) vec4<f32> {
    let material=particle_material;
    let coordinates=input.texture_coordinates;
    var distorted=vec3(0.0);
    if material.distort_options.x!=0u {
        var one=textureSample(particle_distortion_one,particle_trilinear,particle_pan(coordinates,material.layers[0])).xy;
        distorted.z=one.x;
        if material.distort_options.z!=0u {one.y=one.x;}
        distorted=vec3(one*material.layers[0].strength.xy,distorted.z);
        if material.distort_options.y!=0u {
            var two=textureSample(particle_distortion_two,particle_trilinear,particle_pan(coordinates,material.layers[1])).xy;
            let mask=two.x;
            if material.distort_options.w!=0u {two.y=two.x;}
            if material.colour_options.x!=0u {
                distorted=vec3(distorted.xy*two*material.layers[1].strength.xy,distorted.z*mask);
            } else {distorted+=vec3(two*material.layers[1].strength.xy,mask);}
        }
    }
    distorted=vec3(distorted.xy+coordinates.xy,clamp(distorted.z,0.0,1.0));
    let opacity_layer=material.layers[2];
    let angle=particle_view.camera_time.w*opacity_layer.rotation_seed.x;
    let c=cos(angle);let s=sin(angle);
    let opacity_uv=vec2((distorted.x-0.5)*c+(distorted.y-0.5)*s+0.5,
        -(distorted.x-0.5)*s+(distorted.y-0.5)*c+0.5)*opacity_layer.tiling_pan.xy+
        opacity_layer.tiling_pan.zw*particle_view.camera_time.w;
    var colour=vec4(1.0);
    if material.colour_options.w!=0u {
        let delta=coordinates.xy+distorted.xy-vec2(1.0);
        colour=vec4(pow(clamp(1.0-dot(delta,delta),0.0,1.0),material.radial_exposure_fog.x));
    } else {
        colour.a=textureSample(particle_opacity,particle_surface_sampler,opacity_uv).x;
        if material.fade_options.x!=0u {colour.a*=distorted.z;}
        if material.colour_options.y!=0u {colour=vec4(textureSample(particle_diffuse,particle_surface_sampler,opacity_uv).rgb,colour.a);}
        if material.colour_options.z!=0u {
            let modulate=select(input.colour.a,1.0,material.fade_options.w!=0u);
            let lookup=vec2(pow(colour.a,material.depth_fade_lookup.w)*modulate,1.0);
            colour=vec4(colour.rgb*textureSample(particle_lookup,linear_clamp,lookup).rgb,colour.a);
        }
        if colour.a<0.5/255.0 {discard;}
    }
    colour=vec4(colour.rgb*input.colour.rgb*material.colour_adjust.rgb*material.colour_adjust.a,colour.a*input.colour.a);
    let distance=1.0/input.position.w;
    if material.fade_options.y!=0u {
        colour.a*=smoothstep(material.depth_fade_lookup.y,material.depth_fade_lookup.z,distance);
    }
    if material.fade_options.z!=0u {
        let uv=input.position.xy/particle_view.exposure_extent.zw;
        let depth=textureSampleLevel(material_fog_depth,linear_clamp,uv,0.0);
        let linear=-particle_view.inverse_z.z/(particle_view.inverse_z.x*depth+particle_view.inverse_z.y);
        var difference=max(0.0,linear-distance);
        if material.fade_options.z==2u {difference=(linear-distance)/input.fade_radius.y;}
        colour.a*=clamp(difference/material.depth_fade_lookup.x,0.0,1.0);
    }
    let plane_dot=dot(input.clip_plane,vec4(input.world_position,1.0));
    if plane_dot<0.0 {discard;}
    if input.fade_radius.x>0.0 {
        colour.a*=smoothstep(0.0,input.fade_radius.x,plane_dot);
    }
    let exposure=mix(1.0,1.0/particle_adaptation[0].x,material.radial_exposure_fog.y);
    var rgb=colour.rgb*exposure*particle_view.exposure_extent.x;
    let fogged=material_fog_composite(rgb,input.fog);
    rgb=mix(rgb,fogged,material.radial_exposure_fog.z);
    return vec4(rgb,colour.a);
}
