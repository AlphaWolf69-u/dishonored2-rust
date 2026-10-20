#define_import_path dh2_surface

#import dh2_indirect::native_diffuse_response
#import bevy_pbr::pbr_bindings::{occlusion_texture, occlusion_sampler}

struct SurfaceUniform {
    colors: array<vec4<f32>, 3>,
    uv: array<vec4<f32>, 3>,
    mask_uv: array<vec4<f32>, 3>,
    uv_sets: array<vec4<u32>, 3>,
    mask_settings: array<vec4<u32>, 3>,
    mask_range: array<vec4<f32>, 3>,
    reuse: array<vec4<u32>, 3>,
    ranges: vec4<f32>,
    alpha: vec4<f32>,
    flags: vec4<u32>,
    misc: vec4<u32>,
    native_shadow: vec4<u32>,
    specular_dielectric:vec4<f32>,
    diffuse_metal:vec4<f32>,
    specular_reuse:vec4<u32>,
    detail:vec4<f32>,
    emissive_color:vec4<f32>,
    emissive_flags:vec4<u32>,
    emissive_clip:vec4<f32>,
    vista: array<vec4<f32>,4>,
    vista_sun: array<vec4<f32>,2>,
    filling:array<vec4<f32>,11>,
    filling_flags:vec4<u32>,
    sky:array<vec4<f32>,7>,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> surface: SurfaceUniform;

// Installed HLSL uses compile-time #if material switches. Keep numeric
// parameters in the uniform but specialize resource/feature branches.
const compiled_flags = vec4<u32>(#{DH2_EXTRA_LAYERS}u, #{DH2_HAS_GLOSS}u, #{DH2_GLOSS_ALPHA}u, #{DH2_HAS_METALLIC}u);
const compiled_misc = vec4<u32>(#{DH2_HAS_BUMP}u, #{DH2_VERTEX_COLOR_MULTIPLY}u, #{DH2_SURFACE_ENABLED}u, #{DH2_ALPHA_FLAGS}u);
const compiled_occlusion = #{DH2_HAS_OCCLUSION}u;
// Original shadowCommon offset flags and shadowFilteringQuality are shader
// options. Preserve specialization instead of evaluating their branches for
// every local light in every fragment.
const compiled_native_shadow = vec4<u32>(#{DH2_SHADOW_ROUTE}u, #{DH2_SHADOW_OFFSET_FLAGS}u,
    #{DH2_SHADOW_FILTER}u, #{DH2_SHADOW_LIGHT_QUALITY}u);
@group(#{MATERIAL_BIND_GROUP}) @binding(102) var diffuse0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(103) var diffuse1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(104) var diffuse2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(105) var bump0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(106) var bump1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(107) var bump2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(108) var gloss0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(109) var gloss1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(110) var gloss2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(111) var metallic0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(112) var metallic1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(113) var metallic2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(114) var mask0: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(115) var mask1: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(116) var mask2: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(117) var surface_sampler: sampler;

@group(#{MATERIAL_BIND_GROUP}) @binding(134) var specular0:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(135) var specular1:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(136) var specular2:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(137) var detail_bump:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(138) var emissive_map:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(139) var emissive_mask:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(140) var surface_anisotropic_sampler:sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(142) var lightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(143) var filling_diffuse:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(144) var filling_distortion1:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(145) var filling_distortion2:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(146) var filling_lookup:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(147) var filling_normal:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(148) var filling_fresnel:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(149) var linear_clamp:sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(151) var cloud_opacity:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(152) var cloud_density:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(153) var cloud_back_density:texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(154) var<storage,read> material_exposure:array<f32>;
#ifdef PREPASS_PIPELINE
#import bevy_render::globals::Globals
@group(0) @binding(1) var<uniform> globals:Globals;
#else
#import bevy_pbr::mesh_view_bindings::globals
#endif
#import dh2_filling::{filling_pan_uv,filling_lookup_coordinate,filling_transfer_color,filling_step,filling_view_fade}
fn filling_plane_alpha(alpha:f32,world:vec3<f32>,clip_w:f32)->f32 {
    let p=surface.filling;
    var faded_alpha=alpha;
    if (#{DH2_FILLING_FLAGS}u & 16u)!=0u {
        faded_alpha*=filling_view_fade(clip_w,p[10].z,p[10].w);
    }
    let game_position=vec3(world.x,-world.z,world.y);
    let plane=dot(p[9],vec4(game_position,1.0));
    if plane<0.0 {discard;}
    return faded_alpha*p[8].x*filling_step(0.0,p[10].x,plane);
}
fn filling_colour(uv:vec2<f32>,uv1:vec2<f32>,vertex_color:vec4<f32>)->vec4<f32> {
    let p=surface.filling;
    let one=filling_pan_uv(uv,uv1.y*p[4].z+globals.time,p[4].x,p[0].xy,p[2].xy);
    let two=filling_pan_uv(-uv,-uv1.y*p[4].z+globals.time,p[4].y,p[1].xy,p[3].xy);
    let a=textureSampleGrad(filling_distortion1,surface_sampler,one,dpdx(one),dpdy(one)).x;
    let b=textureSampleGrad(filling_distortion2,surface_sampler,two,dpdx(two),dpdy(two)).x;
    let distorted=uv+vec2(a*b);
    let rgb=textureSampleGrad(filling_diffuse,surface_sampler,distorted,dpdx(distorted),dpdy(distorted))
        +textureSampleGrad(filling_diffuse,surface_sampler,-distorted,dpdx(-distorted),dpdy(-distorted));
    let coord=filling_lookup_coordinate(uv.y,a*b,1.0-p[4].w,p[5]);
    let lookup=textureSampleGrad(filling_lookup,surface_sampler,vec2(coord,0.5),vec2(dpdx(coord),0.0),vec2(dpdy(coord),0.0));
    return filling_transfer_color(rgb,lookup,coord,uv.y,1.0-p[4].w,p[5].w,p[6],p[7],1.0);
}


struct SurfaceResult {
    albedo: vec4<f32>,
    roughness: f32,
    metallic: f32,
    occlusion: f32,
    normal_xy: vec2<f32>,
    diffuse_response:vec3<f32>,
    emissive:vec3<f32>,
}

fn scaled_uv(uv: vec2<f32>, scale_bias: vec4<f32>) -> vec2<f32> {
    return uv * scale_bias.xy + scale_bias.zw;
}

// Uniform material switches select resources; explicit gradients also make
// derivative availability independent of control-flow analysis.
fn sample_material_image(image:texture_2d<f32>,uv:vec2<f32>,dx:vec2<f32>,dy:vec2<f32>,anisotropic:bool)->vec4<f32> {
    if anisotropic && (compiled_misc.w & 2u)==0u {
        return textureSampleGrad(image,surface_anisotropic_sampler,uv,dx,dy);
    }
    return textureSampleGrad(image,surface_sampler,uv,dx,dy);
}
fn sample_role(role: u32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    switch role {
        case 0u: { return sample_material_image(diffuse0, uv, dx, dy, true); }
        case 1u: { return sample_material_image(diffuse1, uv, dx, dy, true); }
        case 2u: { return sample_material_image(diffuse2, uv, dx, dy, true); }
        case 3u: { return sample_material_image(bump0, uv, dx, dy, true); }
        case 4u: { return sample_material_image(bump1, uv, dx, dy, true); }
        case 5u: { return sample_material_image(bump2, uv, dx, dy, true); }
        case 6u: { return sample_material_image(gloss0, uv, dx, dy, false); }
        case 7u: { return sample_material_image(gloss1, uv, dx, dy, false); }
        case 8u: { return sample_material_image(gloss2, uv, dx, dy, false); }
        case 9u: { return sample_material_image(metallic0, uv, dx, dy, true); }
        case 10u: { return sample_material_image(metallic1, uv, dx, dy, true); }
        case 11u: { return sample_material_image(metallic2, uv, dx, dy, true); }
        case 12u: { return sample_material_image(mask0, uv, dx, dy, true); }
        case 13u: { return sample_material_image(mask1, uv, dx, dy, true); }
        case 16u: { return sample_material_image(specular0,uv,dx,dy,true); }
        case 17u: { return sample_material_image(specular1,uv,dx,dy,true); }
        case 18u: { return sample_material_image(specular2,uv,dx,dy,true); }
        default: { return sample_material_image(mask2, uv, dx, dy, true); }
    }
}

// global.hlsl uses signed BC5 XY and this non-sqrt reconstruction.
fn unpack_bc5_normal_map(xy: vec2<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(xy, 1.001 - clamp(dot(xy, xy), 0.0, 1.0)));
}

fn evaluate_surface(uv0: vec2<f32>, uv1: vec2<f32>, vertex_color: vec4<f32>) -> SurfaceResult {
    let dx0 = dpdx(uv0);
    let dy0 = dpdy(uv0);
    let dx1 = dpdx(uv1);
    let dy1 = dpdy(uv1);
    var masks = array<f32, 3>(0.0, 0.0, 0.0);
    var raw_masks = array<f32, 3>(0.0, 0.0, 0.0);
    var diffuse: array<vec4<f32>, 3>;
    var gloss: array<f32, 3>;
    var metallic: array<f32, 3>;
    var bump: array<vec2<f32>, 3>;
    let is_decal = (compiled_misc.w & 2u) != 0u;
    let is_vegetation = compiled_misc.z == 2u;
    // arkDecal uses LQ_FetchMasks/normal and Decal_FetchDiffuseAlbedo,
    // not arkDefault's additional albedo/normal layers.
    let last_layer = select(compiled_flags.x, 0u, is_decal);
    for (var i = 0u; i <= last_layer; i += 1u) {
        let info = surface.mask_settings[i];
        let uv = select(uv0, uv1, surface.uv_sets[i].x != 0u);
        let dx = select(dx0, dx1, surface.uv_sets[i].x != 0u);
        let dy = select(dy0, dy1, surface.uv_sets[i].x != 0u);
        let mask_uv = select(uv0, uv1, surface.uv_sets[i].y != 0u);
        let mask_dx = select(dx0, dx1, surface.uv_sets[i].y != 0u);
        let mask_dy = select(dy0, dy1, surface.uv_sets[i].y != 0u);
        if i != 0u || info.z != 0u {
            let channel = vertex_color[info.y];
            if info.x == 0u {
                masks[i] = channel;
            } else {
                let reuse = u32(surface.mask_range[i].z);
                if reuse == 0u {
                    raw_masks[i] = sample_role(12u + i, scaled_uv(mask_uv, surface.mask_uv[i]), mask_dx * surface.mask_uv[i].xy, mask_dy * surface.mask_uv[i].xy).r;
                } else {
                    raw_masks[i] = raw_masks[reuse - 1u];
                }
                if info.x == 1u {
                    masks[i] = raw_masks[i];
                } else {
                    masks[i] = clamp((raw_masks[i] * channel - surface.mask_range[i].x) / (surface.mask_range[i].y - surface.mask_range[i].x), 0.0, 1.0);
                }
            }
        }
        if info.w != 0u { masks[i] = 1.0 - masks[i]; }
        let u = scaled_uv(uv, surface.uv[i]);
        let ux = dx * surface.uv[i].xy;
        let uy = dy * surface.uv[i].xy;
        let reuse = surface.reuse[i];
        if i != 0u && reuse.x != 0u { diffuse[i] = diffuse[reuse.x - 1u]; }
        else { diffuse[i] = sample_role(i, u, ux, uy); }
        // Native forward clipping happens at diffuse0, before normal/gloss
        // fetches and additional-layer composition.
        if compiled_misc.z!=6u && i == 0u && (compiled_misc.w & 1u) != 0u && (is_decal || is_vegetation || compiled_flags.z == 0u) {
            let cutoff = select(surface.alpha.x, 0.5, is_decal || is_vegetation);
            if diffuse[0].a < cutoff { discard; }
        }
        if compiled_flags.y != 0u {
            if i != 0u && reuse.z != 0u { gloss[i] = gloss[reuse.z - 1u]; }
            else { gloss[i] = sample_role(6u + i, u, ux, uy).r; }
        }
        if compiled_flags.w != 0u {
            if i != 0u && reuse.w != 0u { metallic[i] = metallic[reuse.w - 1u]; }
            else { metallic[i] = sample_role(9u + i, u, ux, uy).r; }
        }
        if compiled_misc.x != 0u {
            if i != 0u && reuse.y != 0u { bump[i] = bump[reuse.y - 1u]; }
            else { bump[i] = sample_role(3u + i, u, ux, uy).rg; }
        }
    }
    var specular=vec3(1.0);
    var specular_layers:array<vec3<f32>,3>;
    if #{DH2_HAS_SPECULAR}u != 0u {
        for(var i=0u;i<=last_layer;i++) {
            let reuse=surface.specular_reuse[i];
            if i!=0u && reuse!=0u {specular_layers[i]=specular_layers[reuse-1u];}
            else {
                let uv=select(uv0,uv1,surface.uv_sets[i].x!=0u);
                let dx=select(dx0,dx1,surface.uv_sets[i].x!=0u)*surface.uv[i].xy;
                let dy=select(dy0,dy1,surface.uv_sets[i].x!=0u)*surface.uv[i].xy;
                specular_layers[i]=sample_role(16u+i,scaled_uv(uv,surface.uv[i]),dx,dy).rgb;
            }
            if i==0u {specular=specular_layers[i];}
            else {specular=mix(specular,specular_layers[i],masks[i]);}
        }
    }
    var out: SurfaceResult;
    out.albedo = diffuse[0];
    // arkVegetation samples diffuse0 directly; vertex RGB belongs to wind.
    if !is_vegetation && surface.mask_settings[0].z != 0u {
        out.albedo *= mix(vec4<f32>(1.0), surface.colors[0], masks[0]);
    }
    var g = gloss[0];
    var m = metallic[0];
    out.normal_xy = bump[0];
    for (var i = 1u; i <= last_layer; i += 1u) {
        var color = diffuse[i];
        if surface.mask_settings[i].z != 0u { color *= surface.colors[i]; }
        if !is_vegetation { out.albedo = mix(out.albedo, color, masks[i]); }
        if i != 1u || surface.reuse[i].z == 0u { g = mix(g, gloss[i], masks[i]); }
        if i != 1u || surface.reuse[i].w == 0u { m = mix(m, metallic[i], masks[i]); }
        // Layer1 bump reuse likewise skips interpolation of identical input.
        if i != 1u || surface.reuse[i].y == 0u { out.normal_xy = mix(out.normal_xy, bump[i], masks[i]); }
    }
    if !is_vegetation && compiled_misc.y != 0u { out.albedo *= vertex_color; }
    let base_uv=select(uv0,uv1,surface.uv_sets[0].x!=0u);
    let base_dx=select(dx0,dx1,surface.uv_sets[0].x!=0u)*surface.uv[0].xy;
    let base_dy=select(dy0,dy1,surface.uv_sets[0].x!=0u)*surface.uv[0].xy;
    let base_scaled=scaled_uv(base_uv,surface.uv[0]);
    if !is_decal && compiled_misc.x!=0u && surface.detail.z!=0.0 {
        out.normal_xy+=sample_material_image(detail_bump,base_scaled*surface.detail.x,
            base_dx*surface.detail.x,base_dy*surface.detail.x,true).xy*surface.detail.y;
    }
    out.emissive=vec3(0.0);
    // AQ/HQ ordinary emission and LQ constant emission. Animated emission's
    // separate time/mask owner remains an unsupported branch.
    if surface.emissive_color.w!=0.0 && (surface.emissive_flags.z==0u || is_decal) {
        var e=surface.emissive_color.rgb;
        if !is_decal && surface.emissive_flags.x!=0u {
            e=sample_material_image(emissive_map,base_scaled,base_dx,base_dy,true).rgb;
        }
        if !is_decal && surface.emissive_flags.y!=0u {
            let mask=sample_material_image(emissive_mask,base_scaled,base_dx,base_dy,true).r;
            // Native smoothstep has reversed edges; spell its polynomial out
            // because WGSL smoothstep requires ordered edges.
            let t=clamp((surface.emissive_clip.x-mask-surface.emissive_clip.y)/(-2.0*surface.emissive_clip.y),0.0,1.0);
            e*=t*t*(3.0-2.0*t);
        }
        out.emissive=surface.emissive_color.w*e;
    }
    if !is_decal && !is_vegetation { out.albedo = max(out.albedo, vec4<f32>(0.00001)); }
    if compiled_flags.z != 0u {
        g = mix(surface.ranges.x, surface.ranges.y, out.albedo.a);
        out.albedo.a = 1.0;
    } else if compiled_flags.y != 0u {
        g = mix(surface.ranges.x, surface.ranges.y, g);
    } else { g = surface.ranges.y; }
    out.occlusion = 1.0;
    if !is_decal && compiled_occlusion != 0u {
        let uv = select(uv0, uv1, surface.uv_sets[0].x != 0u);
        let dx = select(dx0, dx1, surface.uv_sets[0].x != 0u) * surface.uv[0].xy;
        let dy = select(dy0, dy1, surface.uv_sets[0].x != 0u) * surface.uv[0].xy;
        out.occlusion = textureSampleGrad(occlusion_texture, occlusion_sampler, scaled_uv(uv, surface.uv[0]), dx, dy).r;
    }
    if compiled_misc.z==6u {out.albedo=filling_colour(uv0,uv1,vertex_color);}
    out.roughness = 1.0 - g;
    if compiled_flags.w != 0u { out.metallic = mix(surface.ranges.z, surface.ranges.w, m); }
    else { out.metallic = surface.ranges.w; }
    out.diffuse_response=native_diffuse_response(out.albedo.rgb,out.roughness,out.metallic,specular,
        #{DH2_HAS_SPECULAR}u==0u,surface.specular_dielectric.rgb,surface.diffuse_metal.rgb);
    return out;
}
