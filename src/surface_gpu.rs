//! GPU inputs for the ordinary arkDefault/arkDecal layer branch.
use crate::material_surface::Surface;
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;

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
    pub native_shadow: UVec4,
    pub specular_dielectric: Vec4,
    pub diffuse_metal: Vec4,
    pub specular_reuse: UVec4,
}

/// Material compile switches in the installed arkDefault/arkDefaultHelpers
/// HLSL are #if parameters, not per-fragment uniform branches. Values must
/// participate in the pipeline cache key; image handles and tints must not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceKey(pub [u32; 14]);

impl SurfaceKey {
    pub const NAMES: [&'static str; 14] = [
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
            surface.native_shadow.x,
            surface.native_shadow.y,
            surface.native_shadow.z,
            surface.native_shadow.w,
            surface.specular_reuse.w,
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
            native_shadow:UVec4::new(3,0,0,0),
            specular_dielectric:Vec3::from_array(s.specular_dielectric).extend(0.0),
            diffuse_metal:Vec3::from_array(s.diffuse_metal).extend(0.0),
            specular_reuse:UVec4::new(s.layers[0].specular_reuse,s.layers[1].specular_reuse,s.layers[2].specular_reuse,u32::from(s.has_specular)),
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
    pub const fn specular(layer:usize)->Self {Self(16+layer)}
    pub const fn occlusion() -> Self {
        Self(15)
    }
}

#[cfg(test)]
mod tests {
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
    fn native_specular_inputs_reach_uniforms_and_pipeline_key() {
        let surface=Surface::parse(b"hasSpecularMap 1\nextraLayer 2\nspecularMap original.tga\nLayer1_specularReuseLayer 1\nLayer2_specularReuseLayer 2\nspecularTintDielectric {0.02,0.03,0.07}\ndiffuseAlbedoMetallic {0.01,0.02,0.03}\n");
        let uniform=SurfaceUniform::from(&surface);
        assert_eq!(surface.layers[0].specular.as_deref(),Some("original.tga"));
        assert_eq!(uniform.specular_reuse,UVec4::new(0,1,2,1));
        assert_eq!(uniform.specular_dielectric,Vec4::new(0.02,0.03,0.07,0.0));
        assert_eq!(uniform.diffuse_metal,Vec4::new(0.01,0.02,0.03,0.0));
        let mut without=uniform.clone();without.specular_reuse.w=0;
        assert_ne!(SurfaceKey::from(&uniform),SurfaceKey::from(&without));
    }

    #[test]
    fn original_shadow_compile_options_select_distinct_pipelines() {
        let uniform=SurfaceUniform::from(&Surface::parse(b"mainProgram arkDefault\n"));
        let key=SurfaceKey::from(&uniform);
        for component in 0..4 {
            let mut changed=uniform.clone();changed.native_shadow[component]^=1;
            assert_ne!(SurfaceKey::from(&changed),key,"shadow compile option {component}");
        }
    }
}
