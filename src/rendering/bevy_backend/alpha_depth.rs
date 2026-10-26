//! Full-target 2fbc60 depthAlpha preparation. The caller supplies completed
//! depthWater, including actual water geometry. This is not an opaque-depth
//! substitute, nor the later alpha/boat-volume geometry and stencil pass.
use super::{fog_targets::FogExtents, tiled_lighting_gpu::TlfPipelines};
use bevy::{
    prelude::default,
    render::{render_resource::*, renderer::RenderDevice},
};

pub struct AlphaDepthPass {
    pub full: Texture,
    pub reduced: Texture,
    pub full_attachment: TextureView,
    pub reduced_attachment: TextureView,
    pub full_sample: TextureView,
    pub reduced_sample: TextureView,
    extents: FogExtents,
    format: TextureFormat,
    copy: RenderPipeline,
    downscale: RenderPipeline,
    vertices: Buffer,
    point: Sampler,
    source: Option<(TextureId, BindGroup)>,
    reduction: BindGroup,
}

impl AlphaDepthPass {
    fn target(
        device: &RenderDevice,
        label: &'static str,
        size: bevy::math::UVec2,
        format: TextureFormat,
    ) -> (Texture, TextureView, TextureView) {
        let texture = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d {
                width: size.x,
                height: size.y,
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
        (texture, attachment, sample)
    }

    fn binding(
        device: &RenderDevice,
        pipeline: &RenderPipeline,
        source: &TextureView,
        sampler: &Sampler,
    ) -> BindGroup {
        device.create_bind_group(
            Some("alpha depth input"),
            &pipeline.get_bind_group_layout(0).into(),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::TextureView(source),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::Sampler(sampler),
                },
            ],
        )
    }

    pub fn new(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        extents: FogExtents,
        format: TextureFormat,
    ) -> Result<Self, String> {
        if !format.has_depth_aspect() || !format.has_stencil_aspect() {
            return Err("alpha depth requires the owning depth/stencil format".into());
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
        // Compact host vertex transport; the captured game stride is 32 bytes.
        let layout = RawVertexBufferLayout {
            array_stride: 20,
            step_mode: VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let copy =
            pipelines.depth_copy_pipeline(device, format, false, std::slice::from_ref(&layout));
        let downscale = pipelines.depth_copy_pipeline(device, format, true, &[layout]);
        let point = device.create_sampler(&SamplerDescriptor {
            label: Some("alpha depth point clamp"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            ..default()
        });
        let (full, full_attachment, full_sample) =
            Self::target(device, "depthAlpha", extents.full, format);
        //36f8a0 allocates this with the shared reduction quality. 2fbc60's
        //half-size viewport is separate from the allocated attachment size.
        let (reduced, reduced_attachment, reduced_sample) =
            Self::target(device, "depthAlphaDiv2", extents.depth, format);
        let reduction = Self::binding(device, &downscale, &full_sample, &point);
        let triangle = [
            [-1.0f32, 1., 0., 0., 0.],
            [3., 1., 0., 2., 0.],
            [-1., -3., 0., 0., 2.],
        ];
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("alpha depth fullscreen triangle"),
            contents: &triangle
                .into_iter()
                .flatten()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
            usage: BufferUsages::VERTEX,
        });
        Ok(Self {
            full,
            reduced,
            full_attachment,
            reduced_attachment,
            full_sample,
            reduced_sample,
            extents,
            format,
            copy,
            downscale,
            vertices,
            point,
            source: None,
            reduction,
        })
    }

    pub fn resize(&mut self, device: &RenderDevice, extents: FogExtents) {
        if self.extents.full == extents.full && self.extents.depth == extents.depth {
            self.extents = extents;
            return;
        }
        if self.extents.full != extents.full {
            (self.full, self.full_attachment, self.full_sample) =
                Self::target(device, "depthAlpha", extents.full, self.format);
            self.reduction = Self::binding(device, &self.downscale, &self.full_sample, &self.point);
        }
        if self.extents.depth != extents.depth {
            (self.reduced, self.reduced_attachment, self.reduced_sample) =
                Self::target(device, "depthAlphaDiv2", extents.depth, self.format);
        }
        self.extents = extents;
    }

    /// The ordinary full-view branch. Sub-view scissor/projection ownership
    /// must be supplied before extending this path to view flag bit zero.
    /// Half-resolution stencil was initialized by the frame clear and is
    /// preserved here; full stencil is cleared unless view+3a34 has bit 3.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        water_depth: &Texture,
        view_flags: u8,
    ) -> Result<(), String> {
        if view_flags & 1 != 0 {
            return Err("alpha depth sub-view scissor owner unresolved".into());
        }
        if water_depth.id() == self.full.id()
            || water_depth.id() == self.reduced.id()
            || water_depth.width() != self.extents.full.x
            || water_depth.height() != self.extents.full.y
            || water_depth.sample_count() != 1
            || water_depth.depth_or_array_layers() != 1
            || !water_depth.format().has_depth_aspect()
        {
            return Err("alpha depth requires completed same-view water depth".into());
        }
        if self
            .source
            .as_ref()
            .is_none_or(|(id, _)| *id != water_depth.id())
        {
            let view = water_depth.create_view(&TextureViewDescriptor {
                aspect: TextureAspect::DepthOnly,
                ..default()
            });
            self.source = Some((
                water_depth.id(),
                Self::binding(device, &self.copy, &view, &self.point),
            ));
        }
        for (pipeline, binding, target, clear_stencil, viewport, allocated) in [
            (
                &self.copy,
                &self.source.as_ref().unwrap().1,
                &self.full_attachment,
                view_flags & 8 == 0,
                self.extents.full,
                self.extents.full,
            ),
            (
                &self.downscale,
                &self.reduction,
                &self.reduced_attachment,
                false,
                (self.extents.full / 2).max(bevy::math::UVec2::ONE),
                self.extents.depth,
            ),
        ] {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("prepare alpha depth"),
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: target,
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
            pass.set_viewport(0., 0., viewport.x as f32, viewport.y as f32, 0., 1.);
            let scissor = viewport.min(allocated);
            pass.set_scissor_rect(0, 0, scissor.x, scissor.y);
            pass.set_bind_group(0, binding, &[]);
            pass.set_vertex_buffer(0, *self.vertices.slice(..));
            pass.draw(0..3, 0..1);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "alpha_depth_tests.rs"]
pub(super) mod tests;
