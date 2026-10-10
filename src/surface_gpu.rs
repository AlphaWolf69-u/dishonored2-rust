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
            misc: UVec4::new(u32::from(s.has_bump), u32::from(s.vertex_color_multiply), 0, 0),
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
