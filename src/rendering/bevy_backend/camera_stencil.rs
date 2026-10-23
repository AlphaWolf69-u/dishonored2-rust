//! Installed near/far blur admission, including the autofocus vertex variant.
//! Geometry, viewport, projection and completed depth/stencil are owner inputs.
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};

pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_camera_stencil.wgsl")
);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CameraStencilKind {
    Near,
    Far,
    Autofocus,
    Constant,
}
#[derive(Clone, Debug)]
pub struct CameraStencilParameters {
    pub projection: [[f32; 4]; 4],
    pub near_clip: f32,
    /// Far sharp distance for Far/Autofocus; literal clip-space Z for Constant.
    pub clip_distance: f32,
}
impl CameraStencilParameters {
    pub fn uniform_bytes(&self) -> [u8; 80] {
        let mut bytes = [0; 80];
        for (i, value) in self
            .projection
            .into_iter()
            .flatten()
            .chain([self.near_clip, self.clip_distance, 0., 0.])
            .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}
pub enum CameraStencilSource<'a> {
    Near(&'a TextureView),
    Far(&'a TextureView),
    Autofocus {
        mask: &'a TextureView,
        focus: &'a TextureView,
    },
    Constant,
}
pub struct CameraStencilTarget<'a> {
    pub attachment: &'a TextureView,
    pub viewport: [u32; 4],
    pub scissor: [u32; 4],
    pub stencil_reference: u32,
}
pub struct CameraStencilPass {
    kind: CameraStencilKind,
    pipeline: RenderPipeline,
    uniform: Buffer,
    linear: Sampler,
    binding: Option<([Option<TextureViewId>; 2], BindGroup)>,
}
impl CameraStencilPass {
    pub fn new(
        device: &RenderDevice,
        kind: CameraStencilKind,
        format: TextureFormat,
        state: crate::render_state::RenderState,
    ) -> Result<Self, String> {
        if !format.has_depth_aspect() || !format.has_stencil_aspect() {
            return Err("camera classification requires its depth/stencil attachment".into());
        }
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("camera stencil classification"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let (vertex, fragment) = match kind {
            CameraStencilKind::Near => ("stencil_near_vertex", "stencil_near_fragment"),
            CameraStencilKind::Far => ("stencil_far_vertex", "stencil_far_fragment"),
            CameraStencilKind::Autofocus => ("stencil_autofocus_vertex", "stencil_far_fragment"),
            CameraStencilKind::Constant => ("stencil_constant_vertex", "stencil_constant_fragment"),
        };
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&state, false);
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("camera stencil classification"),
            layout: None,
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: 20,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[
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
                    ],
                }],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..Default::default()
            },
            depth_stencil: Some(super::pipeline_state::depth_stencil_gpu(&state, format)?),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            kind,
            pipeline,
            uniform: device.create_buffer(&BufferDescriptor {
                label: Some("camera stencil parameters"),
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
        })
    }
    /// Each queued classifier needs its own owner/uniform, as near/far may be
    /// recorded in the same encoder before either is submitted.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        parameters: &CameraStencilParameters,
        source: CameraStencilSource<'_>,
        target: CameraStencilTarget<'_>,
        vertices: &Buffer,
        range: std::ops::Range<u32>,
    ) -> Result<(), String> {
        let (kind, mask, focus) = match source {
            CameraStencilSource::Near(mask) => (CameraStencilKind::Near, Some(mask), None),
            CameraStencilSource::Far(mask) => (CameraStencilKind::Far, Some(mask), None),
            CameraStencilSource::Autofocus { mask, focus } => {
                (CameraStencilKind::Autofocus, Some(mask), Some(focus))
            }
            CameraStencilSource::Constant => (CameraStencilKind::Constant, None, None),
        };
        if kind != self.kind {
            return Err("camera stencil source differs from prepared shader".into());
        }
        if mask
            .into_iter()
            .chain(focus)
            .any(|v| v.id() == target.attachment.id())
        {
            return Err("camera stencil source aliases its attachment".into());
        }
        queue.write_buffer(&self.uniform, 0, &parameters.uniform_bytes());
        let key = [mask.map(TextureView::id), focus.map(TextureView::id)];
        if self.binding.as_ref().is_none_or(|b| b.0 != key) {
            let mut entries = vec![BindGroupEntry {
                binding: 0,
                resource: self.uniform.as_entire_binding(),
            }];
            if let Some(mask) = mask {
                entries.push(BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(mask),
                });
                if kind == CameraStencilKind::Near {
                    entries.push(BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Sampler(&self.linear),
                    });
                }
            }
            if let Some(focus) = focus {
                entries.push(BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(focus),
                });
            }
            // Near's vertex/fragment shaders do not reference the uniform.
            if kind == CameraStencilKind::Near {
                entries.remove(0);
            }
            self.binding = Some((
                key,
                device.create_bind_group(
                    Some("camera stencil inputs"),
                    &self.pipeline.get_bind_group_layout(0).into(),
                    &entries,
                ),
            ));
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("camera stencil classification"),
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: target.attachment,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
            }),
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        pass.set_stencil_reference(target.stencil_reference);
        let v = target.viewport;
        let s = target.scissor;
        pass.set_viewport(v[0] as f32, v[1] as f32, v[2] as f32, v[3] as f32, 0., 1.);
        pass.set_scissor_rect(s[0], s[1], s[2], s[3]);
        pass.set_vertex_buffer(0, *vertices.slice(..));
        pass.draw(range, 0..1);
        Ok(())
    }
}

#[cfg(test)]
#[path = "camera_stencil_tests.rs"]
mod tests;
