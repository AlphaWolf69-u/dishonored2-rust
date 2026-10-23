//! Paired near-mask blur owners; each queued draw keeps its own constants.
use crate::rendering::depth_of_field::NearMaskBlur;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};
use std::sync::Arc;
pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_camera_mask_blur.wgsl");
#[cfg(test)]
#[path = "camera_mask_blur_tests.rs"]
mod tests;
struct BlurPipeline {
    pipeline: RenderPipeline,
    layout: BindGroupLayout,
    linear: Sampler,
}
pub struct CameraMaskBlurPass {
    shared: Arc<BlurPipeline>,
    uniform: Buffer,
    binding: Option<(TextureViewId, BindGroup)>,
}
impl CameraMaskBlurPass {
    pub fn pair(device: &RenderDevice, format: TextureFormat) -> [Self; 2] {
        use binding_types::*;
        let layout = device.create_bind_group_layout(
            Some("camera mask blur inputs"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                (
                    uniform_buffer_sized(false, BufferSize::new(112)),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("camera mask blur"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("camera mask blur"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("camera mask blur"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("blur_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("blur_fragment"),
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
        let linear = device.create_sampler(&SamplerDescriptor {
            label: Some("camera mask linear clamp"),
            min_filter: FilterMode::Linear,
            mag_filter: FilterMode::Linear,
            ..Default::default()
        });
        let shared = Arc::new(BlurPipeline {
            pipeline,
            layout,
            linear,
        });
        std::array::from_fn(|_| Self {
            shared: shared.clone(),
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("camera mask blur parameters"),
                size: 112,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            binding: None,
        })
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &NearMaskBlur,
        resolution_scale: [f32; 2],
        source: &TextureView,
        target: &TextureView,
    ) {
        self.record_region(
            device,
            queue,
            encoder,
            parameters,
            resolution_scale,
            source,
            target,
            None,
        );
    }
    pub fn record_region(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &NearMaskBlur,
        resolution_scale: [f32; 2],
        source: &TextureView,
        target: &TextureView,
        region: Option<([u32; 4], [u32; 4])>,
    ) {
        let floats = parameters
            .offsets
            .iter()
            .flatten()
            .copied()
            .chain(parameters.weights)
            .chain(resolution_scale)
            .chain([0., 0.]);
        let mut bytes = [0; 112];
        for (i, value) in floats.enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
        if self.binding.as_ref().is_none_or(|v| v.0 != source.id()) {
            self.binding = Some((
                source.id(),
                device.create_bind_group(
                    Some("camera mask blur inputs"),
                    &self.shared.layout,
                    &BindGroupEntries::sequential((
                        self.uniform.as_entire_binding(),
                        source,
                        &self.shared.linear,
                    )),
                ),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("camera mask blur"),
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
        pass.set_pipeline(&self.shared.pipeline);
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
