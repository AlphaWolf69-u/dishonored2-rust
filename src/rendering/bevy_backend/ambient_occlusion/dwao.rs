//! Legacy two-depth-sample AO producer selected by r_hbao=0.
use super::*;
use crate::{coarse_depth::DepthPyramid, tiled_lighting_gpu::TlfPipelines};

pub struct DwaoPass {
    extent: [u32; 2],
    reduction: DepthPyramid,
    parameters: Buffer,
    binding: BindGroup,
    pipeline: RenderPipeline,
}
impl DwaoPass {
    pub fn new(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        source: &Texture,
    ) -> Result<Self, String> {
        let extent = [source.width(), source.height()];
        let reduction = DepthPyramid::new(device, pipelines, source)?;
        let parameters = device.create_buffer(&BufferDescriptor {
            label: Some("DWAO parameters"),
            size: 48,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(
            Some("DWAO inputs"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(48),
                    },
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Depth,
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    count: None,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: false },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                },
            ],
        );
        let depth = source.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..default()
        });
        let reduced = reduction.texture().create_view(&TextureViewDescriptor {
            base_mip_level: 0,
            mip_level_count: Some(1),
            ..default()
        });
        let binding = device.create_bind_group(
            Some("DWAO current depth"),
            &layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: parameters.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&depth),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(&reduced),
                },
            ],
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("DWAO"),
            source: ShaderSource::Wgsl(
                include_str!("../../../../assets/shaders/dh2_ao_dwao.wgsl").into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("DWAO"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("DWAO"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("dwao_vertex"),
                buffers: &[],
                compilation_options: default(),
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("dwao_occlusion"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba8Unorm,
                    blend: None,
                    write_mask: ColorWrites::RED,
                })],
            }),
            primitive: default(),
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            extent,
            reduction,
            parameters,
            binding,
            pipeline,
        })
    }
    pub fn record(
        &self,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        pipelines: &TlfPipelines,
        output: &TextureView,
        inverse_projection: [[f32; 4]; 4],
        render_scale: [f32; 2],
    ) -> Result<(), String> {
        self.reduction
            .record(queue, encoder, pipelines, self.extent, inverse_projection)?;
        let values = [
            self.extent[0] as f32 * 0.04,
            0.04,
            0.25,
            2.0,
            -inverse_projection[2][3],
            inverse_projection[3][2],
            inverse_projection[3][3],
            0.0,
            render_scale[0],
            render_scale[1],
            0.0,
            0.0,
        ];
        let bytes: Vec<_> = values.into_iter().flat_map(f32::to_le_bytes).collect();
        queue.write_buffer(&self.parameters, 0, &bytes);
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("DWAO occlusion"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: output,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
