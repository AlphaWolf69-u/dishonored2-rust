//! Area fog reuses resident mesh allocations and one interval attachment.
use super::{
    fog::{AreaFogFrame, AreaFogStates, FogView},
    fog_targets::{FogAttachment, FogExtents},
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

#[derive(Clone, ShaderType)]
pub struct AreaFogUniform {
    pub view_from_local: Mat4,
    pub clip_from_view: Mat4,
    pub world_from_view: Mat4,
    pub distance: Vec4,
    pub color_start: Vec4,
    pub color_end: Vec4,
    pub density: Vec4,
    pub scattering_direction_phase: Vec4,
    pub scattering_color_intensity: Vec4,
    pub extent: Vec4,
    pub ray_scale: Vec4,
}
impl AreaFogUniform {
    fn new(frame: &AreaFogFrame, view: FogView, mask: UVec2, destination: UVec2) -> Self {
        let p = &frame.parameters;
        let flags = 1 + u32::from(p.volume_type.y > 0.) * 2 + u32::from(p.volume_type.z > 0.) * 4;
        let view_from_world = view.world_from_view.inverse();
        Self {
            view_from_local: view_from_world * frame.world_from_local,
            clip_from_view: view.clip_from_world * view.world_from_view,
            world_from_view: view.world_from_view,
            distance: p.distance,
            color_start: p.color_start.extend(0.),
            color_end: p.color_end.extend(0.),
            density: Vec4::new(p.density_factor, p.falloff, view.eye.z, flags as f32),
            scattering_direction_phase: p.scattering_direction.extend(p.scattering_phase),
            scattering_color_intensity: p.scattering_color.extend(p.scattering_intensity),
            extent: Vec4::new(
                mask.x as f32,
                mask.y as f32,
                destination.x as f32,
                destination.y as f32,
            ),
            ray_scale: view.ray_scale,
        }
    }
}
struct Draw {
    pipeline: usize,
    vertex: Buffer,
    indices: Option<(Buffer, IndexFormat, Range<u32>, i32)>,
    vertices: Range<u32>,
}
struct Volume {
    uniform: u32,
    draws: Range<usize>,
}
pub struct AreaFogPass {
    pub states: AreaFogStates,
    module: ShaderModule,
    layout: BindGroupLayout,
    interval_layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    radiance: RenderPipeline,
    interval_blend: Option<BlendState>,
    interval: FogAttachment,
    interval_size: UVec2,
    uniforms: DynamicUniformBuffer<AreaFogUniform>,
    sampler: Sampler,
    binding: Option<((BufferId, TextureViewId), BindGroup, BindGroup)>,
    pipelines: Vec<RenderPipeline>,
    keys: HashMap<MeshVertexBufferLayoutRef, usize>,
    draws: Vec<Draw>,
    volumes: Vec<Volume>,
}
impl AreaFogPass {
    pub fn new(
        device: &RenderDevice,
        extents: FogExtents,
        states: AreaFogStates,
    ) -> Result<Self, String> {
        let source = format!(
            "{}\n{}",
            include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_volume.wgsl")
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("area fog interval and radiance"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_bind_group_layout(
            Some("area fog inputs"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: Some(AreaFogUniform::min_size()),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("area fog"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        // The interval draw writes the mask; it must not bind that same mask
        // for sampling, including through an unused bind-group entry.
        let interval_layout = device.create_bind_group_layout(
            Some("area interval inputs"),
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX_FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: Some(AreaFogUniform::min_size()),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 1,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let interval_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("area fog interval"),
            bind_group_layouts: &[Some(&interval_layout)],
            immediate_size: 0,
        });
        let interval_state = crate::render_state::fog_interval_draw_state(states.interval);
        let interval_blend = super::pipeline_state::blend_target(
            crate::render_state::blend_state(&interval_state).targets[0]
                .ok_or("area interval has no colour output")?,
        )?;
        let radiance_state = crate::render_state::fog_draw_state(states.radiance);
        let radiance_blend = super::pipeline_state::blend_target(
            crate::render_state::blend_state(&radiance_state).targets[0]
                .ok_or("area fog has no colour output")?,
        )?;
        let radiance = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("area fog radiance"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("area_vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("area_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    blend: radiance_blend,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: default(),
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        // 307530 uses half resolution here independently of r_downsampleQuality.
        let size = ((extents.full + UVec2::ONE) >> 1u32).max(UVec2::splat(4));
        Ok(Self {
            states,
            module,
            layout,
            interval_layout,
            pipeline_layout: interval_pipeline_layout,
            radiance,
            interval_blend,
            interval: FogAttachment::new(
                device,
                "area fog min max thickness",
                size,
                TextureFormat::Rgba16Float,
            ),
            interval_size: size,
            uniforms: default(),
            sampler: device.create_sampler(&SamplerDescriptor {
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..default()
            }),
            binding: None,
            pipelines: Vec::new(),
            keys: default(),
            draws: Vec::new(),
            volumes: Vec::new(),
        })
    }

    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        extents: FogExtents,
        view: FogView,
        frames: &[AreaFogFrame],
        meshes: &RenderAssets<RenderMesh>,
        allocator: &MeshAllocator,
        depth: &TextureView,
    ) -> Result<(), String> {
        let size = ((extents.full + UVec2::ONE) >> 1u32).max(UVec2::splat(4));
        if size != self.interval_size {
            self.interval = FogAttachment::new(
                device,
                "area fog min max thickness",
                size,
                TextureFormat::Rgba16Float,
            );
            self.interval_size = size;
            self.binding = None;
        }
        self.uniforms.clear();
        self.draws.clear();
        self.volumes.clear();
        for frame in frames {
            let offset = self
                .uniforms
                .push(&AreaFogUniform::new(frame, view, size, extents.world));
            let start = self.draws.len();
            for handle in &frame.meshes {
                let id = handle.id();
                let mesh = meshes
                    .get(id)
                    .ok_or("area fog mesh awaits GPU preparation")?;
                let vertex = allocator
                    .mesh_vertex_slice(&id)
                    .ok_or("area fog vertex allocation unavailable")?;
                let pipeline = if let Some(index) = self.keys.get(&mesh.layout) {
                    *index
                } else {
                    let vertex_layout = mesh
                        .layout
                        .0
                        .get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])
                        .map_err(|e| e.to_string())?;
                    let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
                        label: Some("area fog interval geometry"),
                        layout: Some(&self.pipeline_layout),
                        vertex: RawVertexState {
                            module: &self.module,
                            entry_point: Some("area_interval_vertex"),
                            compilation_options: default(),
                            buffers: &[RawVertexBufferLayout {
                                array_stride: vertex_layout.array_stride,
                                step_mode: vertex_layout.step_mode,
                                attributes: &vertex_layout.attributes,
                            }],
                        },
                        fragment: Some(RawFragmentState {
                            module: &self.module,
                            entry_point: Some("area_interval_fragment"),
                            compilation_options: default(),
                            targets: &[Some(ColorTargetState {
                                format: TextureFormat::Rgba16Float,
                                blend: self.interval_blend,
                                write_mask: ColorWrites::ALL,
                            })],
                        }),
                        primitive: PrimitiveState {
                            front_face: FrontFace::Ccw,
                            cull_mode: None,
                            ..default()
                        },
                        depth_stencil: None,
                        multisample: default(),
                        multiview_mask: None,
                        cache: None,
                    });
                    let index = self.pipelines.len();
                    self.pipelines.push(pipeline);
                    self.keys.insert(mesh.layout.clone(), index);
                    index
                };
                let indices = if let Some(format) = mesh.index_format() {
                    let index = allocator
                        .mesh_index_slice(&id)
                        .ok_or("area fog index allocation unavailable")?;
                    Some((
                        index.buffer.clone(),
                        format,
                        index.range.clone(),
                        i32::try_from(vertex.range.start)
                            .map_err(|_| "area fog base vertex overflow")?,
                    ))
                } else {
                    None
                };
                self.draws.push(Draw {
                    pipeline,
                    vertex: vertex.buffer.clone(),
                    indices,
                    vertices: vertex.range.clone(),
                });
            }
            self.volumes.push(Volume {
                uniform: offset,
                draws: start..self.draws.len(),
            });
        }
        if self.volumes.is_empty() {
            return Ok(());
        }
        self.uniforms.write_buffer(device, queue);
        let buffer = self
            .uniforms
            .buffer()
            .ok_or("area fog uniform buffer unavailable")?;
        let ids = (buffer.id(), depth.id());
        if self.binding.as_ref().is_none_or(|(old, _, _)| *old != ids) {
            let radiance = device.create_bind_group(
                Some("area fog shared resources"),
                &self.layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: self
                            .uniforms
                            .binding()
                            .ok_or("area fog uniforms not bound")?,
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(depth),
                    },
                    BindGroupEntry {
                        binding: 2,
                        resource: BindingResource::TextureView(&self.interval.view),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                ],
            );
            let interval = device.create_bind_group(
                Some("area interval shared resources"),
                &self.interval_layout,
                &[
                    BindGroupEntry {
                        binding: 0,
                        resource: self
                            .uniforms
                            .binding()
                            .ok_or("area interval uniforms not bound")?,
                    },
                    BindGroupEntry {
                        binding: 1,
                        resource: BindingResource::TextureView(depth),
                    },
                    BindGroupEntry {
                        binding: 3,
                        resource: BindingResource::Sampler(&self.sampler),
                    },
                ],
            );
            self.binding = Some((ids, radiance, interval));
        }
        Ok(())
    }

    pub fn record(&self, encoder: &mut CommandEncoder, destination: &TextureView) {
        if self.volumes.is_empty() {
            return;
        }
        let (_, binding, interval_binding) =
            self.binding.as_ref().expect("area fog prepared bindings");
        for volume in &self.volumes {
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("area fog entry exit thickness"),
                    color_attachments: &[Some(RenderPassColorAttachment {
                        view: &self.interval.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: Operations {
                            load: LoadOp::Clear(LinearRgba::new(1.0e6, 0., 0., 0.).into()),
                            store: StoreOp::Store,
                        },
                    })],
                    ..default()
                });
                pass.set_bind_group(0, interval_binding, &[volume.uniform]);
                for draw in &self.draws[volume.draws.clone()] {
                    pass.set_pipeline(&self.pipelines[draw.pipeline]);
                    pass.set_vertex_buffer(0, *draw.vertex.slice(..));
                    if let Some((index, format, range, base)) = &draw.indices {
                        pass.set_index_buffer(*index.slice(..), *format);
                        pass.draw_indexed(range.clone(), *base, 0..1);
                    } else {
                        pass.draw(draw.vertices.clone(), 0..1);
                    }
                }
            }
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("area fog radiance composition"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: destination,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(&self.radiance);
            pass.set_bind_group(0, binding, &[volume.uniform]);
            pass.draw(0..3, 0..1);
        }
    }
}
