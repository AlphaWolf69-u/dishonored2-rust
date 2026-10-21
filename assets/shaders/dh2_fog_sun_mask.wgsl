// PostFX_GodRays_Mask: projected source-colour mask, distinct from both
// sunlight visibility in fog and the smaller radial blur-visibility target.
// Concatenate after dh2_fog_math.
override SUN_SCATTERING:bool=false;
override SUN_SHADOWING:bool=false;
struct FogSunMaskUniform {
    transform1:vec4<f32>,
    transform2:vec4<f32>,
    threshold_bias:vec4<f32>,
    threshold_scale:vec4<f32>,
    color:vec4<f32>,
    projection:vec4<f32>, // inverse projection XY, far clip, HDR->game radiance
    viewport:vec4<f32>, // target width/height, depth-start, unused
    game_world_from_view:mat4x4<f32>,
}
@group(0) @binding(0) var<uniform> sun:FogSunMaskUniform;
@group(0) @binding(1) var<uniform> world:WorldFogParameters;
@group(0) @binding(2) var sun_source:texture_2d<f32>;
@group(0) @binding(3) var sun_depth:texture_2d<f32>;
@group(0) @binding(4) var sun_shadow:texture_2d<f32>;
@group(0) @binding(5) var sun_clamp:sampler;
const SUN_QUAD_UV=array<vec2<f32>,6>(
    vec2(0.0,1.0),vec2(1.0,1.0),vec2(0.0,0.0),
    vec2(0.0,0.0),vec2(1.0,1.0),vec2(1.0,0.0));
struct SunVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
    @location(1) ray:vec3<f32>,
}
@vertex fn fog_sun_vertex(@builtin(vertex_index) index:u32)->SunVertex {
    let uv=SUN_QUAD_UV[index];
    let local=uv*vec2(2.0,-2.0)+vec2(-1.0,1.0);
    let clip=local.x*sun.transform1.xy+local.y*sun.transform1.zw+sun.transform2.xy;
    var view_ray=vec3(0.0);
    if SUN_SCATTERING {
        // The installed VS uses the unscaled local quad coordinates here.
        let cs=sun.projection.xy*(local*vec2(1.0,sun.transform2.w)+sun.transform2.xy);
        view_ray=(sun.game_world_from_view*vec4(cs,-1.0,0.0)).xyz;
    }
    return SunVertex(vec4(clip,0.0,1.0),uv,view_ray);
}
@fragment fn fog_sun_fragment(in:SunVertex)->@location(0) vec4<f32> {
    let screen_uv=in.position.xy/sun.viewport.xy;
    var color=textureSampleLevel(sun_source,sun_clamp,screen_uv,0.0).rgb*sun.projection.w;
    if SUN_SCATTERING {
        var visibility=1.0;
        if SUN_SHADOWING {visibility=textureSampleLevel(sun_shadow,sun_clamp,screen_uv,0.0).r;}
        let direction=normalize(in.ray);
        let phase=fog_mie_phase(dot(world.scattering_direction_phase.xyz,direction),world.scattering_direction_phase.w);
        let scattering=world.scattering_color_intensity.rgb*(world.scattering_color_intensity.a*phase);
        let alpha=min(1.0,max(scattering.r,max(scattering.g,scattering.b)));
        color+=scattering*visibility*alpha;
    }
    var scaled=max(vec3(0.0),color-sun.threshold_bias.rgb)*sun.threshold_scale.rgb;
    let depth=textureSampleLevel(sun_depth,sun_clamp,screen_uv,0.0).r;
    let depth_mask=fog_saturate((depth-sun.viewport.z)/(sun.projection.z-sun.viewport.z));
    let center=in.uv*2.0-vec2(1.0);
    let circle=fog_saturate((dot(center,center)-1.0)/(-0.5));
    let luminance=dot(scaled,vec3(0.2989,0.5870,0.1140));
    scaled=mix(scaled,vec3(luminance),sun.color.a)*sun.color.rgb;
    return vec4(scaled*depth_mask*circle,0.0);
}
