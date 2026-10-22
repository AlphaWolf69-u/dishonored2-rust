//! Authored bloom upsampling used by 325d80. BloomChain owns persistent H/V
//! targets; its caller supplies the luminance source and exact UV constants.
use bevy::{
    math::Vec4,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/dh2_bloom.wgsl");

#[path = "bloom/chain.rs"]
mod chain;
pub use chain::{BloomChain, BloomViewParameters};

#[derive(Clone, Copy, Default, ShaderType)]
pub struct BloomParameters {
    pub pixel_scale: Vec4,
    pub viewport: Vec4,
    pub blur: Vec4,
}

pub struct BloomUpsamplePass {
    shared: std::sync::Arc<BloomPipeline>,
    parameters: UniformBuffer<BloomParameters>,
    binding: Option<(TextureViewId, BindGroup)>,
}
struct BloomPipeline {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
    mirror: Sampler,
}

impl BloomUpsamplePass {
    pub fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        use binding_types::*;
        let layout = device.create_bind_group_layout(
            Some("bloom upsample inputs"),
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<BloomParameters>(false),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );
        let program = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("bloom upsample"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("bloom upsample"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("bloom upsample"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &program,
                entry_point: Some("bloom_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &program,
                entry_point: Some("bloom_fragment"),
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
        Self {
            shared: std::sync::Arc::new(BloomPipeline {
                layout,
                pipeline,
                mirror: device.create_sampler(&SamplerDescriptor {
                    label: Some("bloom linear mirror"),
                    address_mode_u: AddressMode::MirrorRepeat,
                    address_mode_v: AddressMode::MirrorRepeat,
                    address_mode_w: AddressMode::MirrorRepeat,
                    min_filter: FilterMode::Linear,
                    mag_filter: FilterMode::Linear,
                    ..Default::default()
                }),
            }),
            parameters: UniformBuffer::default(),
            binding: None,
        }
    }

    fn sibling(&self) -> Self {
        Self {
            shared: self.shared.clone(),
            parameters: UniformBuffer::default(),
            binding: None,
        }
    }

    /// A pass owns one uniform upload. Use distinct owners for H/V draws
    /// submitted together so later writes cannot replace earlier parameters.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: BloomParameters,
        source: &TextureView,
        target: &TextureView,
    ) {
        self.parameters.set(parameters);
        self.parameters.write_buffer(device, queue);
        if self.binding.as_ref().is_none_or(|v| v.0 != source.id()) {
            self.binding = Some((
                source.id(),
                device.create_bind_group(
                    Some("bloom upsample source"),
                    &self.shared.layout,
                    &BindGroupEntries::sequential((
                        self.parameters.binding().unwrap(),
                        source,
                        &self.shared.mirror,
                    )),
                ),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("bloom upsample"),
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
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
#[path = "bloom/tests.rs"]
mod tests;
