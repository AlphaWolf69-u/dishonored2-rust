#define_import_path dh2_surface

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
}
@group(#{MATERIAL_BIND_GROUP}) @binding(101) var<uniform> surface: SurfaceUniform;

// Installed HLSL uses compile-time #if material switches. Keep numeric
// parameters in the uniform but specialize resource/feature branches.
const compiled_flags = vec4<u32>(#{DH2_EXTRA_LAYERS}u, #{DH2_HAS_GLOSS}u, #{DH2_GLOSS_ALPHA}u, #{DH2_HAS_METALLIC}u);
const compiled_misc = vec4<u32>(#{DH2_HAS_BUMP}u, #{DH2_VERTEX_COLOR_MULTIPLY}u, #{DH2_SURFACE_ENABLED}u, #{DH2_ALPHA_FLAGS}u);
const compiled_occlusion = #{DH2_HAS_OCCLUSION}u;
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

struct SurfaceResult {
    albedo: vec4<f32>,
    roughness: f32,
    metallic: f32,
    occlusion: f32,
    normal_xy: vec2<f32>,
}

fn scaled_uv(uv: vec2<f32>, scale_bias: vec4<f32>) -> vec2<f32> {
    return uv * scale_bias.xy + scale_bias.zw;
}

// Uniform material switches select resources; explicit gradients also make
// derivative availability independent of control-flow analysis.
fn sample_role(role: u32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    switch role {
        case 0u: { return textureSampleGrad(diffuse0, surface_sampler, uv, dx, dy); }
        case 1u: { return textureSampleGrad(diffuse1, surface_sampler, uv, dx, dy); }
        case 2u: { return textureSampleGrad(diffuse2, surface_sampler, uv, dx, dy); }
        case 3u: { return textureSampleGrad(bump0, surface_sampler, uv, dx, dy); }
        case 4u: { return textureSampleGrad(bump1, surface_sampler, uv, dx, dy); }
        case 5u: { return textureSampleGrad(bump2, surface_sampler, uv, dx, dy); }
        case 6u: { return textureSampleGrad(gloss0, surface_sampler, uv, dx, dy); }
        case 7u: { return textureSampleGrad(gloss1, surface_sampler, uv, dx, dy); }
        case 8u: { return textureSampleGrad(gloss2, surface_sampler, uv, dx, dy); }
        case 9u: { return textureSampleGrad(metallic0, surface_sampler, uv, dx, dy); }
        case 10u: { return textureSampleGrad(metallic1, surface_sampler, uv, dx, dy); }
        case 11u: { return textureSampleGrad(metallic2, surface_sampler, uv, dx, dy); }
        case 12u: { return textureSampleGrad(mask0, surface_sampler, uv, dx, dy); }
        case 13u: { return textureSampleGrad(mask1, surface_sampler, uv, dx, dy); }
        default: { return textureSampleGrad(mask2, surface_sampler, uv, dx, dy); }
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
        if i == 0u && (compiled_misc.w & 1u) != 0u && (is_decal || is_vegetation || compiled_flags.z == 0u) {
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
    out.roughness = 1.0 - g;
    if compiled_flags.w != 0u { out.metallic = mix(surface.ranges.z, surface.ranges.w, m); }
    else { out.metallic = surface.ranges.w; }
    return out;
}
