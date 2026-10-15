//! Receiver transport for the original directional depth-shadow algorithm.
//!
//! Original row vectors are stored as WGSL columns (the shader dots each row).
//! This is a stable shared ShaderBuffer, not one buffer per mesh/material/frame.
//! It transports SampleDirectionalShadowMap(ComputeShadowOffset(...)); the
//! atmosphere/cloud attenuation path remains separate and is not substituted.

use bevy::{
    app::App,
    asset::Handle,
    ecs::prelude::Resource,
    image::Image,
    math::{IVec4, Mat4, UVec4, Vec4},
    render::{
        extract_resource::ExtractResource, render_resource::ShaderType, storage::ShaderBuffer,
    },
    shader::Shader,
};

const SHADOW_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("1d67e2c0-a515-4bfe-9b55-276e82430c06");
const RECEIVER_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("8351debf-491b-4d7d-ad83-b03a15260af9");

pub fn register_shaders(app: &mut App) {
    bevy::asset::load_internal_asset!(
        app,
        SHADOW_SHADER,
        "../assets/shaders/dh2_shadow.wgsl",
        |source: &str, path| Shader::from_wgsl(
            format!("#define_import_path dh2_shadow\n{source}"),
            path
        )
    );
    bevy::asset::load_internal_asset!(
        app,
        RECEIVER_SHADER,
        "../assets/shaders/dh2_parallel_receiver.wgsl",
        Shader::from_wgsl
    );
}

/// Resolved original shadowCommon compile options and render parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeShadowOffsetParameters {
    pub sun_direction: [f32; 3],
    pub use_normal_offset: bool,
    pub use_slope_scale: bool,
    pub use_slope_bias: bool,
    pub slope_post_projection: bool,
    /// The original omitted-argument UV-normal-offset caller is still untraced.
    pub use_uv_normal_offset: bool,
    pub normal_offset: f32,
    pub bias: f32,
    pub slope_bias: f32,
}

/// Authored GPU transport layout, not a claim about the original constant-buffer ABI.
#[derive(Clone, Debug, ShaderType)]
pub struct NativeParallelSamplingUniform {
    pub world_to_shadow: [Mat4; 5],
    pub view_row2: Vec4,
    pub cascade_fars: Vec4,
    pub sun_direction: Vec4,
    pub shadow_size: Vec4,
    pub bias: Vec4,
    /// Path: 0=unavailable, 1=direct atlas, 2=screen producer; sun index, options, matrix count.
    pub config: UVec4,
    /// original light quality, original shadowFilteringQuality, unused, unused.
    pub quality: IVec4,
}

impl Default for NativeParallelSamplingUniform {
    fn default() -> Self {
        Self {
            world_to_shadow: [Mat4::ZERO; 5],
            view_row2: Vec4::ZERO,
            cascade_fars: Vec4::ZERO,
            sun_direction: Vec4::ZERO,
            shadow_size: Vec4::ZERO,
            bias: Vec4::ZERO,
            config: UVec4::ZERO,
            quality: IVec4::ZERO,
        }
    }
}

impl NativeParallelSamplingUniform {
    pub fn from_receiver(
        receiver: &crate::native_shadow_scene::NativeParallelReceiverVars,
    ) -> Result<Self, String> {
        if receiver.path != crate::render_shadow::ParallelShadowReceiverPath::DirectVisibility {
            return Err(format!(
                "original parallel receiver path {:?} is not implemented by the scalar atlas transport",
                receiver.path,
            ));
        }
        Self::pack_receiver(receiver)
    }

    /// The original screenShadows program calls ComputeParallelShadow with a
    /// ZERO vertex normal and explicitly disables vertex-normal offset. Its
    /// transport must not accidentally enable the direct-atlas opaque hook.
    pub fn from_screen_receiver(
        receiver: &crate::native_shadow_scene::NativeParallelReceiverVars,
    ) -> Result<Self, String> {
        if receiver.path != crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace {
            return Err(
                "screen-shadow producer requires the original ScreenSpace receiver path".into(),
            );
        }
        let mut value = Self::pack_receiver(receiver)?;
        value.config.x = 2;
        value.config.z &= !1;
        Ok(value)
    }

    fn pack_receiver(
        receiver: &crate::native_shadow_scene::NativeParallelReceiverVars,
    ) -> Result<Self, String> {
        let count = receiver.world_to_shadow.len();
        if count == 0
            || count > 5
            || receiver
                .world_to_shadow
                .iter()
                .flatten()
                .flatten()
                .any(|v| !v.is_finite())
        {
            return Err("native parallel receiver needs 1..=5 finite owner matrices".into());
        }
        if receiver.offset.use_uv_normal_offset {
            return Err("original UV-normal-offset compiled caller remains unsupported".into());
        }
        if receiver.offset.sun_direction.iter().any(|v| !v.is_finite())
            || [
                receiver.offset.normal_offset,
                receiver.offset.bias,
                receiver.offset.slope_bias,
            ]
            .iter()
            .any(|v| !v.is_finite())
            || receiver
                .view_row2
                .iter()
                .chain(&receiver.cascade_fars)
                .any(|v| !v.is_finite())
            || receiver
                .shadow_size
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err(
                "nonfinite native parallel sampling parameters or invalid shadow size".into(),
            );
        }
        let mut value = Self::default();
        for (destination, rows) in value
            .world_to_shadow
            .iter_mut()
            .zip(&receiver.world_to_shadow)
        {
            *destination = Mat4::from_cols_array_2d(rows);
        }
        let flags = u32::from(receiver.offset.use_normal_offset)
            | (u32::from(receiver.offset.use_slope_scale) << 1)
            | (u32::from(receiver.offset.use_slope_bias) << 2)
            | (u32::from(receiver.offset.slope_post_projection) << 3);
        value.view_row2 = Vec4::from_array(receiver.view_row2);
        value.cascade_fars = Vec4::from_array(receiver.cascade_fars);
        value.sun_direction = Vec4::new(
            receiver.offset.sun_direction[0],
            receiver.offset.sun_direction[1],
            receiver.offset.sun_direction[2],
            0.0,
        );
        value.shadow_size = Vec4::new(receiver.shadow_size[0], receiver.shadow_size[1], 0.0, 0.0);
        value.bias = Vec4::new(
            receiver.offset.normal_offset,
            receiver.offset.bias,
            receiver.offset.slope_bias,
            0.0,
        );
        value.config = UVec4::new(1, receiver.directional_light_index, flags, count as u32);
        value.quality = IVec4::new(receiver.light_quality, receiver.filtering_quality, 0, 0);
        Ok(value)
    }
}

#[derive(Resource, Clone, ExtractResource)]
pub struct NativeParallelSamplingBuffer {
    pub buffer: Handle<ShaderBuffer>,
    pub unavailable_depth: Handle<Image>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn receiver() -> crate::native_shadow_scene::NativeParallelReceiverVars {
        crate::native_shadow_scene::NativeParallelReceiverVars {
            path: crate::render_shadow::ParallelShadowReceiverPath::DirectVisibility,
            world_to_shadow: vec![std::array::from_fn(|r| {
                std::array::from_fn(|c| (r * 4 + c) as f32)
            })],
            view_row2: [0.0, 0.0, -1.0, 0.0],
            cascade_fars: [4.0, 12.0, 30.0, 75.0],
            shadow_size: [256.0, 256.0],
            light_quality: 2,
            filtering_quality: 3,
            directional_light_index: 0,
            offset: NativeShadowOffsetParameters {
                sun_direction: [0.0, 0.0, -1.0],
                use_normal_offset: true,
                use_slope_scale: false,
                use_slope_bias: true,
                slope_post_projection: false,
                use_uv_normal_offset: false,
                normal_offset: 0.01,
                bias: 0.001,
                slope_bias: 0.002,
            },
        }
    }

    #[test]
    fn transport_preserves_original_rows_and_resolved_options() {
        let input = receiver();
        let uniform = NativeParallelSamplingUniform::from_receiver(&input).unwrap();
        assert_eq!(
            uniform.world_to_shadow[0].to_cols_array_2d(),
            input.world_to_shadow[0]
        );
        assert_eq!(uniform.config.to_array(), [1, 0, 5, 1]);
        assert_eq!(uniform.quality.to_array(), [2, 3, 0, 0]);
        assert_eq!(uniform.bias.to_array(), [0.01, 0.001, 0.002, 0.0]);
        let neutral = NativeParallelSamplingUniform::default();
        assert_eq!(neutral.config.x, 0);
        let buffer = ShaderBuffer::from(neutral);
        assert_eq!(buffer.data.unwrap().len(), 432);
    }

    #[test]
    fn opaque_screen_visibility_is_not_replaced_by_direct_atlas_sampling() {
        let mut input = receiver();
        for path in [
            crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace,
            crate::render_shadow::ParallelShadowReceiverPath::DirectWithCasterDistance,
        ] {
            input.path = path;
            assert!(NativeParallelSamplingUniform::from_receiver(&input)
                .unwrap_err()
                .contains("scalar atlas transport"));
        }
    }

    #[test]
    fn screen_producer_transport_keeps_rows_but_disables_vertex_normal_offset() {
        let mut input = receiver();
        assert!(NativeParallelSamplingUniform::from_screen_receiver(&input).is_err());
        input.path = crate::render_shadow::ParallelShadowReceiverPath::ScreenSpace;
        let uniform = NativeParallelSamplingUniform::from_screen_receiver(&input).unwrap();
        assert_eq!(uniform.config.to_array(), [2, 0, 4, 1]);
        assert_eq!(
            uniform.world_to_shadow[0].to_cols_array_2d(),
            input.world_to_shadow[0]
        );
        assert_eq!(uniform.bias.to_array(), [0.01, 0.001, 0.002, 0.0]);
        assert!(NativeParallelSamplingUniform::from_receiver(&input).is_err());
        input.offset.use_uv_normal_offset = true;
        assert!(NativeParallelSamplingUniform::from_screen_receiver(&input).is_err());
    }

    #[test]
    fn unresolved_offset_branch_is_not_silently_substituted() {
        let mut input = receiver();
        input.offset.use_uv_normal_offset = true;
        assert!(NativeParallelSamplingUniform::from_receiver(&input)
            .unwrap_err()
            .contains("unsupported"));
        input.offset.use_uv_normal_offset = false;
        input.offset.bias = f32::NAN;
        assert!(NativeParallelSamplingUniform::from_receiver(&input).is_err());
        input.offset.bias = 0.0;
        input.world_to_shadow.clear();
        assert!(NativeParallelSamplingUniform::from_receiver(&input).is_err());
    }
}
