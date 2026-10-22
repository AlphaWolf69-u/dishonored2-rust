//! 2fdea0's full/reduced underwater marking over completed depthAlpha targets.
//! Admission, wave-height mask and waterMatrix come from the water owner.
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Clone, Copy, Default, ShaderType)]
pub struct UnderwaterStencilParameters {
    pub world_from_clip: Mat4,
    pub water_matrix: Mat4,
    pub viewport: Vec4,
    pub resolution_scale: Vec4,
}

pub struct UnderwaterStencilTarget<'a> {
    pub attachment: &'a TextureView,
    pub parameters: UnderwaterStencilParameters,
}

pub enum UnderwaterStencilSource<'a> {
    Ocean(&'a TextureView),
    Volume,
}

pub struct UnderwaterStencilPass {
    pipelines: [RenderPipeline; 2],
    uniforms: [UniformBuffer<UnderwaterStencilParameters>; 2],
    bindings: [Option<((bool, Option<TextureViewId>), BindGroup)>; 2],
    sampler: Sampler,
}
impl UnderwaterStencilPass {
    pub fn new(
        device: &RenderDevice,
        format: TextureFormat,
        state: crate::render_state::RenderState,
    ) -> Result<Self, String> {
        if !format.has_depth_aspect() || !format.has_stencil_aspect() {
            return Err("underwater marking requires the shared depth/stencil attachment".into());
        }
        let prepared = crate::render_state::underwater_stencil_state(state);
        let depth = super::pipeline_state::depth_stencil_gpu(&prepared, format)?;
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&prepared, false);
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("underwater stencil"),
            source: ShaderSource::Wgsl(
                include_str!("../../../assets/shaders/dh2_underwater_stencil.wgsl").into(),
            ),
        });
        let pipelines = std::array::from_fn(|index| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("underwater stencil"),
                layout: None,
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("vertex"),
                    compilation_options: default(),
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
                    entry_point: Some(if index == 0 {
                        "ocean_fragment"
                    } else {
                        "volume_fragment"
                    }),
                    compilation_options: default(),
                    targets: &[],
                }),
                primitive: PrimitiveState {
                    front_face,
                    cull_mode,
                    ..default()
                },
                depth_stencil: Some(depth.clone()),
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        });
        Ok(Self {
            pipelines,
            uniforms: std::array::from_fn(|_| default()),
            bindings: std::array::from_fn(|_| None),
            sampler: device.create_sampler(&SamplerDescriptor {
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..default()
            }),
        })
    }

    /// Keep two independent view uniforms: both targets are recorded before
    /// submission, so updating one shared buffer would corrupt the first draw.
    /// Existing depth and every stencil bit other than 0x40 are retained.
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        source: UnderwaterStencilSource,
        targets: [UnderwaterStencilTarget; 2],
        vertices: &Buffer,
        range: std::ops::Range<u32>,
    ) -> Result<(), String> {
        for target in &targets {
            let viewport = target.parameters.viewport;
            if !viewport.is_finite()
                || viewport.x < 0.
                || viewport.y < 0.
                || viewport.z <= 0.
                || viewport.w <= 0.
            {
                return Err("underwater stencil viewport is invalid".into());
            }
        }
        let volume = matches!(source, UnderwaterStencilSource::Volume);
        let mask = match source {
            UnderwaterStencilSource::Ocean(view) => Some(view),
            UnderwaterStencilSource::Volume => None,
        };
        let pipeline = &self.pipelines[usize::from(volume)];
        for (index, target) in targets.into_iter().enumerate() {
            self.uniforms[index].set(target.parameters);
            self.uniforms[index].write_buffer(device, queue);
            let key = (volume, mask.map(TextureView::id));
            if self.bindings[index].as_ref().is_none_or(|old| old.0 != key) {
                let mut entries = vec![BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms[index].binding().unwrap(),
                }];
                if let Some(mask) = mask {
                    entries.push(BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(mask),
                    });
                    entries.push(BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::Sampler(&self.sampler),
                    });
                }
                self.bindings[index] = Some((
                    key,
                    device.create_bind_group(
                        Some("underwater stencil inputs"),
                        &pipeline.get_bind_group_layout(0).into(),
                        &entries,
                    ),
                ));
            }
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("underwater full/reduced stencil"),
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
                ..default()
            });
            let viewport = target.parameters.viewport;
            pass.set_viewport(viewport.x, viewport.y, viewport.z, viewport.w, 0., 1.);
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.bindings[index].as_ref().unwrap().1, &[]);
            pass.set_stencil_reference(0x42);
            pass.set_vertex_buffer(0, *vertices.slice(..));
            pass.draw(range.clone(), 0..1);
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "underwater_stencil/tests.rs"]
mod tests;
