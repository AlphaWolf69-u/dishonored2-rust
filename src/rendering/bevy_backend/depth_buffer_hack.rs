//! FUN_1402fcff0's "Depth buffer hack" pass. After the depth prepass, the
//! view depth/stencil is copied (FUN_1402bb270), then unhackDepthBuffer
//! rewrites the copy's stencil-one pixels (models with a positive
//! modelDepthHack) with depth * modelDepthHack, where modelDepthHack is
//! `1 / r_playerDepthHack` when the cvar is positive and 1 otherwise. Screen
//! shadows and AO reconstruct from the copy; the hacked view depth stays
//! bound for the forward passes.
use super::tiled_lighting_gpu::TlfPipelines;
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};

/// The unhack callback's modelDepthHack.x.
pub fn unhack_factor(player_depth_hack: f32) -> f32 {
    if player_depth_hack > 0.0 {
        1.0 / player_depth_hack
    } else {
        1.0
    }
}

/// The unhacked copy of one view depth texture and its draw resources.
pub struct DepthBufferHack {
    source_id: TextureId,
    pub texture: Texture,
    attachment: TextureView,
    pipeline: RenderPipeline,
    vertices: Buffer,
    factor: Buffer,
    binding: BindGroup,
}

impl DepthBufferHack {
    /// `source` must be single-sample, carry stencil, and allow COPY_SRC and
    /// TEXTURE_BINDING (the camera's depth texture usages).
    pub fn new(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        source: &Texture,
    ) -> Result<Self, String> {
        let format = source.format();
        if source.sample_count() != 1
            || !source
                .usage()
                .contains(TextureUsages::COPY_SRC | TextureUsages::TEXTURE_BINDING)
        {
            return Err("depth buffer hack needs a single-sample copyable sampled view depth".into());
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
        let pipeline =
            pipelines.unhack_depth_pipeline(device, format, std::slice::from_ref(&layout))?;
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("unhacked view depth"),
            size: source.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let attachment = texture.create_view(&TextureViewDescriptor::default());
        // One full-screen triangle; the program's UVs address the source.
        let triangle = [
            [-1.0f32, 1.0, 0.0, 0.0, 0.0],
            [3.0, 1.0, 0.0, 2.0, 0.0],
            [-1.0, -3.0, 0.0, 0.0, 2.0],
        ];
        let bytes: Vec<u8> = triangle.into_iter().flatten().flat_map(f32::to_le_bytes).collect();
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("depth buffer hack triangle"),
            contents: &bytes,
            usage: BufferUsages::VERTEX,
        });
        let factor = device.create_buffer(&BufferDescriptor {
            label: Some("unhackDepthBuffer modelDepthHack"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampled = source.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..Default::default()
        });
        let point = device.create_sampler(&SamplerDescriptor {
            label: Some("unhackDepthBuffer pointClamp"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            ..Default::default()
        });
        let binding: BindGroup = device.wgpu_device().create_bind_group(&BindGroupDescriptor {
            label: Some("unhackDepthBuffer inputs"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(&sampled),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(&point),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: factor.as_entire_binding(),
                },
            ],
        })
        .into();
        Ok(Self {
            source_id: source.id(),
            texture,
            attachment,
            pipeline,
            vertices,
            factor,
            binding,
        })
    }

    pub fn source_id(&self) -> TextureId {
        self.source_id
    }

    /// Copy the hacked view depth/stencil, then unhack its stencil-one pixels.
    pub fn record(
        &self,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        source: &Texture,
        player_depth_hack: f32,
    ) {
        let factor = [unhack_factor(player_depth_hack), 0.0, 0.0, 0.0];
        let bytes: Vec<u8> = factor.into_iter().flat_map(f32::to_le_bytes).collect();
        queue.write_buffer(&self.factor, 0, &bytes);
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            self.texture.as_image_copy(),
            source.size(),
        );
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("Depth buffer hack"),
            color_attachments: &[],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &self.attachment,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.set_stencil_reference(1);
        pass.set_vertex_buffer(0, *self.vertices.slice(..));
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::unhack_factor;

    #[test]
    fn unhack_factor_is_the_reciprocal_of_a_positive_cvar() {
        assert_eq!(unhack_factor(5.0), 0.2);
        for hack in [0.0, -0.0, -5.0] {
            assert_eq!(unhack_factor(hack), 1.0);
        }
    }
}
