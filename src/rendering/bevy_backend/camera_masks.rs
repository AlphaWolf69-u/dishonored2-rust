//! 3185e0's ordered near-mask generation, H/V blur, and combined far mask.
//! Inputs are the completed same-view alpha depth and autofocus resources.
use super::{
    camera_fuzziness::{CameraFuzzinessInputs, CameraFuzzinessParameters, CameraFuzzinessPass},
    camera_mask_blur::CameraMaskBlurPass,
};
use crate::rendering::{
    camera_effects::CameraSettings,
    depth_of_field::{DofPass, NearMaskBlur},
};
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
#[cfg(test)]
#[path = "camera_masks_tests.rs"]
mod tests;

pub struct CameraMasks {
    extent: [u32; 2],
    near_extent: [u32; 2],
    near: Texture,
    near_view: TextureView,
    _temporary: Texture,
    temporary_view: TextureView,
    far: Texture,
    far_view: TextureView,
    near_pass: CameraFuzzinessPass,
    far_pass: CameraFuzzinessPass,
    blur: [CameraMaskBlurPass; 2],
}
pub struct CameraMaskInputs<'a> {
    pub settings: &'a CameraSettings,
    pub alpha_depth: &'a TextureView,
    pub autofocus: &'a TextureView,
    pub inverse_z: [f32; 3],
    pub resolution_scale: [f32; 2],
    pub near_mask_gain: f32,
    pub far_clip: f32,
    pub near_viewport: [u32; 4],
    pub near_scissor: [u32; 4],
    pub far_viewport: [u32; 4],
    pub far_scissor: [u32; 4],
}
impl CameraMasks {
    /// `extent` is the allocated dofMaskDiv2 size. Source quality and view
    /// resource allocation remain with the frame owner, as in the game.
    pub fn new(device: &RenderDevice, extent: [u32; 2]) -> Result<Self, String> {
        if extent.contains(&0) {
            return Err("camera masks require a nonempty target".into());
        }
        let near_extent = extent.map(|size| size.div_ceil(2).max(4));
        let texture = |label, size: [u32; 2], format| {
            device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let near = texture("camera near mask", near_extent, TextureFormat::R16Float);
        let near_view = near.create_view(&Default::default());
        let temporary = texture(
            "camera near mask horizontal",
            near_extent,
            TextureFormat::R16Float,
        );
        let temporary_view = temporary.create_view(&Default::default());
        let far = texture("camera far mask", extent, TextureFormat::Rg8Unorm);
        let far_view = far.create_view(&Default::default());
        Ok(Self {
            extent,
            near_extent,
            near,
            near_view,
            _temporary: temporary,
            temporary_view,
            far,
            far_view,
            near_pass: CameraFuzzinessPass::new(device, TextureFormat::R16Float),
            far_pass: CameraFuzzinessPass::new(device, TextureFormat::Rg8Unorm),
            blur: CameraMaskBlurPass::pair(device, TextureFormat::R16Float),
        })
    }
    pub fn matches(&self, extent: [u32; 2]) -> bool {
        self.extent == extent
    }
    pub fn near_texture(&self) -> &Texture {
        &self.near
    }
    pub fn far_texture(&self) -> &Texture {
        &self.far
    }
    pub fn near_view(&self) -> &TextureView {
        &self.near_view
    }
    pub fn far_view(&self) -> &TextureView {
        &self.far_view
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        inputs: CameraMaskInputs<'_>,
    ) {
        let optics = inputs.settings.combine(inputs.far_clip);
        let flags = inputs.settings.range_flags();
        let mut parameters = CameraFuzzinessParameters {
            pass: DofPass::Near,
            near: optics.near,
            // Only the near/far branches execute in this task.
            alpha: [0.; 3],
            far: optics.far,
            inverse_z: inputs.inverse_z,
            centre: optics.centre,
            resolution_scale: inputs.resolution_scale,
            autofocus: inputs.settings.autofocus_enabled(),
        };
        if flags & 1 == 0 {
            self.near_pass.record_region(
                device,
                queue,
                encoder,
                &parameters,
                CameraFuzzinessInputs {
                    alpha_depth: inputs.alpha_depth,
                    // This binding is not sampled by the near shader branch.
                    near_mask: &self.far_view,
                    autofocus: inputs.autofocus,
                },
                &self.near_view,
                Some((inputs.near_viewport, inputs.near_scissor)),
            );
            let [horizontal, vertical] =
                NearMaskBlur::separable(self.near_extent, inputs.near_mask_gain)
                    .expect("allocated near-mask extent");
            self.blur[0].record_region(
                device,
                queue,
                encoder,
                &horizontal,
                inputs.resolution_scale,
                &self.near_view,
                &self.temporary_view,
                Some((inputs.near_viewport, inputs.near_scissor)),
            );
            self.blur[1].record_region(
                device,
                queue,
                encoder,
                &vertical,
                inputs.resolution_scale,
                &self.temporary_view,
                &self.near_view,
                Some((inputs.near_viewport, inputs.near_scissor)),
            );
        } else {
            Self::clear(encoder, &self.near_view);
        }
        if flags & 2 == 0 {
            parameters.pass = DofPass::Far;
            parameters.autofocus &= flags & 4 == 0;
            self.far_pass.record_region(
                device,
                queue,
                encoder,
                &parameters,
                CameraFuzzinessInputs {
                    alpha_depth: inputs.alpha_depth,
                    near_mask: &self.near_view,
                    autofocus: inputs.autofocus,
                },
                &self.far_view,
                Some((inputs.far_viewport, inputs.far_scissor)),
            );
        } else {
            Self::clear(encoder, &self.far_view);
        }
    }
    fn clear(encoder: &mut CommandEncoder, target: &TextureView) {
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("disabled camera mask"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(Default::default()),
                    store: StoreOp::Store,
                },
            })],
            ..Default::default()
        });
    }
}
