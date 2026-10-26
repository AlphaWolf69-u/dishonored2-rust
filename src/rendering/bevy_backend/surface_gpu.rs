//! GPU inputs for the ordinary arkDefault/arkDecal layer branch.
use crate::material_surface::Surface;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

const DIRECT_SHADER: Handle<bevy::shader::Shader> =
    bevy::asset::uuid_handle!("2dc96760-8a79-41cb-bd11-c3fc029c6705");

/// Shared surface-lighting kernels used by authored materials and host light loops.
pub fn register_shaders(app: &mut App) {
    const FRESNEL: Handle<Shader> = bevy::asset::uuid_handle!("a8f2b3a0-fad8-4dac-9f19-bc81c2f59c32");
    bevy::asset::load_internal_asset!(app,FRESNEL,"../../../assets/shaders/dh2_fresnel.wgsl",Shader::from_wgsl);
    const ALPHA_CLIP: Handle<Shader> = bevy::asset::uuid_handle!("e7f61878-cebb-4244-af0f-ad48081c2673");
    bevy::asset::load_internal_asset!(app,ALPHA_CLIP,"../../../assets/shaders/dh2_alpha_clip.wgsl",Shader::from_wgsl);
    bevy::asset::load_internal_asset!(app,DIRECT_SHADER,"../../../assets/shaders/dh2_direct.wgsl",bevy::shader::Shader::from_wgsl);
}

#[derive(Clone, Debug, Reflect, ShaderType)]
pub struct SurfaceUniform {
    pub colors: [Vec4; 3],
    pub uv: [Vec4; 3],
    pub mask_uv: [Vec4; 3],
    pub uv_sets: [UVec4; 3],
    pub mask_settings: [UVec4; 3],
    pub mask_range: [Vec4; 3],
    pub reuse: [UVec4; 3],
    pub ranges: Vec4,
    pub alpha: Vec4,
    pub flags: UVec4,
    pub misc: UVec4,
    /// Original forward receiver route, offset compile flags, filtering, light quality.
    pub shadow: UVec4,
    pub specular_dielectric: Vec4,
    pub diffuse_metal: Vec4,
    pub specular_reuse: UVec4,
    pub detail: Vec4,
    pub emissive_color: Vec4,
    pub emissive_flags: UVec4,
    pub emissive_clip: Vec4,
    pub vista: [Vec4; 4],
    pub vista_sun: [Vec4; 2],
    pub filling: [Vec4; 11],
    pub filling_flags: UVec4,
    pub sky: [Vec4; 7],
    pub direct: Vec4,
    /// Refraction IOR, uint blur size converted to float, normal mask, padding.
    pub glass: Vec4,
    /// arkSSSRender (program 11): wardRoughness2.xy/wardRoughness3.xy,
    /// SSS/tweakSpecularIntensity, then fuzz/intensity, fuzz/fresnel, SSS/Scale.x.
    pub skin: [Vec4; 3],
    /// Resolved arkSSSRender options: SSS/useSecondarySpecular, fuzz/enable,
    /// hasDiffuseLookup, then bit 0 SSS/hasSkinFresnelLookup, bit 1
    /// SSS/hasMask, bit 2 fuzz/hasMask.
    pub skin_flags: UVec4,
}

/// Material compile switches in the installed arkDefault/arkDefaultHelpers
/// HLSL are #if parameters, not per-fragment uniform branches. Values must
/// participate in the pipeline cache key; image handles and tints must not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceKey(pub [u32; 16]);

impl SurfaceKey {
    /// arkDefault's Z program returns without sampling when alphaTest is off.
    /// arkVegetation's Z VS still runs AnimateVegetation from its material's
    /// wind records; procedural liquid/sky and unsupported programs also keep
    /// their material path.
    pub fn depth_needs_material(self) -> bool {
        self.0[6] != 1 || self.0[7] & 1 != 0
    }

    /// The Z shader reads only the program/alpha switches. Keep procedural
    /// filling and projected-far controls as well; lighting/layers are absent
    /// from arkDefault's ClipZorShadowPass branch and dh2_probe_prepass.wgsl.
    pub fn depth_prepass(self) -> Self {
        let mut depth = [0; 16];
        for index in [6, 7, 14, 15] {
            depth[index] = self.0[index];
        }
        // Colour-only fog routes never enter ClipZorShadowPass.
        depth[7] &= 7;
        Self(depth)
    }
    pub const NAMES: [&'static str; 16] = [
        "DH2_EXTRA_LAYERS",
        "DH2_HAS_GLOSS",
        "DH2_GLOSS_ALPHA",
        "DH2_HAS_METALLIC",
        "DH2_HAS_BUMP",
        "DH2_VERTEX_COLOR_MULTIPLY",
        "DH2_SURFACE_ENABLED",
        "DH2_ALPHA_FLAGS",
        "DH2_HAS_OCCLUSION",
        "DH2_SHADOW_ROUTE",
        "DH2_SHADOW_OFFSET_FLAGS",
        "DH2_SHADOW_FILTER",
        "DH2_SHADOW_LIGHT_QUALITY",
        "DH2_HAS_SPECULAR",
        "DH2_FILLING_FLAGS",
        "DH2_PROJECT_FAR",
    ];
}

impl From<&SurfaceUniform> for SurfaceKey {
    fn from(surface: &SurfaceUniform) -> Self {
        Self([
            surface.flags.x,
            surface.flags.y,
            surface.flags.z,
            surface.flags.w,
            surface.misc.x,
            surface.misc.y,
            surface.misc.z,
            surface.misc.w,
            u32::from(surface.alpha.y != 0.0),
            surface.shadow.x,
            surface.shadow.y,
            surface.shadow.z,
            surface.shadow.w,
            surface.specular_reuse.w,
            surface.filling_flags.x,
            u32::from(surface.misc.z == 7 || (surface.misc.z == 8 && surface.sky[4].x != 0.0)),
        ])
    }
}

impl From<&Surface> for SurfaceUniform {
    fn from(s: &Surface) -> Self {
        Self {
            colors: s.layers.each_ref().map(|l| Vec4::from_array(l.color)),
            uv: s
                .layers
                .each_ref()
                .map(|l| Vec4::from_array(l.uv_scale_bias)),
            mask_uv: s
                .layers
                .each_ref()
                .map(|l| Vec4::from_array(l.mask_scale_bias)),
            uv_sets: s
                .layers
                .each_ref()
                .map(|l| UVec4::new(l.uv_set, l.mask_uv_set, 0, 0)),
            mask_settings: s.layers.each_ref().map(|l| {
                UVec4::new(
                    l.mask_mode,
                    l.mask_channel as u32,
                    u32::from(l.use_color),
                    u32::from(l.invert_mask),
                )
            }),
            mask_range: s
                .layers
                .each_ref()
                .map(|l| Vec4::new(l.rescale[0], l.rescale[1], l.reuse[4] as f32, 0.0)),
            reuse: s.layers.each_ref().map(|l| {
                UVec4::new(
                    l.reuse[0] as u32,
                    l.reuse[1] as u32,
                    l.reuse[2] as u32,
                    l.reuse[3] as u32,
                )
            }),
            ranges: Vec4::new(
                s.ward[0],
                s.ward[1],
                s.metallic_min_max[0],
                s.metallic_min_max[1],
            ),
            alpha: Vec4::new(s.alpha_cutoff, u32::from(s.has_occlusion) as f32, 0.0, 0.0),
            flags: UVec4::new(
                s.extra_layers as u32,
                u32::from(s.has_gloss),
                u32::from(s.gloss_in_alpha),
                u32::from(s.has_metallic),
            ),
            misc: UVec4::new(
                u32::from(s.has_bump),
                u32::from(s.vertex_color_multiply),
                s.program() as u32,
                u32::from(s.alpha_test),
            ),
            shadow: UVec4::new(3, 0, 0, 0),
            specular_dielectric: Vec3::from_array(s.specular_dielectric).extend(0.0),
            diffuse_metal: Vec3::from_array(s.diffuse_metal).extend(0.0),
            specular_reuse: UVec4::new(
                s.layers[0].specular_reuse,
                s.layers[1].specular_reuse,
                s.layers[2].specular_reuse,
                u32::from(s.has_specular),
            ),
            detail: Vec4::new(
                s.detail_tiling,
                s.detail_intensity,
                u32::from(s.has_detail_bump) as f32,
                0.,
            ),
            emissive_color: Vec3::from_array(s.emissive_color).extend(s.emissive_intensity),
            emissive_flags: UVec4::new(
                u32::from(s.has_emissive_map),
                u32::from(s.has_emissive_mask),
                u32::from(s.animated_emissive),
                0,
            ),
            emissive_clip: Vec4::new(s.emissive_clip[0], s.emissive_clip[1], 0., 0.),
            vista: [Vec4::ZERO; 4],
            vista_sun: [Vec4::ZERO; 2],
            filling: [Vec4::ZERO; 11],
            filling_flags: UVec4::ZERO,
            sky: [Vec4::ZERO; 7],
            direct: Vec4::new(s.sheen[0],s.sheen[1],0.0,0.0),
            glass: Vec4::ZERO,
            skin: [
                Vec4::new(s.skin.ward2[0], s.skin.ward2[1], s.skin.ward3[0], s.skin.ward3[1]),
                Vec4::from_array(s.skin.tweak_specular),
                Vec4::new(s.skin.fuzz[0], s.skin.fuzz[1], s.skin.scale, 0.0),
            ],
            skin_flags: UVec4::new(
                0,
                0,
                u32::from(s.skin.diffuse_lookup.is_some()),
                u32::from(s.skin.skin_fresnel_lookup.is_some())
                    | (u32::from(s.skin.sss_mask.is_some()) << 1)
                    | (u32::from(s.skin.fuzz_mask.is_some()) << 2),
            ),
        }
    }
}

/// Role is part of a material binding, not an image identity. One resident
/// image can be shared by several roles/layers/materials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageRole(pub usize);
impl ImageRole {
    pub const fn diffuse(layer: usize) -> Self {
        Self(layer)
    }
    pub const fn bump(layer: usize) -> Self {
        Self(3 + layer)
    }
    pub const fn gloss(layer: usize) -> Self {
        Self(6 + layer)
    }
    pub const fn metallic(layer: usize) -> Self {
        Self(9 + layer)
    }
    pub const fn mask(layer: usize) -> Self {
        Self(12 + layer)
    }
    pub const fn specular(layer: usize) -> Self {
        Self(16 + layer)
    }
    pub const fn occlusion() -> Self {
        Self(15)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn depth_key_retains_only_consumed_shader_controls() {
        let key = super::SurfaceKey([9; 16]);
        let depth = key.depth_prepass();
        for index in 0..16 {
            let mut changed = key;
            changed.0[index] ^= 1;
            assert_eq!(
                changed.depth_prepass() != depth,
                [6, 7, 14, 15].contains(&index)
            );
        }
        assert_eq!(depth.depth_prepass(), depth);
        let mut key = key;
        key.0[7] &= 7;
        for fog_flags in [8, 24, 56] {
            let mut fog = key;
            fog.0[7] |= fog_flags;
            assert_ne!(fog, key);
            assert_eq!(
                fog.depth_prepass(),
                depth,
                "colour fog must not split equivalent depth programs"
            );
        }
    }
    use super::*;

    #[test]
    fn direct_surface_conversion_preserves_authored_alpha_test() {
        let enabled =
            SurfaceUniform::from(&Surface::parse(b"mainProgram arkDefault\nalphaTest 1\n"));
        let disabled =
            SurfaceUniform::from(&Surface::parse(b"mainProgram arkDefault\nalphaTest 0\n"));
        assert_eq!(enabled.misc.w & 1, 1);
        assert_eq!(disabled.misc.w & 1, 0);
        assert_ne!(SurfaceKey::from(&enabled), SurfaceKey::from(&disabled));
    }

    #[test]
    fn vegetation_program_is_a_distinct_authored_pipeline() {
        let default = SurfaceUniform::from(&Surface::parse(b"mainProgram arkDefault\n"));
        let vegetation = SurfaceUniform::from(&Surface::parse(b"mainProgram arkVegetation\n"));
        let water = SurfaceUniform::from(&Surface::parse(b"mainProgram arkwater\n"));
        assert_eq!(default.misc.z, 1);
        assert_eq!(vegetation.misc.z, 2);
        assert_eq!(water.misc.z, 3);
        assert_ne!(SurfaceKey::from(&default), SurfaceKey::from(&vegetation));
        assert_ne!(SurfaceKey::from(&default), SurfaceKey::from(&water));
        assert_eq!(
            SurfaceUniform::from(&Surface::parse(b"mainProgram unknown\n"))
                .misc
                .z,
            0
        );
    }

    #[test]
    fn compile_key_ignores_numeric_material_parameters_but_tracks_all_switches() {
        let surface = Surface::parse(b"");
        let mut uniform = SurfaceUniform::from(&surface);
        let key = SurfaceKey::from(&uniform);
        uniform.colors[0] = Vec4::ZERO;
        uniform.uv[1] *= 2.0;
        uniform.ranges += Vec4::ONE;
        assert_eq!(SurfaceKey::from(&uniform), key);
        uniform.flags.x += 1;
        assert_ne!(SurfaceKey::from(&uniform), key);
        uniform.flags.x -= 1;
        uniform.misc.w ^= 1;
        assert_ne!(SurfaceKey::from(&uniform), key);
        uniform.misc.w ^= 1;
        uniform.alpha.y = 1.0 - uniform.alpha.y;
        assert_ne!(SurfaceKey::from(&uniform), key);
    }

    #[test]
    fn specular_inputs_reach_uniforms_and_pipeline_key() {
        let surface=Surface::parse(b"hasSpecularMap 1\nextraLayer 2\nspecularMap original.tga\nLayer1_specularReuseLayer 1\nLayer2_specularReuseLayer 2\nspecularTintDielectric {0.02,0.03,0.07}\ndiffuseAlbedoMetallic {0.01,0.02,0.03}\n");
        let uniform = SurfaceUniform::from(&surface);
        assert_eq!(surface.layers[0].specular.as_deref(), Some("original.tga"));
        assert_eq!(uniform.specular_reuse, UVec4::new(0, 1, 2, 1));
        assert_eq!(
            uniform.specular_dielectric,
            Vec4::new(0.02, 0.03, 0.07, 0.0)
        );
        assert_eq!(uniform.diffuse_metal, Vec4::new(0.01, 0.02, 0.03, 0.0));
        let mut without = uniform.clone();
        without.specular_reuse.w = 0;
        assert_ne!(SurfaceKey::from(&uniform), SurfaceKey::from(&without));
    }

    #[test]
    fn shadow_compile_options_select_distinct_pipelines() {
        let uniform = SurfaceUniform::from(&Surface::parse(b"mainProgram arkDefault\n"));
        let key = SurfaceKey::from(&uniform);
        for component in 0..4 {
            let mut changed = uniform.clone();
            changed.shadow[component] ^= 1;
            assert_ne!(
                SurfaceKey::from(&changed),
                key,
                "shadow compile option {component}"
            );
        }
    }
}
