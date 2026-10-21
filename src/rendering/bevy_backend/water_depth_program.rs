//! Rigid arkWater depth raster with source MVP history and RG16F motion output.
//! The selected program/state and mesh admission belong to the scene owner;
//! this is not a substitute for wind/skinned or other water-sort programs.
use bevy::{
    prelude::default,
    render::{render_resource::*, renderer::RenderDevice},
};

pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_shadow_world_project.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_motion_vector.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_depth.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_world_depth.wgsl"),
);

#[cfg(test)]
#[path = "water_depth_program/tests.rs"]
mod tests;
#[cfg(test)]
#[path = "water_depth_program/world_tests.rs"]
mod world_tests;

#[derive(Clone, Copy, Debug)]
pub struct WaterDepthParameters {
    /// Logical rows, before modelDepthHack, from the actual instance owner.
    pub clip_from_local: [[f32; 4]; 4],
    pub previous_clip_from_local: [[f32; 4]; 4],
    pub model_depth_hack: f32,
    pub jitter_relative_to_previous: [f32; 2],
}
impl WaterDepthParameters {
    pub fn uniform_bytes(&self) -> [u8; 144] {
        depth_parameter_bytes(
            self.clip_from_local,
            self.previous_clip_from_local,
            [
                self.model_depth_hack.to_bits(),
                self.jitter_relative_to_previous[0].to_bits(),
                self.jitter_relative_to_previous[1].to_bits(),
                0,
            ],
        )
    }
}

/// VS14487 takes view-projection history and a remap offset, with no model hack.
#[derive(Clone, Copy, Debug)]
pub struct WorldWaterDepthParameters {
    pub clip_from_world: [[f32; 4]; 4],
    pub previous_clip_from_world: [[f32; 4]; 4],
    pub instance_buffer_offset: u32,
    /// WGPU includes this in instance_index; D3D11 SV_InstanceID starts at zero.
    pub first_instance: u32,
    pub jitter_relative_to_previous: [f32; 2],
}
impl WorldWaterDepthParameters {
    pub fn uniform_bytes(&self) -> [u8; 144] {
        depth_parameter_bytes(
            self.clip_from_world,
            self.previous_clip_from_world,
            [
                self.instance_buffer_offset,
                self.jitter_relative_to_previous[0].to_bits(),
                self.jitter_relative_to_previous[1].to_bits(),
                self.first_instance,
            ],
        )
    }
}

fn depth_parameter_bytes(
    current: [[f32; 4]; 4],
    previous: [[f32; 4]; 4],
    controls: [u32; 4],
) -> [u8; 144] {
    let mut bytes = [0; 144];
    for (i, word) in current
        .into_iter()
        .flatten()
        .chain(previous.into_iter().flatten())
        .map(f32::to_bits)
        .chain(controls)
        .enumerate()
    {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    bytes
}

pub struct WaterDepthProgram {
    pub parameters: BindGroupLayout,
    pub world_inputs: Option<BindGroupLayout>,
    module: ShaderModule,
    layout: PipelineLayout,
    depth: DepthStencilState,
    primitive: PrimitiveState,
    motion: ColorTargetState,
    alpha_depth: bool,
}

/// A rigid surface's current shared mesh allocation and prepared parameters.
/// The pipeline is cached per decoded vertex layout by the submission owner.
pub struct WaterDepthDraw<'a> {
    pub pipeline: &'a RenderPipeline,
    pub parameters: &'a BindGroup,
    pub parameter_offset: u32,
    pub positions: &'a Buffer,
    pub vertices: std::ops::Range<u32>,
    pub instances: std::ops::Range<u32>,
    pub indices: Option<(&'a Buffer, IndexFormat, std::ops::Range<u32>, i32)>,
}

impl WaterDepthProgram {
    pub fn new(
        device: &RenderDevice,
        format: TextureFormat,
        state: crate::render_state::RenderState,
        alpha_depth: bool,
        indices_reversed: bool,
    ) -> Result<Self, String> {
        Self::with_world_inputs(device, format, state, alpha_depth, indices_reversed, false)
    }

    pub fn new_world(
        device: &RenderDevice,
        format: TextureFormat,
        state: crate::render_state::RenderState,
        alpha_depth: bool,
        indices_reversed: bool,
    ) -> Result<Self, String> {
        Self::with_world_inputs(device, format, state, alpha_depth, indices_reversed, true)
    }

    fn with_world_inputs(
        device: &RenderDevice,
        format: TextureFormat,
        state: crate::render_state::RenderState,
        alpha_depth: bool,
        indices_reversed: bool,
        world: bool,
    ) -> Result<Self, String> {
        if !format.has_stencil_aspect() {
            return Err("water depth requires its depth/stencil owner".into());
        }
        let raster = crate::render_state::raster_state(&state);
        if raster.fill_mode != 3
            || !raster.depth_clip
            || raster.antialiased_line
            || raster.multisample
        {
            return Err(
                "rigid water depth requires solid clipped single-sample rasterization".into(),
            );
        }
        let parameters = device.create_bind_group_layout(
            Some("water depth parameters"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: BufferSize::new(144),
                },
                count: None,
            }],
        );
        let world_inputs = world.then(|| {
            device.create_bind_group_layout(
                Some("water world instances"),
                &[4, 48]
                    .into_iter()
                    .enumerate()
                    .map(|(binding, size)| BindGroupLayoutEntry {
                        binding: binding as u32,
                        visibility: ShaderStages::VERTEX,
                        ty: BindingType::Buffer {
                            ty: BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: BufferSize::new(size),
                        },
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            )
        });
        let mut layouts = vec![Some(&*parameters)];
        if let Some(inputs) = &world_inputs {
            layouts.push(Some(&**inputs));
        }
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("rigid water depth"),
            bind_group_layouts: &layouts,
            immediate_size: 0,
        });
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("arkWater rigid depth"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&state, indices_reversed);
        let target = crate::render_state::blend_state(&state).targets[0]
            .ok_or("water depth has no motion target state")?;
        Ok(Self {
            parameters,
            world_inputs,
            module,
            layout,
            depth: super::pipeline_state::depth_stencil_gpu(&state, format)?,
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..default()
            },
            motion: ColorTargetState {
                format: TextureFormat::Rg16Float,
                blend: super::pipeline_state::blend_target(target)?,
                write_mask: ColorWrites::from_bits_truncate(u32::from(target.write_mask)),
            },
            alpha_depth,
        })
    }

    pub fn pipeline(
        &self,
        device: &RenderDevice,
        vertices: &bevy::mesh::VertexBufferLayout,
    ) -> RenderPipeline {
        let motion_targets = [Some(self.motion.clone())];
        device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("rigid water depth"),
            layout: Some(&self.layout),
            vertex: RawVertexState {
                module: &self.module,
                entry_point: Some(if self.world_inputs.is_some() {
                    "water_world_depth_vertex"
                } else {
                    "water_depth_vertex"
                }),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: vertices.array_stride,
                    step_mode: vertices.step_mode,
                    attributes: &vertices.attributes,
                }],
            },
            fragment: if self.alpha_depth {
                None
            } else {
                Some(RawFragmentState {
                    module: &self.module,
                    entry_point: Some("water_depth_fragment"),
                    compilation_options: default(),
                    targets: &motion_targets,
                })
            },
            primitive: self.primitive,
            depth_stencil: Some(self.depth.clone()),
            multisample: default(),
            multiview_mask: None,
            cache: None,
        })
    }

    pub fn parameter_binding(&self, device: &RenderDevice, buffer: &Buffer) -> BindGroup {
        device.create_bind_group(
            Some("water depth instance"),
            &self.parameters,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer,
                    offset: 0,
                    size: BufferSize::new(144),
                }),
            }],
        )
    }

    pub fn world_binding(
        &self,
        device: &RenderDevice,
        indices: &Buffer,
        matrices: &Buffer,
    ) -> Result<BindGroup, String> {
        let layout = self
            .world_inputs
            .as_ref()
            .ok_or("rigid water has no world instance buffers")?;
        Ok(device.create_bind_group(
            Some("water world instances"),
            layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: indices.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: matrices.as_entire_binding(),
                },
            ],
        ))
    }

    /// Preserve initialized attachments and source draw order. World depth
    /// supplies reference zero, model water four; alpha omits motion output.
    pub fn record(
        &self,
        encoder: &mut CommandEncoder,
        depth: &TextureView,
        motion: Option<&TextureView>,
        viewport: [f32; 4],
        scissor: [u32; 4],
        stencil_reference: u8,
        world_inputs: Option<&BindGroup>,
        draws: &[WaterDepthDraw<'_>],
    ) -> Result<(), String> {
        if self.alpha_depth != motion.is_none() {
            return Err("water depth mode and motion attachment differ".into());
        }
        if self.world_inputs.is_some() != world_inputs.is_some() {
            return Err("water depth program and world instance buffers differ".into());
        }
        let colours = [motion.map(|view| RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("rigid water depth"),
            color_attachments: if self.alpha_depth { &[] } else { &colours },
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth,
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
        pass.set_viewport(viewport[0], viewport[1], viewport[2], viewport[3], 0., 1.);
        pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
        pass.set_stencil_reference(u32::from(stencil_reference));
        if let Some(inputs) = world_inputs {
            pass.set_bind_group(1, inputs, &[]);
        }
        for draw in draws {
            pass.set_pipeline(draw.pipeline);
            pass.set_bind_group(0, draw.parameters, &[draw.parameter_offset]);
            pass.set_vertex_buffer(0, *draw.positions.slice(..));
            if let Some((indices, format, range, base)) = &draw.indices {
                pass.set_index_buffer(*indices.slice(..), *format);
                pass.draw_indexed(range.clone(), *base, draw.instances.clone());
            } else {
                pass.draw(draw.vertices.clone(), draw.instances.clone());
            }
        }
        Ok(())
    }
}
