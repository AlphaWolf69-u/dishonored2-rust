//! One full-screen display pass, intended to replace backend tone mapping.
use super::ToneMappingParameters;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};

pub struct ToneMappingPass {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
    parameters: Buffer,
    linear: Sampler,
    binding: Option<((TextureViewId, TextureViewId, BufferId), BindGroup)>,
    last_parameters: Option<[u8; 80]>,
}
impl ToneMappingPass {
    pub fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        let mut entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(80),
            },
        }];
        entries.extend((1..=2).map(|binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float {
                    filterable: binding == 2,
                },
                view_dimension: if binding == 1 {
                    TextureViewDimension::D2
                } else {
                    TextureViewDimension::D3
                },
                multisampled: false,
            },
        }));
        entries.push(BindGroupLayoutEntry {
            binding: 3,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Sampler(SamplerBindingType::Filtering),
        });
        entries.push(BindGroupLayoutEntry {
            binding: 4,
            visibility: ShaderStages::FRAGMENT,
            count: None,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(32),
            },
        });
        let layout = device.create_bind_group_layout(Some("display transform inputs"), &entries);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("display transform"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("display transform"),
            source: ShaderSource::Wgsl(
                format!(
                    "{}\n{}",
                    super::SHADER_SOURCE,
                    include_str!("../../../../assets/shaders/dh2_tone_mapping_pass.wgsl")
                )
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("filmic colour-cube display"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("display_vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("display_fragment"),
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
            layout,
            pipeline,
            parameters: device.create_buffer(&BufferDescriptor {
                label: Some("display transform parameters"),
                size: 80,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            linear: device.create_sampler(&SamplerDescriptor {
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..Default::default()
            }),
            binding: None,
            last_parameters: None,
        }
    }
    /// `exposure` is the existing auto-exposure state buffer (32 bytes).
    /// Parameters.exposure undoes host camera units and applies custom EV;
    /// the shader multiplies it by the current GPU EngineLuminanceFactor.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &ToneMappingParameters,
        hdr: &TextureView,
        cube: &TextureView,
        exposure: &Buffer,
        target: &TextureView,
    ) {
        let bytes = parameters.uniform_bytes();
        if self.last_parameters.as_ref() != Some(&bytes) {
            queue.write_buffer(&self.parameters, 0, &bytes);
            self.last_parameters = Some(bytes);
        }
        let ids = (hdr.id(), cube.id(), exposure.id());
        if self.binding.as_ref().is_none_or(|b| b.0 != ids) {
            let binding = device.create_bind_group(
                Some("display transform sources"),
                &self.layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: self.parameters.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(hdr),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(cube),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Sampler(&self.linear),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: exposure.as_entire_binding(),
                    },
                ],
            );
            self.binding = Some((ids, binding));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("filmic display transform"),
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
        pass.draw(0..3, 0..1);
    }
}
