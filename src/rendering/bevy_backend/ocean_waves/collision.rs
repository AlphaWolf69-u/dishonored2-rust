//! Source finite-mesh rasterization into the prewarped R16F height texture.
use super::*;

pub(super) struct WaveCollision {
    texture: Texture,
    view: TextureView,
    vertices: Buffer,
    indices: Buffer,
    index_count: u32,
    pipeline: RenderPipeline,
    bindings: [BindGroup; 2],
}
impl WaveCollision {
    pub(super) fn new(
        device: &RenderDevice,
        displacement: [&TextureView; 2],
    ) -> Result<Self, String> {
        let mesh = crate::rendering::ocean::build_wave_collision_mesh(WAVE_RESOLUTION >> 2)?;
        let index_count = mesh.indices.len() as u32;
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("ocean collision mesh vertices"),
            usage: BufferUsages::VERTEX,
            contents: &words(mesh.vertices.into_iter().flatten().map(f32::to_bits)),
        });
        let indices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("ocean collision mesh indices"),
            usage: BufferUsages::INDEX,
            contents: &words(mesh.indices),
        });
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("ocean collision height"),
            size: Extent3d {
                width: WAVE_RESOLUTION,
                height: WAVE_RESOLUTION,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            // Source packed 0x5c2 maps to DXGI54/R16_FLOAT.
            format: TextureFormat::R16Float,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&default());
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("ocean collision height"),
            source: ShaderSource::Wgsl(
                include_str!("../../../../assets/shaders/dh2_ocean_collision.wgsl").into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("ocean finite collision-height raster"),
            layout: None,
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("collision_vertex"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: 8,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[VertexAttribute {
                        format: VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
                }],
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("collision_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::R16Float,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleStrip,
                strip_index_format: Some(IndexFormat::Uint32),
                cull_mode: None,
                ..default()
            },
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0).into();
        // This shared helper implements the installed bilinearSampler declaration
        // (linear min/mag, point mip, wrap); its existing shadow use has identical state.
        let sampler = super::super::shadow_pipeline::shadow_alpha_sampler(device);
        let bindings = displacement.map(|source| {
            device.create_bind_group(
                "ocean collision input",
                &layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: BindingResource::TextureView(source),
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::Sampler(&sampler),
                    },
                ],
            )
        });
        Ok(Self {
            texture,
            view,
            vertices,
            indices,
            index_count,
            pipeline,
            bindings,
        })
    }
    pub(super) fn record(&self, encoder: &mut CommandEncoder, index: usize) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("ocean collision height"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                depth_slice: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bindings[index], &[]);
        pass.set_vertex_buffer(0, *self.vertices.slice(..));
        pass.set_index_buffer(*self.indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
    pub(super) fn texture(&self) -> &Texture {
        &self.texture
    }
    pub(super) fn view(&self) -> &TextureView {
        &self.view
    }
    pub(super) fn buffer_bytes(&self) -> u64 {
        self.vertices.size() + self.indices.size()
    }
}
