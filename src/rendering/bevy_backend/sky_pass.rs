//! Sky colour/fog-mask submission from current scene meshes and image assets.
use super::{
    fog::FogFrame,
    sky_mask::{GpuSkyMaterial, SkyInstance, SkyMaskPipeline, SkyProgram, SkySurface, SkyView},
};
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    platform::collections::HashMap,
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
use std::ops::Range;

struct Draw {
    pipeline: usize,
    material: BindGroup,
    vertex: Buffer,
    indices: Option<(Buffer, IndexFormat, Range<u32>, i32)>,
    vertices: Range<u32>,
    surface: u32,
}
#[derive(Hash, PartialEq, Eq)]
struct PipelineKey {
    layout: MeshVertexBufferLayoutRef,
    state: crate::render_state::RenderState,
}

pub struct SkyPass {
    ready: bool,
    draws: Vec<Draw>,
    pipelines: Vec<SkyMaskPipeline>,
    keys: HashMap<PipelineKey, usize>,
    view: UniformBuffer<SkyView>,
    surfaces: DynamicUniformBuffer<SkySurface>,
    binding: Option<((BufferId, BufferId), BindGroup)>,
    hdr_format: TextureFormat,
    depth_format: TextureFormat,
}
impl SkyPass {
    pub fn new(hdr_format: TextureFormat, depth_format: TextureFormat) -> Self {
        Self {
            ready: false,
            draws: Vec::new(),
            pipelines: Vec::new(),
            keys: HashMap::default(),
            view: default(),
            surfaces: default(),
            binding: None,
            hdr_format,
            depth_format,
        }
    }
    pub fn prepare<'a>(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        program: &SkyProgram,
        instances: impl IntoIterator<Item = &'a SkyInstance>,
        allocator: &MeshAllocator,
        meshes: &RenderAssets<RenderMesh>,
        materials: &RenderAssets<GpuSkyMaterial>,
        frame: &FogFrame,
        clip_from_world: Mat4,
        time: f32,
        exposure: f32,
    ) -> Result<(), String> {
        self.ready = false;
        self.draws.clear();
        self.surfaces.clear();
        let mut instances = instances.into_iter().collect::<Vec<_>>();
        // Preserve the scene's source order across ECS insertion/removal.
        instances.sort_by_key(|instance| instance.order);
        for instance in instances {
            let material = materials
                .get(&instance.material)
                .ok_or("sky images await GPU preparation")?;
            let mesh = meshes
                .get(instance.mesh)
                .ok_or("sky mesh awaits GPU preparation")?;
            let vertex = allocator
                .mesh_vertex_slice(&instance.mesh)
                .ok_or("sky vertex allocation unavailable")?;
            let key = PipelineKey {
                layout: mesh.layout.clone(),
                state: material.source.state,
            };
            let pipeline = if let Some(index) = self.keys.get(&key) {
                *index
            } else {
                let layout = mesh
                    .layout
                    .0
                    .get_layout(&[
                        Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
                        Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
                        Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
                        Mesh::ATTRIBUTE_COLOR.at_shader_location(3),
                    ])
                    .map_err(|error| error.to_string())?;
                let index = self.pipelines.len();
                self.pipelines.push(SkyMaskPipeline::with_program(
                    device,
                    self.hdr_format,
                    self.depth_format,
                    &layout,
                    key.state,
                    true,
                    program,
                )?);
                self.keys.insert(key, index);
                index
            };
            let indices = if let Some(format) = mesh.index_format() {
                let index = allocator
                    .mesh_index_slice(&instance.mesh)
                    .ok_or("sky index allocation unavailable")?;
                Some((
                    index.buffer.clone(),
                    format,
                    index.range.clone(),
                    i32::try_from(vertex.range.start)
                        .map_err(|_| "sky base vertex exceeds GPU index range")?,
                ))
            } else {
                None
            };
            let mut sky = material.source.sky;
            let direction = frame.parameters.scattering_direction_phase;
            sky[5] = Vec4::new(direction.x, direction.z, -direction.y, direction.w);
            sky[6] = frame
                .parameters
                .scattering_color_intensity
                .truncate()
                .extend(0.);
            let surface = self.surfaces.push(&SkySurface {
                sky,
                world_from_local: instance.world_from_local,
                fog: Vec4::new(
                    material.source.depth_offset,
                    frame.sky_masking_factor,
                    0.,
                    0.,
                ),
                depth_uv: material.source.depth_uv,
                flags: UVec4::new(
                    u32::from(material.source.clouds),
                    u32::from(material.source.project_far),
                    0,
                    0,
                ),
            });
            self.draws.push(Draw {
                pipeline,
                material: material.binding.clone(),
                vertex: vertex.buffer.clone(),
                indices,
                vertices: vertex.range.clone(),
                surface,
            });
        }
        self.view.set(SkyView {
            clip_from_world,
            time,
            exposure,
            padding: Vec2::ZERO,
        });
        self.view.write_buffer(device, queue);
        if !self.draws.is_empty() {
            self.surfaces.write_buffer(device, queue);
            let view = self.view.buffer().ok_or("sky view buffer unavailable")?;
            let surface = self
                .surfaces
                .buffer()
                .ok_or("sky surface buffer unavailable")?;
            let ids = (view.id(), surface.id());
            if self.binding.as_ref().is_none_or(|(old, _)| *old != ids) {
                self.binding = Some((
                    ids,
                    device.create_bind_group(
                        Some("sky view and instance surfaces"),
                        &program.view_layout,
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: view.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: self
                                    .surfaces
                                    .binding()
                                    .ok_or("sky surface binding unavailable")?,
                            },
                        ],
                    ),
                ));
            }
        }
        self.ready = true;
        Ok(())
    }
    /// An actually empty sky list still produces a cleared mask, so fog never
    /// consumes old sky coverage from a preceding camera frame.
    pub fn record(
        &self,
        encoder: &mut CommandEncoder,
        hdr: &TextureView,
        depth: &TextureView,
        mask: &TextureView,
    ) {
        assert!(self.ready, "sky submission requires completed preparation");
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("sky colour and fog mask"),
            color_attachments: &[
                Some(RenderPassColorAttachment {
                    view: hdr,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                }),
                Some(RenderPassColorAttachment {
                    view: mask,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(LinearRgba::NONE.into()),
                        store: StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                }),
                stencil_ops: self
                    .depth_format
                    .has_stencil_aspect()
                    .then_some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
            }),
            ..default()
        });
        for draw in &self.draws {
            pass.set_pipeline(&self.pipelines[draw.pipeline].pipeline);
            pass.set_bind_group(
                0,
                &self.binding.as_ref().expect("prepared sky view binding").1,
                &[draw.surface],
            );
            pass.set_bind_group(1, &draw.material, &[]);
            pass.set_vertex_buffer(0, *draw.vertex.slice(..));
            if let Some((indices, format, range, base)) = &draw.indices {
                pass.set_index_buffer(*indices.slice(..), *format);
                pass.draw_indexed(range.clone(), *base, 0..1);
            } else {
                pass.draw(draw.vertices.clone(), 0..1);
            }
        }
    }
}
