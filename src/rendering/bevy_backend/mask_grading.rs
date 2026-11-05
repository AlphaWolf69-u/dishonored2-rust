//! Installed postfx/colorgrading: 1403134e0, called before DoF by 140315450.
//! The caller owns HDR ping-pong and the mask target from 1403222f0. Construct
//! this host pipeline lazily when the live effect owner requests grading;
//! owning a compiled masking material alone does not activate this pass.
use bevy::render::{render_resource::*, renderer::{RenderDevice, RenderQueue}};

pub struct MaskGradingParameters {
    pub grey_amount: f32,
    pub white_level: f32,
    pub compressed_hdr: bool,
    pub invert: bool,
    pub black_white_mode: bool,
}

impl MaskGradingParameters {
    /// 1403134e0 bypasses this operation when the view amount is not positive
    /// or r_BlackAndWhiteMode is active (registered at 1413754e0).
    pub fn active(&self) -> bool {
        self.grey_amount > 0.0 && !self.black_white_mode
    }
}

/// 14036d0c0's full-resolution R8_UNORM mask (minimum eight per axis).
/// Kept with the consuming view, rather than in a global image registry.
struct MaskTarget {
    texture: Texture,
    view: TextureView,
    extent: [u32; 2],
}

impl MaskTarget {
    fn new(device: &RenderDevice, extent: [u32; 2]) -> Self {
        let extent = extent.map(|axis| axis.max(8));
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("bend-time mask"),
            size: Extent3d { width: extent[0], height: extent[1], depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: TextureDimension::D2,
            format: TextureFormat::R8Unorm,
            // COPY_SRC permits the existing numeric capture owner to inspect
            // this target; ordinary rendering schedules no copy.
            usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&TextureViewDescriptor::default());
        Self { texture, view, extent }
    }
}

pub struct MaskGradingPass {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
    parameters: Buffer,
    sampler: Sampler,
    binding: Option<((TextureViewId, TextureViewId, BufferId), BindGroup)>,
    last_parameters: Option<[u8; 16]>,
    mask: Option<MaskTarget>,
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
        Self { layout, pipeline, parameters, sampler, binding: None, last_parameters: None, mask: None }
    }

    /// Invoke only for an active consuming view. Resizing releases old input
    /// bindings before replacing the target, so they cannot pin the old image.
    pub fn prepare_mask(&mut self, device: &RenderDevice, extent: [u32; 2]) -> &TextureView {
        let extent = extent.map(|axis| axis.max(8));
        if self.mask.as_ref().is_none_or(|mask| mask.extent != extent) {
            self.binding = None;
            self.mask = Some(MaskTarget::new(device, extent));
        }
        &self.mask.as_ref().unwrap().view
    }

    /// 1403222f0 clears the target even when its masking-surface list is empty.
    /// It leaves scene depth intact for the subsequent surface draws.
    pub fn clear_mask(&self, encoder: &mut CommandEncoder) -> Result<(), String> {
        let mask = self.mask.as_ref().ok_or("mask target was not prepared")?;
        let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("clear bend-time mask"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &mask.view, depth_slice: None, resolve_target: None,
                ops: Operations { load: LoadOp::Clear(Default::default()), store: StoreOp::Store },
            })],
            depth_stencil_attachment: None, timestamp_writes: None,
            occlusion_query_set: None, multiview_mask: None,
        });
        Ok(())
    }

    /// The compiled pass may remain cached while its view is inactive; release
    /// target and sampled input references together at that transition.
    pub fn release_view(&mut self) {
        self.binding = None;
        self.mask = None;
    }

    pub fn mask_texture(&self) -> Option<&Texture> {
        self.mask.as_ref().map(|mask| &mask.texture)
    }

    pub fn record_prepared_mask(&mut self, device: &RenderDevice, queue: &RenderQueue,
        encoder: &mut CommandEncoder, parameters: &MaskGradingParameters,
        hdr: &TextureView, adaptation: &Buffer, target: &TextureView,
    ) -> Result<(), String> {
        if !parameters.active() { self.release_view(); return Ok(()); }
        let mask = self.mask.as_ref().ok_or("mask target was not prepared")?.view.clone();
        self.record(device, queue, encoder, parameters, hdr, &mask, adaptation, target)
    }

    /// The caller supplies distinct HDR source/destination views. Parameters
    /// and bindings upload only when changed; no target allocation occurs here.
    pub fn record(&mut self, device: &RenderDevice, queue: &RenderQueue,
        encoder: &mut CommandEncoder, parameters: &MaskGradingParameters,
        hdr: &TextureView, mask: &TextureView, adaptation: &Buffer, target: &TextureView,
    ) -> Result<(), String> {
        if !parameters.active() { self.release_view(); return Ok(()); }
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
