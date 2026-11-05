//! Installed postfx/colorgrading, consumed by the original 140327270 post pass.
//! The caller owns HDR ping-pong and the mask target from 1403222f0. Construct
//! this host pipeline lazily when the live effect owner requests grading;
//! owning a compiled masking material alone does not activate this pass.
use bevy::render::{render_resource::*, renderer::{RenderDevice, RenderQueue}};

pub struct MaskGradingParameters {
    pub grey_amount: f32,
    pub white_level: f32,
    pub compressed_hdr: bool,
    pub invert: bool,
}

pub struct MaskGradingPass {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
    parameters: Buffer,
    sampler: Sampler,
    binding: Option<((TextureViewId, TextureViewId, BufferId), BindGroup)>,
    last_parameters: Option<[u8; 16]>,
}

impl MaskGradingPass {
    pub fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(Some("mask grading inputs"), &[
            BindGroupLayoutEntry { binding: 0, visibility: ShaderStages::FRAGMENT, count: None,
                ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(16) } },
            BindGroupLayoutEntry { binding: 1, visibility: ShaderStages::FRAGMENT, count: None,
                ty: BindingType::Texture { sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2, multisampled: false } },
            BindGroupLayoutEntry { binding: 2, visibility: ShaderStages::FRAGMENT, count: None,
                ty: BindingType::Texture { sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2, multisampled: false } },
            BindGroupLayoutEntry { binding: 3, visibility: ShaderStages::FRAGMENT, count: None,
                ty: BindingType::Sampler(SamplerBindingType::Filtering) },
            BindGroupLayoutEntry { binding: 4, visibility: ShaderStages::FRAGMENT, count: None,
                ty: BindingType::Buffer { ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false, min_binding_size: BufferSize::new(32) } },
        ]);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("mask grading"), bind_group_layouts: &[Some(&layout)], immediate_size: 0,
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("mask grading"),
            source: ShaderSource::Wgsl(format!("{}\n{}\n{}",
                include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
                super::colour_output::SHADER_SOURCE,
                include_str!("../../../assets/shaders/dh2_mask_grading.wgsl")).into()),
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("mask grading"), layout: Some(&pipeline_layout),
            vertex: RawVertexState { module: &module, entry_point: Some("mask_grade_vertex"),
                compilation_options: Default::default(), buffers: &[] },
            fragment: Some(RawFragmentState { module: &module, entry_point: Some("mask_grade_fragment"),
                compilation_options: Default::default(), targets: &[Some(ColorTargetState {
                    format, blend: None, write_mask: ColorWrites::ALL,
                })] }),
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(),
            multiview_mask: None, cache: None,
        });
        let parameters = device.create_buffer(&BufferDescriptor {
            label: Some("mask grading parameters"), size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&SamplerDescriptor {
            label: Some("mask grading bilinear clamp"),
            address_mode_u: AddressMode::ClampToEdge, address_mode_v: AddressMode::ClampToEdge,
            min_filter: FilterMode::Linear, mag_filter: FilterMode::Linear, ..Default::default()
        });
        Self { layout, pipeline, parameters, sampler, binding: None, last_parameters: None }
    }

    /// The caller supplies distinct HDR source/destination views. Parameters
    /// and bindings upload only when changed; no target allocation occurs here.
    pub fn record(&mut self, device: &RenderDevice, queue: &RenderQueue,
        encoder: &mut CommandEncoder, parameters: &MaskGradingParameters,
        hdr: &TextureView, mask: &TextureView, adaptation: &Buffer, target: &TextureView,
    ) -> Result<(), String> {
        if hdr.id() == target.id() || mask.id() == target.id() {
            return Err("mask grading requires a distinct output attachment".into());
        }
        let mut bytes = [0; 16];
        for (index, value) in [parameters.grey_amount, parameters.white_level,
            f32::from(u8::from(parameters.compressed_hdr)), f32::from(u8::from(parameters.invert))]
            .into_iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        if self.last_parameters != Some(bytes) {
            queue.write_buffer(&self.parameters, 0, &bytes);
            self.last_parameters = Some(bytes);
        }
        let key = (hdr.id(), mask.id(), adaptation.id());
        if self.binding.as_ref().is_none_or(|(old, _)| *old != key) {
            let binding = device.create_bind_group(Some("mask grading inputs"), &self.layout, &[
                BindGroupEntry { binding: 0, resource: self.parameters.as_entire_binding() },
                BindGroupEntry { binding: 1, resource: BindingResource::TextureView(hdr) },
                BindGroupEntry { binding: 2, resource: BindingResource::TextureView(mask) },
                BindGroupEntry { binding: 3, resource: BindingResource::Sampler(&self.sampler) },
                BindGroupEntry { binding: 4, resource: adaptation.as_entire_binding() },
            ]);
            self.binding = Some((key, binding));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("mask grading"),
            color_attachments: &[Some(RenderPassColorAttachment { view: target, depth_slice: None,
                resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } })],
            depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
