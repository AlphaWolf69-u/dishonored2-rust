//! GPU inputs for the ordinary arkDefault/arkDecal layer branch.
use bevy::prelude::*;
use bevy::render::render_resource::ShaderType;
use crate::material_surface::Surface;

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
}

/// Material compile switches in the installed arkDefault/arkDefaultHelpers
/// HLSL are #if parameters, not per-fragment uniform branches. Values must
/// participate in the pipeline cache key; image handles and tints must not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SurfaceKey(pub [u32; 9]);

impl SurfaceKey {
    pub const NAMES: [&'static str; 9] = [
        "DH2_EXTRA_LAYERS", "DH2_HAS_GLOSS", "DH2_GLOSS_ALPHA",
        "DH2_HAS_METALLIC", "DH2_HAS_BUMP", "DH2_VERTEX_COLOR_MULTIPLY",
        "DH2_SURFACE_ENABLED", "DH2_ALPHA_FLAGS", "DH2_HAS_OCCLUSION",
    ];
}

impl From<&SurfaceUniform> for SurfaceKey {
    fn from(surface: &SurfaceUniform) -> Self {
        Self([
            surface.flags.x, surface.flags.y, surface.flags.z, surface.flags.w,
            surface.misc.x, surface.misc.y, surface.misc.z, surface.misc.w,
            u32::from(surface.alpha.y != 0.0),
        ])
    }
}

impl From<&Surface> for SurfaceUniform {
    fn from(s: &Surface) -> Self {
        Self {
            colors: s.layers.each_ref().map(|l| Vec4::from_array(l.color)),
            uv: s.layers.each_ref().map(|l| Vec4::from_array(l.uv_scale_bias)),
            mask_uv: s.layers.each_ref().map(|l| Vec4::from_array(l.mask_scale_bias)),
            uv_sets: s.layers.each_ref().map(|l| UVec4::new(l.uv_set, l.mask_uv_set, 0, 0)),
            mask_settings: s.layers.each_ref().map(|l| UVec4::new(l.mask_mode, l.mask_channel as u32, u32::from(l.use_color), u32::from(l.invert_mask))),
            mask_range: s.layers.each_ref().map(|l| Vec4::new(l.rescale[0], l.rescale[1], l.reuse[4] as f32, 0.0)),
            reuse: s.layers.each_ref().map(|l| UVec4::new(l.reuse[0] as u32, l.reuse[1] as u32, l.reuse[2] as u32, l.reuse[3] as u32)),
            ranges: Vec4::new(s.ward[0], s.ward[1], s.metallic_min_max[0], s.metallic_min_max[1]),
            alpha: Vec4::new(s.alpha_cutoff, u32::from(s.has_occlusion) as f32, 0.0, 0.0),
            flags: UVec4::new(s.extra_layers as u32, u32::from(s.has_gloss), u32::from(s.gloss_in_alpha), u32::from(s.has_metallic)),
            misc: UVec4::new(u32::from(s.has_bump), u32::from(s.vertex_color_multiply), s.program() as u32, 0),
        }
    }
}

/// Role is part of a material binding, not an image identity. One resident
/// image can be shared by several roles/layers/materials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImageRole(pub usize);
impl ImageRole {
    pub const fn diffuse(layer: usize) -> Self { Self(layer) }
    pub const fn bump(layer: usize) -> Self { Self(3 + layer) }
    pub const fn gloss(layer: usize) -> Self { Self(6 + layer) }
    pub const fn metallic(layer: usize) -> Self { Self(9 + layer) }
    pub const fn mask(layer: usize) -> Self { Self(12 + layer) }
    pub const fn occlusion() -> Self { Self(15) }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(SurfaceUniform::from(&Surface::parse(b"mainProgram unknown\n")).misc.z, 0);
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
}
