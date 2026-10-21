//! Same-view depthWater attachment and 2fe850/2feef0 copy lifecycle.
//! Geometry producers draw into `attachment` between prepare and restoration;
//! this owner does not claim that opaque depth alone is completed water depth.
use super::tiled_lighting_gpu::TlfPipelines;
use bevy::{
    prelude::default,
    render::{render_resource::*, renderer::RenderDevice},
};

pub struct WaterDepthPass {
    pub texture: Texture,
    pub attachment: TextureView,
    pub sample: TextureView,
    copy: RenderPipeline,
    restore: RenderPipeline,
    vertices: Buffer,
    point: Sampler,
    source: Option<(TextureId, BindGroup, BindGroup)>,
    prepared: bool,
}

impl WaterDepthPass {
    pub fn new(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        extent: [u32; 2],
        format: TextureFormat,
    ) -> Result<Self, String> {
        if !format.has_depth_aspect() || !format.has_stencil_aspect() || extent.contains(&0) {
            return Err("water depth requires a nonempty depth/stencil target".into());
        }
        let attributes = [
            VertexAttribute {
                format: VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            VertexAttribute {
                format: VertexFormat::Float32x2,
                offset: 12,
                shader_location: 1,
            },
        ];
        let layout = RawVertexBufferLayout {
            array_stride: 20,
            step_mode: VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let copy =
            pipelines.depth_copy_pipeline(device, format, false, std::slice::from_ref(&layout));
        let restore = pipelines.depth_copy_pipeline_with_state(
            device,
            false,
            &[layout],
            super::pipeline_state::depth_stencil_gpu(
                &crate::render_state::boat_depth_restore_state(),
                format,
            )?,
        );
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("depthWater"),
            size: Extent3d {
                width: extent[0],
                height: extent[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let attachment = texture.create_view(&default());
        let sample = texture.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..default()
        });
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("water depth fullscreen triangle"),
            contents: &[
                [-1.0f32, 1., 0., 0., 0.],
                [3., 1., 0., 2., 0.],
                [-1., -3., 0., 0., 2.],
            ]
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
            usage: BufferUsages::VERTEX,
        });
        Ok(Self {
            texture,
            attachment,
            sample,
            copy,
            restore,
            vertices,
            point: device.create_sampler(&SamplerDescriptor {
                label: Some("water depth point clamp"),
                address_mode_u: AddressMode::ClampToEdge,
                address_mode_v: AddressMode::ClampToEdge,
                ..default()
            }),
            source: None,
            prepared: false,
        })
    }

    /// Supply immutable opaque depth from this view, before any water or boat
    /// draws. The alternate task branch preserves stencil; ordinary clears it.
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        opaque: &Texture,
        alternate_path: bool,
    ) -> Result<(), String> {
        self.prepared = false;
        if opaque.id() == self.texture.id()
            || opaque.width() != self.texture.width()
            || opaque.height() != self.texture.height()
            || opaque.dimension() != TextureDimension::D2
            || opaque.depth_or_array_layers() != 1
            || opaque.sample_count() != 1
            || !opaque.format().has_depth_aspect()
            || !opaque.usage().contains(TextureUsages::TEXTURE_BINDING)
        {
            return Err("water depth requires immutable full-size same-view opaque depth".into());
        }
        if self
            .source
            .as_ref()
            .is_none_or(|(id, _, _)| *id != opaque.id())
        {
            let view = opaque.create_view(&TextureViewDescriptor {
                aspect: TextureAspect::DepthOnly,
                ..default()
            });
            let bind = |pipeline: &RenderPipeline| {
                device.create_bind_group(
                    Some("water opaque depth"),
                    &pipeline.get_bind_group_layout(0).into(),
                    &[
                        BindGroupEntry {
                            binding: 0,
                            resource: BindingResource::TextureView(&view),
                        },
                        BindGroupEntry {
                            binding: 1,
                            resource: BindingResource::Sampler(&self.point),
                        },
                    ],
                )
            };
            self.source = Some((opaque.id(), bind(&self.copy), bind(&self.restore)));
        }
        self.record(
            encoder,
            &self.copy,
            &self.source.as_ref().unwrap().1,
            !alternate_path,
            0,
        );
        self.prepared = true;
        Ok(())
    }

    /// 2feef0: after the boat and water depth draws, restore opaque depth only
    /// where (stencil & 12)==8. All stencil bits and other depths survive.
    pub fn restore_boat_depth(&self, encoder: &mut CommandEncoder) -> Result<(), String> {
        if !self.prepared {
            return Err("boat depth restoration requires this view's opaque copy first".into());
        }
        self.record(
            encoder,
            &self.restore,
            &self.source.as_ref().unwrap().2,
            false,
            8,
        );
        Ok(())
    }

    fn record(
        &self,
        encoder: &mut CommandEncoder,
        pipeline: &RenderPipeline,
        source: &BindGroup,
        clear_stencil: bool,
        reference: u32,
    ) {
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("water depth copy/boat restoration"),
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &self.attachment,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: Some(Operations {
                    load: if clear_stencil {
                        LoadOp::Clear(0)
                    } else {
                        LoadOp::Load
                    },
                    store: StoreOp::Store,
                }),
            }),
            ..default()
        });
        pass.set_pipeline(pipeline);
        pass.set_stencil_reference(reference);
        pass.set_bind_group(0, source, &[]);
        pass.set_vertex_buffer(0, *self.vertices.slice(..));
        pass.draw(0..3, 0..1);
    }
}
