//! Authored FSdisto draw in the existing screen vector-field owner.
//!
//! Installed PostFX_uberMotionVector, wobble/camera PS checksum
//! 935452d7cb0fe0552975f05940d5e5c3; all-five PS
//! f0ec2b7fc743613ff5729aee2a854e59. 3586a0 binds current/previous matrices,
//! inverse projection, time/group1 and localTime independently; 374e40 owns
//! history. No history or images are acquired here.
use super::{passes::pipeline, FullscreenInputs};
use bevy::{math::Mat4, render::{render_resource::*, renderer::{RenderDevice, RenderQueue}}};

pub(super) struct FullscreenDistortion {
    uniform: Buffer,
    pub pipeline: RenderPipeline,
    pub binding: Option<((BufferId, TextureViewId), BindGroup)>,
}
impl FullscreenDistortion {
    pub fn new(device: &RenderDevice, state: &crate::render_state::RenderState) -> Result<Self, String> {
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("fullscreen FX distortion"),
            source: ShaderSource::Wgsl(include_str!("../../../../assets/shaders/dh2_fullscreen_distortion.wgsl").into()),
        });
        let pipeline = pipeline(device, &module, None, "fullscreen FX distortion",
            "fullscreen_distortion_vertex", "fullscreen_distortion_fragment",
            TextureFormat::Rgba16Float, state)?;
        Ok(Self {
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("fullscreen FX distortion parameters"),
                size: 368,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            pipeline,
            binding: None,
        })
    }

    pub fn prepare(&mut self, device: &RenderDevice, queue: &RenderQueue,
        inputs: FullscreenInputs<'_>, extent: [u32; 2], depth: &TextureView,
        sampler: &Sampler, inverse_z: [f32; 3]) -> Result<(), String> {
        let frame = inputs.frame;
        let current = &inputs.matrices.current;
        let mut values = [[0.; 4]; 23];
        if frame.camera_blur {
            let inverse_current = Mat4::from_cols_array_2d(&current.view_projection).transpose().inverse();
            if !inverse_current.is_finite() {
                return Err("fullscreen distortion requires an invertible current view projection".into());
            }
            values[..4].copy_from_slice(&inverse_current.to_cols_array_2d());
            values[4..8].copy_from_slice(&Mat4::from_cols_array_2d(&inputs.matrices.previous.view_projection).transpose().to_cols_array_2d());
        }
        if frame.sand_storm {
            values[8..12].copy_from_slice(&Mat4::from_cols_array_2d(&current.view_projection).transpose().to_cols_array_2d());
        }
        // inverseViewMatrix's third column is the third row of this rigid
        // orthonormal world-to-view transform; original shader negates it.
        values[12] = [-current.view[2][0], -current.view[2][1], -current.view[2][2], 0.];
        values[13] = [extent[0] as f32, extent[1] as f32, inputs.shader_time, inputs.local_time];
        values[14] = [inverse_z[0], inverse_z[1], inverse_z[2], 0.];
        values[15] = [frame.wobble_low_bound, frame.wobble_high_bound, frame.wobble_speed, frame.wobble_strength];
        values[16] = [frame.radial_blur_dead_zone, frame.radial_blur_strength, frame.radial_disto_strength, frame.lens_strength];
        values[17] = [frame.sand_storm_dead_zone, frame.sand_storm_start_depth, frame.sand_storm_end_depth, 0.];
        values[18] = frame.sand_storm_start_value;
        values[19] = frame.sand_storm_end_value;
        values[20] = [frame.sand_storm_world_wind[0], frame.sand_storm_world_wind[1], frame.sand_storm_world_wind[2], frame.camera_blur_strength];
        values[21] = [frame.camera_blur_start_depth, frame.camera_blur_end_depth, 0., 0.];
        values[22] = [u32::from(frame.radial_blur), u32::from(frame.lens), u32::from(frame.wobble),
            u32::from(frame.sand_storm) | (u32::from(frame.camera_blur) << 1)].map(f32::from_bits);
        let mut bytes = [0; 368];
        for (destination, value) in bytes.chunks_exact_mut(4).zip(values.into_iter().flatten()) {
            destination.copy_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        let key = (self.uniform.id(), depth.id());
        if self.binding.as_ref().is_none_or(|binding| binding.0 != key) {
            self.binding = Some((key, device.create_bind_group(
                Some("fullscreen FX distortion inputs"), &self.pipeline.get_bind_group_layout(0).into(), &[
                    BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() },
                    BindGroupEntry { binding: 1, resource: BindingResource::TextureView(depth) },
                    BindGroupEntry { binding: 2, resource: BindingResource::Sampler(sampler) },
                ])));
        }
        Ok(())
    }
}
