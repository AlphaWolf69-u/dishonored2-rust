//! Actual depth-driven near/far/transparent masks for camera blur.
//! The frame owner supplies alpha depth, autofocus and the blurred near mask.
use crate::rendering::depth_of_field::DofPass;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_camera_fuzziness.wgsl")
);
#[cfg(test)]
#[path = "camera_fuzziness_tests.rs"]
mod tests;
#[derive(Clone, Debug)]
pub struct CameraFuzzinessParameters {
    pub pass: DofPass,
    pub near: [f32; 2],
    /// Alpha sharp distance, fuzzy distance and bias, from the frame owner.
    pub alpha: [f32; 3],
    pub far: [f32; 4],
    /// Inverse projection [2].w, [3].w and [3].z.
    pub inverse_z: [f32; 3],
    pub centre: [f32; 2],
    pub resolution_scale: [f32; 2],
    pub autofocus: bool,
}
impl CameraFuzzinessParameters {
    pub fn uniform_bytes(&self) -> [u8; 80] {
        let mut bytes = [0; 80];
        let floats = self
            .near
            .into_iter()
            .chain(self.alpha[..2].iter().copied())
            .chain(self.far)
            .chain(self.inverse_z)
            .chain([self.alpha[2]])
            .chain(self.centre)
            .chain(self.resolution_scale);
        let pass = match self.pass {
            DofPass::Alpha => 0,
            DofPass::Near => 1,
            DofPass::Far => 2,
        };
        for (i, word) in floats
            .map(f32::to_bits)
            .chain([pass, u32::from(self.autofocus), 0, 0])
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }
}
pub struct CameraFuzzinessInputs<'a> {
    pub alpha_depth: &'a TextureView,
    pub near_mask: &'a TextureView,
    pub autofocus: &'a TextureView,
}
pub struct CameraFuzzinessPass {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    uniform: Buffer,
    linear: Sampler,
    binding: Option<([TextureViewId; 3], BindGroup)>,
}
impl CameraFuzzinessPass {
    /// Near writes R16F; far and transparent masks write RG8. The caller owns
    /// render targets and supplies the exact format selected by its schedule.
    pub fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        use binding_types::*;
        let layout = device.create_bind_group_layout(
            Some("camera fuzziness inputs"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                (
                    uniform_buffer_sized(false, BufferSize::new(80)),
                    texture_depth_2d(),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    texture_2d(TextureSampleType::Float { filterable: false }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("camera fuzziness"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("camera fuzziness"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("camera fuzziness"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("fuzz_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("fuzz_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState {
                    format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&BufferDescriptor {
            label: Some("camera fuzziness parameters"),
            size: 80,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let linear = device.create_sampler(&SamplerDescriptor {
            label: Some("camera mask linear clamp"),
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            ..Default::default()
        });
        Self {
            pipeline,
            layout,
            uniform,
            linear,
            binding: None,
        }
    }
    /// Separate owners are required when multiple masks are queued together.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &CameraFuzzinessParameters,
        inputs: CameraFuzzinessInputs<'_>,
        target: &TextureView,
    ) {
        self.record_region(device, queue, encoder, parameters, inputs, target, None);
    }
    pub fn record_region(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &CameraFuzzinessParameters,
        inputs: CameraFuzzinessInputs<'_>,
        target: &TextureView,
        region: Option<([u32; 4], [u32; 4])>,
    ) {
        queue.write_buffer(&self.uniform, 0, &parameters.uniform_bytes());
        let images = [inputs.alpha_depth, inputs.near_mask, inputs.autofocus];
        let key = images.map(TextureView::id);
        if self.binding.as_ref().is_none_or(|v| v.0 != key) {
            let group = device.create_bind_group(
                Some("camera fuzziness inputs"),
                &self.layout,
                &BindGroupEntries::sequential((
                    self.uniform.as_entire_binding(),
                    images[0],
                    images[1],
                    images[2],
                    &self.linear,
                )),
            );
            self.binding = Some((key, group));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("camera fuzziness"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        if let Some((viewport, scissor)) = region {
            pass.set_viewport(
                viewport[0] as f32,
                viewport[1] as f32,
                viewport[2] as f32,
                viewport[3] as f32,
                0.,
                1.,
            );
            pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
        }
        pass.draw(0..3, 0..1);
    }
}
