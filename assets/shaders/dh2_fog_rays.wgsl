// PostFX_GodRays_{Masking,Scattering}. Concatenate after dh2_shadow and
// dh2_fog_math; the source is the separately rendered sun colour mask.
override RAY_TURBULENCE:bool=false;
override RAY_DEPTH_SCALE_X:f32=1.0;
override RAY_DEPTH_SCALE_Y:f32=1.0;
struct FogRayUniform {
    center:vec4<f32>, // projected NDC centre, same as mask Transform2
    controls:vec4<f32>, // density, decay, weight, unused
    extent:vec4<f32>, // current target width/height and their reciprocals
}
@group(0) @binding(0) var<uniform> ray:FogRayUniform;
@group(0) @binding(1) var<uniform> world:WorldFogParameters;
@group(0) @binding(2) var ray_source:texture_2d<f32>;
@group(0) @binding(3) var ray_depth:texture_2d<f32>;
@group(0) @binding(4) var ray_canvas:texture_2d<f32>;
@group(0) @binding(5) var ray_interior:texture_2d<f32>;
@group(0) @binding(6) var ray_clamp:sampler;
struct RayVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) uv:vec2<f32>,
}
@vertex fn fog_ray_vertex(@builtin(vertex_index) index:u32)->RayVertex {
    let uv=vec2<f32>(f32((index<<1u)&2u),f32(index&2u));
    return RayVertex(vec4(uv*vec2(2.0,-2.0)+vec2(-1.0,1.0),0.0,1.0),uv);
}
fn ray_center_uv()->vec2<f32> {
    return ray.center.xy*vec2(0.5,-0.5)+vec2(0.5);
}
fn ray_interleave(position:vec2<f32>)->f32 {
    return f32(screen_b8(vec2<u32>(position)%vec2(8u)))/64.0;
}
@fragment fn fog_ray_visibility_fragment(in:RayVertex)->@location(0) f32 {
    let delta=in.uv-ray_center_uv();
    let distance=length(delta)+ray.extent.z*0.5;
    var color=textureGather(1,ray_source,ray_clamp,in.uv);
    let offset=ray_interleave(in.position.xy)*ray.extent.zw;
    let tc=in.uv+offset*(delta/distance)-delta*fog_saturate(ray.controls.x);
    color+=textureGather(1,ray_source,ray_clamp,tc);
    return smoothstep(64.0/255.0,1.0,dot(color,vec4(0.25)));
}
fn fog_ray_scatter(in:RayVertex)->vec4<f32> {
    let density=fog_saturate(ray.controls.x);
    let delta=in.uv-ray_center_uv();
    let distance=length(delta)+ray.extent.z*0.5;
    var color=textureSampleLevel(ray_source,ray_clamp,in.uv,0.0).rgb;
    let offset=ray_interleave(in.position.xy)*(distance/8.0*density);
    var tc=in.uv+offset*(delta/distance);
    let step=delta*(density/8.0);
    var illumination_decay=1.0;
    for(var i=0u;i<8u;i++) {
        tc-=step;
        let texel=textureSampleLevel(ray_source,ray_clamp,tc,0.0).rgb;
        color+=texel*illumination_decay*ray.controls.z;
        illumination_decay*=ray.controls.y;
    }
    if RAY_TURBULENCE {
        let noise=textureSampleLevel(ray_canvas,ray_clamp,in.uv,0.0).r;
        let depth=textureSampleLevel(ray_depth,ray_clamp,in.uv*vec2(RAY_DEPTH_SCALE_X,RAY_DEPTH_SCALE_Y),0.0).r
            -textureSampleLevel(ray_interior,ray_clamp,in.uv,0.0).r;
        let factor=fog_saturate((depth-world.turbulence.x)/(world.turbulence.y-world.turbulence.x));
        color=mix(color,mix(color,world.turbulence_color.rgb,noise),factor);
    }
    return vec4(color,0.0);
}
@fragment fn fog_ray_scatter_fragment(in:RayVertex)->@location(0) vec4<f32> {
    return fog_ray_scatter(in);
}
