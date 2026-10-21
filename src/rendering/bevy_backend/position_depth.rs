//! Rigid position-only depth/stencil passes over shared scene mesh allocations.
//! The caller owns source list admission, render state, stencil reference and
//! target initialization. Procedural ocean/skinning are separate producers.
use super::position_meshes::{PositionInstance, PositionMeshes};
use bevy::{
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

pub struct PositionDepthProgram {
    pub models: BindGroupLayout,
    pub view: BindGroupLayout,
    module: ShaderModule,
    layout: PipelineLayout,
    depth: DepthStencilState,
    primitive: PrimitiveState,
}

impl PositionDepthProgram {
    pub fn new(
        device: &RenderDevice,
        format: TextureFormat,
        state: crate::render_state::RenderState,
    ) -> Result<Self, String> {
        let raster = crate::render_state::raster_state(&state);
        if raster.fill_mode != 3
            || !raster.depth_clip
            || raster.depth_bias_clamp_bits & 0x7fffffff != 0
        {
            return Err(
                "position depth requires solid, clipped rasterization without bias clamp".into(),
            );
        }
        let view = device.create_bind_group_layout(
            Some("position depth view"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(Mat4::min_size()),
                },
                count: None,
            }],
        );
        let models = device.create_bind_group_layout(
            Some("position depth instances"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: Some(Mat4::min_size()),
                },
                count: None,
            }],
        );
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("position depth layout"),
            bind_group_layouts: &[Some(&view), Some(&models)],
            immediate_size: 0,
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("rigid position depth"),
            source: ShaderSource::Wgsl(
                include_str!("../../../assets/shaders/position_depth.wgsl").into(),
            ),
        });
        // render_indices reverses native CW indices; mirrored_render_mesh
        // additionally compensates reflected instances before this transport.
        let (front_face, cull_mode) = super::shadow_pipeline::shadow_mesh_raster(&state, true);
        Ok(Self {
            view,
            models,
            module,
            layout,
            depth: super::shadow_pipeline::depth_stencil_gpu(&state, format)?,
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..default()
            },
        })
    }

    pub fn pipeline(
        &self,
        device: &RenderDevice,
        vertices: &bevy::mesh::VertexBufferLayout,
    ) -> RenderPipeline {
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("position depth/stencil"),
            layout: Some(&self.layout),
            vertex: RawVertexState {
                module: &self.module,
                entry_point: Some("position_depth"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: vertices.array_stride,
                    step_mode: vertices.step_mode,
                    attributes: &vertices.attributes,
                }],
            },
            fragment: None,
            primitive: self.primitive,
            depth_stencil: Some(self.depth.clone()),
            multisample: default(),
            multiview_mask: None,
            cache: None,
        })
    }
}

#[cfg(test)]
#[path = "position_depth_tests.rs"]
mod tests;

pub struct PositionDepthPass {
    pub program: PositionDepthProgram,
    meshes: PositionMeshes,
    view: UniformBuffer<Mat4>,
    binding: Option<(BufferId, BindGroup)>,
    reference: u32,
    prepared: bool,
}

impl PositionDepthPass {
    pub fn new(
        device: &RenderDevice,
        format: TextureFormat,
        state: crate::render_state::RenderState,
        reference: u8,
    ) -> Result<Self, String> {
        Ok(Self {
            program: PositionDepthProgram::new(device, format, state)?,
            meshes: default(),
            view: default(),
            binding: None,
            reference: u32::from(reference),
            prepared: false,
        })
    }
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        clip_from_world: Mat4,
        instances: impl IntoIterator<Item = PositionInstance>,
        allocator: &MeshAllocator,
        meshes: &RenderAssets<RenderMesh>,
    ) -> Result<(), String> {
        self.prepared = false;
        self.view.set(clip_from_world);
        self.view.write_buffer(device, queue);
        let buffer = self
            .view
            .buffer()
            .ok_or("position depth view unavailable")?;
        if self
            .binding
            .as_ref()
            .is_none_or(|(id, _)| *id != buffer.id())
        {
            self.binding = Some((
                buffer.id(),
                device.create_bind_group(
                    Some("position depth view"),
                    &self.program.view,
                    &[BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                ),
            ));
        }
        self.meshes.prepare(
            device,
            queue,
            instances,
            allocator,
            meshes,
            &self.program.models,
            |layout| self.program.pipeline(device, layout),
        )?;
        self.prepared = true;
        Ok(())
    }

    pub fn record(&self, encoder: &mut CommandEncoder, target: &TextureView) -> Result<(), String> {
        if !self.prepared {
            return Err("position depth meshes are not prepared".into());
        }
        let mut pass =
            encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("position depth/stencil"),
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: target,
                    depth_ops: Some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
                    stencil_ops: self.program.depth.format.has_stencil_aspect().then_some(
                        Operations {
                            load: LoadOp::Load,
                            store: StoreOp::Store,
                        },
                    ),
                }),
                ..default()
            });
        pass.set_stencil_reference(self.reference);
        pass.set_bind_group(0, &self.binding.as_ref().unwrap().1, &[]);
        for draw in self.meshes.draws() {
            pass.set_pipeline(draw.pipeline);
            pass.set_bind_group(1, draw.models, &[]);
            pass.set_vertex_buffer(0, *draw.positions.slice(..));
            if let Some((indices, format, range, base)) = draw.indices {
                pass.set_index_buffer(*indices.slice(..), format);
                pass.draw_indexed(range, base, draw.instances);
            } else {
                pass.draw(draw.vertices, draw.instances);
            }
        }
        Ok(())
    }
}
