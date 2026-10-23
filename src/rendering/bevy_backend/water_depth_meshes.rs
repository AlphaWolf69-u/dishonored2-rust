//! Shared raw game-space mesh submission for water and boat depth/stencil.
//! Callers retain material admission, phase order and target initialization.
use super::{
    water_depth_program::{
        WaterDepthDraw, WaterDepthParameters, WaterDepthProgram, WorldWaterDepthParameters,
    },
    water_scene::WaterWorldMeshes,
};
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::GpuShaderBuffer,
    },
};
use std::{collections::HashMap, ops::Range};
#[derive(Clone)]
pub struct DepthSurface {
    pub mesh: AssetId<Mesh>,
    pub matrix: Option<usize>,
    pub world_from_local: Mat4,
    pub order: usize,
    pub state: crate::render_state::RenderState,
    pub reversed: bool,
    pub motion: bool,
    pub reference: u8,
    pub host_space: bool,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    layout: MeshVertexBufferLayoutRef,
    format: TextureFormat,
    state: crate::render_state::RenderState,
    world: bool,
    reversed: bool,
    motion: bool,
    host_space: bool,
}
struct Program {
    source: WaterDepthProgram,
    pipeline: RenderPipeline,
    parameters: Option<(BufferId, BindGroup)>,
    world: Option<([BufferId; 2], BindGroup)>,
}
struct Draw {
    program: usize,
    vertex: Buffer,
    vertices: Range<u32>,
    indices: Option<(Buffer, IndexFormat, Range<u32>, i32)>,
    offset: u32,
    world: bool,
    reference: u8,
}

#[derive(Default)]
pub struct WaterDepthMeshes {
    programs: Vec<Program>,
    keys: HashMap<Key, usize>,
    parameters: Option<Buffer>,
    remap: Option<Buffer>,
    draws: Vec<Draw>,
    model_history: HashMap<(AssetId<Mesh>, usize), (u32, Mat4, Mat4)>,
}
impl WaterDepthMeshes {
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        instances: impl IntoIterator<Item = DepthSurface>,
        world: &WaterWorldMeshes,
        meshes: &RenderAssets<RenderMesh>,
        allocator: &MeshAllocator,
        buffers: &RenderAssets<GpuShaderBuffer>,
        frame: crate::rendering::view_history::ViewMatrixFrame,
        format: TextureFormat,
    ) -> Result<(), String> {
        self.draws.clear();
        let stride = 144u64.next_multiple_of(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        ));
        let mut parameters = Vec::new();
        let mut remap = Vec::new();
        let mut ordered: Vec<_> = instances.into_iter().collect();
        ordered.sort_by_key(|i| i.order);
        for instance in ordered {
            let mesh_id = instance.mesh;
            let matrix = instance.matrix;
            let world_draw = matrix.is_some();
            let mesh = meshes
                .get(mesh_id)
                .ok_or("depth mesh awaits GPU preparation")?;
            let vertex = allocator
                .mesh_vertex_slice(&mesh_id)
                .ok_or("depth vertex allocation unavailable")?;
            let state = instance.state;
            let reversed = instance.reversed;
            let key = Key {
                layout: mesh.layout.clone(),
                format,
                state,
                world: world_draw,
                reversed,
                motion: instance.motion,
                host_space: instance.host_space,
            };
            let program = if let Some(&program) = self.keys.get(&key) {
                program
            } else {
                let layout = mesh
                    .layout
                    .0
                    .get_layout(&[if instance.host_space {
                        Mesh::ATTRIBUTE_POSITION
                    } else {
                        super::shadow_pass::VEGETATION_POSITION
                    }
                    .at_shader_location(0)])
                    .map_err(|e| e.to_string())?;
                let source = if world_draw {
                    WaterDepthProgram::new_world(device, format, state, !instance.motion, reversed)?
                } else {
                    WaterDepthProgram::new(device, format, state, !instance.motion, reversed)?
                };
                let pipeline = source.pipeline(device, &layout);
                let index = self.programs.len();
                self.programs.push(Program {
                    source,
                    pipeline,
                    parameters: None,
                    world: None,
                });
                self.keys.insert(key, index);
                index
            };
            let words = if let Some(matrix) = matrix {
                let offset =
                    u32::try_from(remap.len() / 4).map_err(|_| "water instance remap overflow")?;
                let matrix = u32::try_from(matrix).map_err(|_| "water matrix index overflow")?;
                if matrix > 0x00ff_ffff {
                    return Err("water matrix index exceeds source remap".into());
                }
                remap.extend_from_slice(&matrix.to_le_bytes());
                WorldWaterDepthParameters {
                    clip_from_world: frame.current.view_projection,
                    previous_clip_from_world: frame.previous.view_projection,
                    instance_buffer_offset: offset,
                    first_instance: 0,
                    jitter_relative_to_previous: [0.; 2],
                }
                .uniform_bytes()
            } else {
                // Mesh positions retain game coordinates; instance transforms use the host basis.
                let game_from_view = Mat4::from_cols(Vec4::X, Vec4::Z, -Vec4::Y, Vec4::W);
                let model = if instance.host_space {
                    game_from_view * instance.world_from_local
                } else {
                    game_from_view * instance.world_from_local * game_from_view.transpose()
                };
                let history = self
                    .model_history
                    .entry((mesh_id, instance.order))
                    .or_insert((frame.frame, model, model));
                if history.0 != frame.frame {
                    history.2 = if history.0.wrapping_add(1) == frame.frame {
                        history.1
                    } else {
                        model
                    };
                    history.0 = frame.frame;
                }
                history.1 = model;
                let model_rows = model.transpose().to_cols_array_2d();
                WaterDepthParameters {
                    clip_from_local: crate::render_shadow::multiply_rows(
                        frame.current.view_projection,
                        model_rows,
                    ),
                    previous_clip_from_local: crate::render_shadow::multiply_rows(
                        frame.previous.view_projection,
                        history.2.transpose().to_cols_array_2d(),
                    ),
                    model_depth_hack: 1.,
                    jitter_relative_to_previous: [0.; 2],
                }
                .uniform_bytes()
            };
            let offset = u32::try_from(parameters.len())
                .map_err(|_| "water depth uniform offset overflow")?;
            parameters.extend_from_slice(&words);
            parameters.resize(parameters.len() + stride as usize - words.len(), 0);
            let indices = if let Some(format) = mesh.index_format() {
                let index = allocator
                    .mesh_index_slice(&mesh_id)
                    .ok_or("water depth index allocation unavailable")?;
                Some((
                    index.buffer.clone(),
                    format,
                    index.range.clone(),
                    i32::try_from(vertex.range.start).map_err(|_| "water base vertex overflow")?,
                ))
            } else {
                None
            };
            self.draws.push(Draw {
                program,
                vertex: vertex.buffer.clone(),
                vertices: vertex.range.clone(),
                indices,
                offset,
                world: world_draw,
                reference: instance.reference,
            });
        }
        if self.draws.is_empty() {
            return Ok(());
        }
        let uniform = Self::upload(
            device,
            queue,
            &mut self.parameters,
            &parameters,
            BufferUsages::UNIFORM,
            "water depth parameters",
        );
        let world_buffer = if !remap.is_empty() {
            let remap = Self::upload(
                device,
                queue,
                &mut self.remap,
                &remap,
                BufferUsages::STORAGE,
                "water depth remap",
            );
            let matrices = buffers
                .get(
                    world
                        .matrices
                        .as_ref()
                        .ok_or("water world matrices unavailable")?,
                )
                .ok_or("water world matrices await GPU preparation")?;
            Some((remap, &matrices.buffer))
        } else {
            None
        };
        for program in &mut self.programs {
            if program
                .parameters
                .as_ref()
                .is_none_or(|(id, _)| *id != uniform.id())
            {
                program.parameters = Some((
                    uniform.id(),
                    program.source.parameter_binding(device, uniform),
                ));
            }
            if program.source.world_inputs.is_some() {
                if let Some((remap, matrices)) = world_buffer {
                    let ids = [remap.id(), matrices.id()];
                    if program.world.as_ref().is_none_or(|(old, _)| *old != ids) {
                        program.world =
                            Some((ids, program.source.world_binding(device, remap, matrices)?));
                    }
                }
            }
        }
        Ok(())
    }

    fn upload<'a>(
        device: &RenderDevice,
        queue: &RenderQueue,
        target: &'a mut Option<Buffer>,
        bytes: &[u8],
        usage: BufferUsages,
        label: &'static str,
    ) -> &'a Buffer {
        if target
            .as_ref()
            .is_none_or(|buffer| buffer.size() < bytes.len() as u64)
        {
            *target = Some(device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: usage | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let buffer = target.as_ref().unwrap();
        queue.write_buffer(buffer, 0, bytes);
        buffer
    }

    pub fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }
    pub fn record(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        motion: Option<&TextureView>,
        extent: [u32; 2],
    ) -> Result<(), String> {
        for draw in &self.draws {
            let program = &self.programs[draw.program];
            let draws = [WaterDepthDraw {
                pipeline: &program.pipeline,
                parameters: &program.parameters.as_ref().unwrap().1,
                parameter_offset: draw.offset,
                positions: &draw.vertex,
                vertices: draw.vertices.clone(),
                instances: 0..1,
                indices: draw
                    .indices
                    .as_ref()
                    .map(|(b, f, r, v)| (b, *f, r.clone(), *v)),
            }];
            program.source.record(
                encoder,
                target,
                motion,
                [0., 0., extent[0] as f32, extent[1] as f32],
                [0, 0, extent[0], extent[1]],
                draw.reference,
                if draw.world {
                    program.world.as_ref().map(|(_, b)| b)
                } else {
                    None
                },
                &draws,
            )?;
        }
        Ok(())
    }
}
