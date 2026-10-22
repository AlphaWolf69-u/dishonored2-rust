//! Submit the visible placed-water depth programs using their retained meshes.
use super::{
    alpha_depth::AlphaDepthPass,
    fog_targets::FogExtents,
    tiled_lighting_gpu::TlfPipelines,
    water_depth::WaterDepthPass,
    water_depth_program::{
        WaterDepthDraw, WaterDepthParameters, WaterDepthProgram, WorldWaterDepthParameters,
    },
    water_scene::{WaterGeometry, WaterInstance, WaterMaterials, WaterWorldMeshes},
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

#[cfg(test)]
#[path = "water_depth_scene/tests.rs"]
mod tests;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    layout: MeshVertexBufferLayoutRef,
    state: crate::render_state::RenderState,
    world: bool,
    reversed: bool,
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
}

pub struct WaterDepthScene {
    pub water: WaterDepthPass,
    pub alpha: AlphaDepthPass,
    motion: TextureView,
    _motion: Texture,
    programs: Vec<Program>,
    keys: HashMap<Key, usize>,
    parameters: Option<Buffer>,
    remap: Option<Buffer>,
    draws: Vec<Draw>,
    model_history: HashMap<(AssetId<Mesh>, usize), (u32, Mat4, Mat4)>,
}
impl WaterDepthScene {
    pub fn new(
        device: &RenderDevice,
        pipelines: &TlfPipelines,
        extents: FogExtents,
        format: TextureFormat,
    ) -> Result<Self, String> {
        let motion = device.create_texture(&TextureDescriptor {
            label: Some("placed water motion"),
            size: Extent3d {
                width: extents.full.x,
                height: extents.full.y,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rg16Float,
            usage: TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Ok(Self {
            water: WaterDepthPass::new(device, pipelines, extents.full.to_array(), format)?,
            alpha: AlphaDepthPass::new(device, pipelines, extents, format)?,
            motion: motion.create_view(&default()),
            _motion: motion,
            programs: Vec::new(),
            keys: HashMap::new(),
            parameters: None,
            remap: None,
            draws: Vec::new(),
            model_history: HashMap::new(),
        })
    }

    pub fn prepare<'a>(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        instances: impl IntoIterator<Item = &'a WaterInstance>,
        materials: &WaterMaterials,
        world: &WaterWorldMeshes,
        meshes: &RenderAssets<RenderMesh>,
        allocator: &MeshAllocator,
        buffers: &RenderAssets<GpuShaderBuffer>,
        frame: crate::rendering::view_history::ViewMatrixFrame,
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
            let material = materials
                .0
                .get(&instance.material)
                .ok_or("water depth material unavailable")?
                .as_ref()
                .map_err(Clone::clone)?;
            let (mesh_id, matrix, surface, reversed) = match instance.geometry {
                Some(WaterGeometry::World { object, matrix }) => {
                    let source = world
                        .objects
                        .get(&object)
                        .ok_or("world water depth mesh unavailable")?
                        .as_ref()
                        .map_err(Clone::clone)?;
                    (
                        source.mesh.id(),
                        Some(matrix),
                        source.surface_flags,
                        source.indices_reversed,
                    )
                }
                Some(WaterGeometry::Model) => (instance.mesh, None, 0, false),
                None => return Err("water depth geometry association unavailable".into()),
            };
            let world_draw = matrix.is_some();
            let Some(source) = material.depth[usize::from(world_draw)]
                .as_ref()
                .map_err(Clone::clone)?
            else {
                continue;
            };
            let expected = if world_draw {
                crate::shader_cache::CachedWaterDepthClass::World
            } else {
                crate::shader_cache::CachedWaterDepthClass::Rigid
            };
            if source.class != expected {
                return Err("water depth program and geometry class differ".into());
            }
            let mesh = meshes
                .get(mesh_id)
                .ok_or("water depth mesh awaits GPU preparation")?;
            let vertex = allocator
                .mesh_vertex_slice(&mesh_id)
                .ok_or("water depth vertex allocation unavailable")?;
            let state = if world_draw {
                crate::render_state::world_depth_state(material.state, surface)
            } else {
                crate::render_state::water_depth_state()
            };
            let key = Key {
                layout: mesh.layout.clone(),
                state,
                world: world_draw,
                reversed,
            };
            let program = if let Some(&program) = self.keys.get(&key) {
                program
            } else {
                let layout = mesh
                    .layout
                    .0
                    .get_layout(&[super::shadow_pass::VEGETATION_POSITION.at_shader_location(0)])
                    .map_err(|e| e.to_string())?;
                let source = if world_draw {
                    WaterDepthProgram::new_world(
                        device,
                        self.water.texture.format(),
                        state,
                        false,
                        reversed,
                    )?
                } else {
                    WaterDepthProgram::new(
                        device,
                        self.water.texture.format(),
                        state,
                        false,
                        reversed,
                    )?
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
                let model = game_from_view * instance.world_from_local * game_from_view.transpose();
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

    pub fn record(
        &mut self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        opaque: &Texture,
    ) -> Result<(), String> {
        self.water.prepare(device, encoder, opaque, false)?;
        {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("water motion clear"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: &self.motion,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Default::default()),
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
        }
        let extent = [opaque.width(), opaque.height()];
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
                &self.water.attachment,
                Some(&self.motion),
                [0., 0., extent[0] as f32, extent[1] as f32],
                [0, 0, extent[0], extent[1]],
                if draw.world { 0 } else { 4 },
                program.world.as_ref().map(|(_, b)| b),
                &draws,
            )?;
        }
        self.water.restore_boat_depth(encoder)?;
        self.alpha.record(device, encoder, &self.water.texture, 0)
    }
}
