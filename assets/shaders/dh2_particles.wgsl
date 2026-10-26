// particle/add_default sprite colour. Shared material fog is appended by
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
struct ParticleTransparencyParameters {weights:vec4<f32>,parameters:vec4<f32>,mode:vec4<u32>,}
@group(0) @binding(5) var<storage,read> particle_transparency:array<ParticleTransparencyParameters>;
@group(0) @binding(6) var<storage,read> particle_root_lighting:array<array<vec4<f32>,9>>;

struct ParticleTextureLayer {tiling_pan:vec4<f32>,strength:vec4<f32>,rotation_seed:vec4<f32>,}
struct ParticleMaterialParameters {
    layers:array<ParticleTextureLayer,4>,
    colour_adjust:vec4<f32>,
    depth_fade_lookup:vec4<f32>,
    radial_exposure_fog:vec4<f32>,
    distort_options:vec4<u32>,
    colour_options:vec4<u32>,
    fade_options:vec4<u32>,
    animation_options:vec4<u32>,
    surface_options:vec4<u32>,
    macro_fresnel:vec4<f32>,
    fresnel_uv:vec4<f32>,
    lighting_parameters:vec4<f32>,
    lighting_dielectric:vec4<f32>,
    lighting_metal_diffuse:vec4<f32>,
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
@group(1) @binding(9) var particle_animation:texture_3d<f32>;
@group(1) @binding(10) var particle_scroll_mask:texture_2d<f32>;
@group(1) @binding(11) var particle_macro_uv:texture_2d<f32>;
@group(1) @binding(12) var particle_fresnel_normal:texture_2d<f32>;
@group(1) @binding(13) var particle_fresnel_lookup:texture_2d<f32>;
@group(1) @binding(14) var particle_bilinear:sampler;

struct ParticleFragment {
    @builtin(position) position:vec4<f32>,
    @location(0) texture_coordinates:vec4<f32>,
    @location(1) colour:vec4<f32>,
    @location(2) world_position:vec3<f32>,
    @location(3) fog:vec4<f32>,
    @location(4) @interpolate(flat) clip_plane:vec4<f32>,
    @location(5) @interpolate(flat) fade_radius:vec2<f32>,
    @location(6) @interpolate(flat) transparency_weights:vec4<f32>,
    @location(7) @interpolate(flat) transparency_parameters:vec4<f32>,
    @location(8) @interpolate(flat) transparency_mode:u32,
    @location(9) world_normal:vec3<f32>,
    @location(10) world_tangent:vec3<f32>,
    @location(11) indirect_irradiance:vec3<f32>,
    @location(12) reflection_gain:f32,
    @location(13) @interpolate(flat) lighting_record:u32,
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
    // Grid1dc200 has local +X normal and +Y tangent. vertex::TransformVertex
    // applies the instance matrix and normalizes both independently.
    output.world_normal=normalize(vec3(instance.center.w,instance.axis_y.w,instance.axis_z.w));
    output.world_tangent=normalize(instance.axis_y.xyz);
    if particle_material.animation_options.w>=2u {
        let root=particle_root_lighting[instance.parameters.w];
        let lighting=tetrahedron_lighting_game(u32(root[6].w),position);
        let direction=normalize(particle_view.camera_time.xyz-position);
        output.indirect_irradiance=irradiance(normalize(output.world_normal+direction),lighting.sh);
        output.reflection_gain=lighting.reflection_gain;
    }
    output.fog=material_fog_vertex(clip,vec3(position.x,position.z,-position.y),false,instance.parameters.w);
    output.clip_plane=instance.clip_plane;
    output.fade_radius=vec2(instance.clip_fade.x,unpack2x16float(instance.parameters.z).x);
    let transparency=particle_transparency[instance.parameters.w];
    output.transparency_weights=transparency.weights;
    output.transparency_parameters=transparency.parameters;
    output.transparency_mode=transparency.mode.x;
    output.lighting_record=instance.parameters.w;
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
struct ParticleColour {colour:vec4<f32>,albedo:vec3<f32>,}
fn particle_colour(input:ParticleFragment)->ParticleColour {
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
    let normal=normalize(input.world_normal);
    if material.surface_options.x!=0u {
        let tile=material.macro_fresnel.y;
        let a=textureSample(particle_macro_uv,particle_bilinear,input.world_position.yz*tile).xy*2.0-1.0;
        let b=textureSample(particle_macro_uv,particle_bilinear,input.world_position.zx*tile).xy*2.0-1.0;
        let c=textureSample(particle_macro_uv,particle_bilinear,input.world_position.xy*tile).xy*2.0-1.0;
        distorted=vec3(distorted.xy+(a*normal.x+b*normal.y+c*normal.z)*material.macro_fresnel.x,distorted.z);
    }
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
        if material.animation_options.x!=0u {
            // particles::TextureColor uses unrotated distorted XY for the
            // volume, while ordinary opacity/diffuse retain opacity rotation.
            let volume_uv=vec3(distorted.xy*opacity_layer.tiling_pan.xy+
                opacity_layer.tiling_pan.zw*particle_view.camera_time.w,coordinates.z);
            let animation=textureSample(particle_animation,particle_surface_sampler,volume_uv);
            colour=select(animation,vec4(1.0,1.0,1.0,animation.x),material.animation_options.y!=0u);
        } else {
            colour.a=textureSample(particle_opacity,particle_surface_sampler,opacity_uv).x;
        }
        if material.fade_options.x!=0u {colour.a*=distorted.z;}
        if material.colour_options.y!=0u {colour=vec4(textureSample(particle_diffuse,particle_surface_sampler,opacity_uv).rgb,colour.a);}
        if material.colour_options.z!=0u {
            let modulate=select(input.colour.a,1.0,material.fade_options.w!=0u);
            let lookup=vec2(pow(colour.a,material.depth_fade_lookup.w)*modulate,1.0);
            colour=vec4(colour.rgb*textureSample(particle_lookup,linear_clamp,lookup).rgb,colour.a);
        }
        if colour.a<0.5/255.0 {discard;}
        // add_default applies this after TextureColor's own low-alpha clip,
        // only in the branch that modulates lookup by the particle alpha.
        if material.fade_options.w==0u && material.animation_options.z!=0u {
            colour.a*=textureSample(particle_scroll_mask,particle_trilinear,
                particle_pan(coordinates,material.layers[3])).x;
        }
    }
    colour=vec4(colour.rgb*input.colour.rgb*material.colour_adjust.rgb*material.colour_adjust.a,colour.a*input.colour.a);
    let distance=1.0/input.position.w;
    if material.fade_options.y!=0u {
        colour.a*=smoothstep(material.depth_fade_lookup.y,material.depth_fade_lookup.z,distance);
    }
    if material.fade_options.z!=0u {
        let uv=input.position.xy/particle_view.exposure_extent.zw;
        let depth=textureSampleLevel(material_fog_depth,linear_clamp,uv,0);
        let linear=-particle_view.inverse_z.z/(particle_view.inverse_z.x*depth+particle_view.inverse_z.y);
        var difference=max(0.0,linear-distance);
        if material.fade_options.z==2u {difference=(linear-distance)/input.fade_radius.y;}
        colour.a*=clamp(difference/material.depth_fade_lookup.x,0.0,1.0);
    }
    if material.surface_options.y!=0u {
        var fresnel_normal=normal;
        if material.surface_options.z!=0u {
            let uv=coordinates.xy*material.fresnel_uv.xy+material.fresnel_uv.zw;
            let xy=textureSample(particle_fresnel_normal,particle_trilinear,uv).xy;
            // The same global.hlsl BC5 reconstruction used by dh2_surface.
            let ts=normalize(vec3(xy,1.001-clamp(dot(xy,xy),0.0,1.0)));
            let tangent=normalize(input.world_tangent);
            let bitangent=cross(normal,tangent);
            fresnel_normal=normalize(ts.x*tangent+ts.y*bitangent+ts.z*normal);
        }
        let view_direction=normalize(particle_view.camera_time.xyz-input.world_position);
        let cosine=clamp(dot(view_direction,fresnel_normal),0.0,1.0);
        var fresnel=min(cosine/material.macro_fresnel.z,1.0);
        if material.surface_options.w!=0u {
            fresnel=textureSample(particle_fresnel_lookup,linear_clamp,vec2(cosine)).x;
        }
        colour.a*=pow(fresnel,material.macro_fresnel.w);
    }
    let plane_dot=dot(input.clip_plane,vec4(input.world_position,1.0));
    if plane_dot<0.0 {discard;}
    if input.fade_radius.x>0.0 {
        colour.a*=smoothstep(0.0,input.fade_radius.x,plane_dot);
    }
    var lit_colour=colour.rgb;
    var albedo=colour.rgb;
    if material.animation_options.w==2u {
        let view_direction=normalize(particle_view.camera_time.xyz-input.world_position);
        let gloss=mix(material.lighting_parameters.y,
            mix(material.lighting_parameters.x,material.lighting_parameters.y,colour.a),
            material.lighting_parameters.w);
        let roughness=1.0-gloss;
        let mapped=material_lighting_values(colour.rgb,roughness,material.lighting_parameters.z,
            vec3(1.0),true,material.lighting_dielectric.rgb,material.lighting_metal_diffuse.rgb);
        let root=particle_root_lighting[input.lighting_record];
        let reflection=particle_lq_reflection(input.world_position,normal,view_direction,
            roughness*roughness,mapped.ior,mapped.fresnel_strength,root[0].w,input.reflection_gain);
        albedo*=1.0/3.14159265358979323846;
        lit_colour=albedo*input.indirect_irradiance+min(vec3(250.0),reflection);
    } else {
        let exposure=mix(1.0,1.0/particle_adaptation[0].x,material.radial_exposure_fog.y);
        lit_colour*=exposure;
    }
    var rgb=lit_colour*particle_view.exposure_extent.x;
    let fogged=material_fog_composite(rgb,input.fog);
    rgb=mix(rgb,fogged,material.radial_exposure_fog.z);
    return ParticleColour(vec4(rgb,colour.a),albedo);
}
@fragment fn particle_fragment(input:ParticleFragment)->@location(0) vec4<f32> {
    return particle_colour(input).colour;
}
struct ParticleAccumulation {@location(0) colour:vec4<f32>,@location(1) opacity:vec4<f32>,}
@fragment fn particle_accumulation_fragment(input:ParticleFragment)->ParticleAccumulation {
    let source=particle_colour(input);
    // Weighted transparency operates in the game's lighting units. The host
    // HDR transport's camera exposure and luminance scale are applied later.
    let colour=vec4(source.colour.rgb/particle_view.exposure_extent.x,source.colour.a);
    let weights=input.transparency_weights;
    let parameters=input.transparency_parameters;
    if input.transparency_mode==2u {
        let lum=mix(1.0,clamp(colour.g,0.0,1.0),parameters.y);
        let alpha=colour.a*lum;
        var weight=pow(1e-2+alpha,weights.x)*clamp(parameters.z/
            (1e-5+pow(input.position.w/weights.y,weights.z)),1e-2,3e3);
        weight*=weights.w;
        return ParticleAccumulation(min(vec4(6000.0),vec4(colour.rgb,parameters.w)*(weight*alpha)),
            vec4(alpha*parameters.w,0.0,0.0,0.0));
    }
    let albedo_luminance=dot(source.albedo,vec3(0.2126,0.7152,0.0722));
    var additive_luminance=1.0;
    if parameters.w<1.0 && albedo_luminance<colour.a {
        additive_luminance=smoothstep(0.0,parameters.w,albedo_luminance);
    }
    let opacity=parameters.w*additive_luminance;
    let colour_length=length(colour);
    var colour_normal=1.0;
    if colour_length>0.0 {colour_normal=1.0-dot(colour/colour_length,vec4(0.25));}
    colour_normal*=mix(1.0,parameters.z,colour.a);
    colour_normal*=mix(1.0,colour.a,parameters.y);
    let a=pow(1e-2+colour_normal,weights.x);
    let b=pow(min(0.02245,input.position.z)*weights.y,weights.z);
    let weight=clamp(a*b,1e-2,3e3)*weights.w;
    return ParticleAccumulation(vec4(colour.rgb,opacity)*(weight*colour.a),vec4(colour.a*opacity,0.0,0.0,0.0));
}
