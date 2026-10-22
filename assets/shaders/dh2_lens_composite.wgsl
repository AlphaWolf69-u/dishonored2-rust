// Standard PostFX/ToneMapping before exposure/curve/grading. Inputs are in
// game radiance units; the display integration handles host exposure units.
struct LensCompositeParameters {
    viewport:vec4<f32>, // resolutionScale.xy, subpixelOffset.xy
    pixel:vec4<f32>, // inverse target size.xy, source camera axis, host units
    controls:vec4<f32>, // veil strength, lens dirt strength, blend weight, white level
    streak:vec4<f32>, // radius, random offset, width, opacity
    tint:vec4<f32>,
    flags:vec4<u32>, // bloom enabled, flares enabled, default dirt, BW program
    options:vec4<u32>, // compressed HDR, BW runtime, BW excluded, embed cube
}
fn lens_composite(p:LensCompositeParameters,raw_colour:vec3<f32>,uv:vec2<f32>,
    engine_luminance:f32,bloom_map:texture_2d<f32>,dirt_from:texture_2d<f32>,
    dirt_to:texture_2d<f32>,flare_map:texture_2d<f32>,linear_clamp:sampler)->vec3<f32> {
    var colour=hdr_range_expand(raw_colour,p.options.x!=0u,engine_luminance,p.controls.w);
    if p.options.w==0u && (p.flags.x!=0u || p.flags.y!=0u) {
        let scaled_uv=uv*p.viewport.xy;
        var dirt=textureSampleLevel(dirt_from,linear_clamp,uv+p.viewport.zw,0.0).rgb;
        if p.flags.z==0u {
            let dirt_target=textureSampleLevel(dirt_to,linear_clamp,uv+p.viewport.zw,0.0).rgb;
            dirt=mix(dirt,dirt_target,p.controls.z);
        }
        if p.flags.w!=0u {dirt=black_white_overlay(dirt,p.tint.xyz,p.options.y!=0u,p.options.z!=0u);}
        // Preserve this branch: multiplying a NaN bloom sample by zero does
        // not reproduce the source's disabled-veil behaviour.
        if p.controls.x>0.0 {
            var bloom=p.controls.x*hdr_range_expand(textureSampleLevel(bloom_map,linear_clamp,scaled_uv,0.0).rgb,
                p.options.x!=0u,engine_luminance,p.controls.w);
            if p.flags.w!=0u {bloom=black_white_overlay(bloom,p.tint.xyz,p.options.y!=0u,p.options.z!=0u);}
            colour+=p.controls.y*dirt*bloom+bloom;
        }
        dirt+=vec3(lens_streak(uv,p.pixel.z,p.streak));
        let flare_uv=clamp(scaled_uv-p.pixel.xy*(vec2(1.0)/p.viewport.xy),vec2(0.0),vec2(1.0));
        var flares=hdr_range_expand(textureSampleLevel(flare_map,linear_clamp,flare_uv,0.0).rgb,
            p.options.x!=0u,engine_luminance,p.controls.w);
        flares*=vec3(0.2)+0.8*dirt;
        if p.flags.w!=0u {flares=black_white_overlay(flares,p.tint.xyz,p.options.y!=0u,p.options.z!=0u);}
        colour+=flares;
    }
    return max(vec3(0.0),colour);
}
