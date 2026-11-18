//! ARK_RENDERPASS_DARKVISION (14031f960) device (mask bit 1) and pickup (mask
//! bit 4) layers. Each layer's surfaces get a depth prepass into a cleared
//! depth target (1402bb060) with their own depth program state, then
//! `rendering/darkvision` draws them into hdrBuffer (`rendering/ColorIn` the
//! copied scene, `rendering/DepthBufferIn` decompressedDepth) with that
//! program's state: additive, depth EQUAL against the prepass. Layer depth
//! clips are record +0x4260 (device) and +0x4268 (pickup).
//! bevy_backend: one dynamic-offset uniform per draw replaces the original's
//! per-instance constant buffer; vertex buffers come from the mesh allocator.
use crate::gameplay::powers::dark_vision::DarkVisionRecord;
use crate::render_state::RenderState;
use bevy::{
    asset::AssetId,
    math::Mat4,
    mesh::{Mesh, MeshVertexBufferLayoutRef},
    platform::collections::HashMap,
    prelude::Resource,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
use std::sync::Arc;

/// One visible forward surface whose material selects a Dark Vision layer.
#[derive(Clone)]
pub struct DarkVisionSurface {
    pub mesh: AssetId<Mesh>,
    /// Bevy world transform.
    pub model: Mat4,
    /// 1 device, 3 pickup.
    pub layer: u8,
    /// The surface's depth program state (the material's MAIN state).
    pub depth_state: RenderState,
    pub depth_hack: f32,
}

/// Published by the app with the FX frame.
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct DarkVisionSurfaces(pub Arc<Vec<DarkVisionSurface>>);

/// Installed program state and registered layer constants.
pub struct DarkVisionLayerSources {
    pub colour_state: RenderState,
    /// Device then pickup: tint rgb and opacity.
    pub tints: [[f32; 4]; 2],
    pub depth_bias: f32,
}

pub struct LayerInputs<'a> {
    pub record: &'a DarkVisionRecord,
    pub mask: u32,
    pub surfaces: &'a [DarkVisionSurface],
    pub matrices: &'a crate::rendering::view_history::ViewMatrices,
    pub colour_texture: &'a Texture,
    pub colour: &'a TextureView,
    pub format: TextureFormat,
    pub scene_depth: &'a TextureView,
    pub adaptation: &'a Buffer,
    pub extent: [u32; 2],
    pub host_units: f32,
    pub white_level: f32,
    pub compressed: bool,
}

const DRAW_STRIDE: u64 = 256;
const DEPTH_FORMAT: TextureFormat = TextureFormat::Depth32Float;

pub struct DarkVisionLayers {
    module: ShaderModule,
    layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    frame: Buffer,
    draws: Option<(u64, Buffer)>,
    targets: Option<([u32; 2], TextureFormat, Texture, TextureView, TextureView)>,
    depth_pipelines: HashMap<(MeshVertexBufferLayoutRef, [u32; 9]), RenderPipeline>,
    colour_pipelines: HashMap<(MeshVertexBufferLayoutRef, TextureFormat), RenderPipeline>,
}

impl DarkVisionLayers {
    pub fn new(device: &RenderDevice) -> Self {
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("dark vision layers"),
            source: ShaderSource::Wgsl(include_str!("../../../assets/shaders/dh2_dark_vision_layers.wgsl").into()),
        });
        let uniform = |binding, dynamic| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            ty: BindingType::Buffer { ty: BufferBindingType::Uniform, has_dynamic_offset: dynamic, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(Some("dark vision layers"), &[
            uniform(0, false),
            uniform(1, true),
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture { sample_type: TextureSampleType::Float { filterable: false },
                    view_dimension: TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture { sample_type: TextureSampleType::Depth,
                    view_dimension: TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer { ty: BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
        ]);
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("dark vision layers"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let frame = device.create_buffer(&BufferDescriptor {
            label: Some("dark vision layer frame"),
            size: 128,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { module, layout, pipeline_layout, frame, draws: None, targets: None,
            depth_pipelines: HashMap::default(), colour_pipelines: HashMap::default() }
    }

    /// The record's layers that this pass draws: device and pickup.
    pub fn active(mask: u32, surfaces: &[DarkVisionSurface]) -> bool {
        surfaces.iter().any(|surface| layer_enabled(mask, surface.layer))
    }

    pub fn record(&mut self, device: &RenderDevice, queue: &RenderQueue, encoder: &mut CommandEncoder,
        sources: &DarkVisionLayerSources, inputs: &LayerInputs<'_>, allocator: &MeshAllocator,
        meshes: &RenderAssets<RenderMesh>) -> Result<(), String> {
        let surfaces: Vec<&DarkVisionSurface> = inputs.surfaces.iter().filter(|surface| layer_enabled(inputs.mask, surface.layer)).collect();
        if surfaces.is_empty() { return Ok(()); }
        if std::env::var_os("DH2_TRACE_DARK_VISION_LAYERS").is_some() {
            let device = surfaces.iter().filter(|surface| surface.layer == 1).count();
            eprintln!("DARK_VISION_LAYERS mask={:#x} device={device} pickup={} reveal={:?} power={}",
                inputs.mask, surfaces.len() - device, inputs.record.reveal_radius, inputs.record.reveal_power);
        }
        // 1402bb270 copies hdrBuffer before the layers read it as ColorIn.
        if self.targets.as_ref().is_none_or(|(extent, format, ..)| *extent != inputs.extent || *format != inputs.format) {
            let size = Extent3d { width: inputs.extent[0], height: inputs.extent[1], depth_or_array_layers: 1 };
            let texture = |label, format, usage| device.create_texture(&TextureDescriptor {
                label: Some(label), size, mip_level_count: 1, sample_count: 1, dimension: TextureDimension::D2,
                format, usage, view_formats: &[],
            });
            let copy = texture("dark vision colour in", inputs.format, TextureUsages::COPY_DST | TextureUsages::TEXTURE_BINDING);
            let depth = texture("dark vision layer depth", DEPTH_FORMAT, TextureUsages::RENDER_ATTACHMENT);
            let copy_view = copy.create_view(&TextureViewDescriptor::default());
            let depth_view = depth.create_view(&TextureViewDescriptor::default());
            self.targets = Some((inputs.extent, inputs.format, copy, copy_view, depth_view));
        }
        let (_, _, copy, copy_view, depth_view) = self.targets.as_ref().unwrap();
        encoder.copy_texture_to_texture(inputs.colour_texture.as_image_copy(), copy.as_image_copy(),
            Extent3d { width: inputs.extent[0], height: inputs.extent[1], depth_or_array_layers: 1 });

        // Frame constants.
        let view_projection = Mat4::from_cols_array_2d(&inputs.matrices.view_projection).transpose();
        let projection = Mat4::from_cols_array_2d(&inputs.matrices.projection).transpose();
        let [inner, outer] = DarkVisionRecord::radii(inputs.record.reveal_radius);
        let mut values = [0f32; 32];
        values[..16].copy_from_slice(&view_projection.to_cols_array());
        values[16..19].copy_from_slice(&inputs.matrices.position);
        values[19] = depth_at_bias(projection, sources.depth_bias)?;
        values[20..24].copy_from_slice(&[inner, outer, inputs.record.reveal_power, inputs.host_units]);
        values[24..27].copy_from_slice(&inputs.record.centre);
        values[27] = inputs.white_level;
        values[28..32].copy_from_slice(&[f32::from(u8::from(inputs.compressed)), inputs.extent[0] as f32, inputs.extent[1] as f32, 0.]);
        queue.write_buffer(&self.frame, 0, &bytes(&values));

        // Per-draw constants.
        let needed = (surfaces.len() as u64).max(1) * DRAW_STRIDE;
        if self.draws.as_ref().is_none_or(|(size, _)| *size < needed) {
            let size = needed.next_power_of_two();
            self.draws = Some((size, device.create_buffer(&BufferDescriptor {
                label: Some("dark vision layer draws"), size,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST, mapped_at_creation: false,
            })));
        }
        let draw_buffer = &self.draws.as_ref().unwrap().1;
        let mut data = vec![0u8; surfaces.len() * DRAW_STRIDE as usize];
        for (index, surface) in surfaces.iter().enumerate() {
            let normal = surface.model.inverse().transpose();
            if !normal.is_finite() { return Err("dark vision surface transform is singular".into()); }
            let tint = sources.tints[usize::from(surface.layer == 3)];
            // 14031f960 binds darkVisionDepthClip per layer: +0x4260 device, +0x4268 pickup.
            let clip = inputs.record.depth_clip[usize::from(surface.layer - 1)];
            let mut draw = [0f32; 40];
            draw[..16].copy_from_slice(&surface.model.to_cols_array());
            draw[16..32].copy_from_slice(&normal.to_cols_array());
            draw[32..36].copy_from_slice(&tint);
            draw[36..38].copy_from_slice(&[clip, surface.depth_hack]);
            let start = index * DRAW_STRIDE as usize;
            data[start..start + 160].copy_from_slice(&bytes(&draw));
        }
        queue.write_buffer(draw_buffer, 0, &data);
        let binding = device.create_bind_group(Some("dark vision layers"), &self.layout, &[
            BindGroupEntry { binding: 0, resource: self.frame.as_entire_binding() },
            BindGroupEntry { binding: 1, resource: BindingResource::Buffer(BufferBinding {
                buffer: draw_buffer, offset: 0, size: std::num::NonZeroU64::new(160) }) },
            BindGroupEntry { binding: 2, resource: BindingResource::TextureView(copy_view) },
            BindGroupEntry { binding: 3, resource: BindingResource::TextureView(inputs.scene_depth) },
            BindGroupEntry { binding: 4, resource: inputs.adaptation.as_entire_binding() },
        ]);

        // Resolve each surface's mesh allocation once.
        let mut prepared = Vec::with_capacity(surfaces.len());
        for surface in &surfaces {
            let render_mesh = meshes.get(surface.mesh).ok_or("dark vision surface mesh has no RenderMesh")?;
            let (Some(vertex), Some(index), Some(index_format)) = (allocator.mesh_vertex_slice(&surface.mesh),
                allocator.mesh_index_slice(&surface.mesh), render_mesh.index_format()) else {
                return Err("dark vision surface mesh has no indexed GPU allocation".into());
            };
            prepared.push((render_mesh.layout.clone(), vertex, index, index_format));
        }
        for (surface, (layout, ..)) in surfaces.iter().zip(&prepared) {
            let key = (layout.clone(), surface.depth_state);
            if !self.depth_pipelines.contains_key(&key) {
                let pipeline = self.depth_pipeline(device, layout, &surface.depth_state)?;
                self.depth_pipelines.insert(key, pipeline);
            }
            let key = (layout.clone(), inputs.format);
            if !self.colour_pipelines.contains_key(&key) {
                let pipeline = self.colour_pipeline(device, layout, &sources.colour_state, inputs.format)?;
                self.colour_pipelines.insert(key, pipeline);
            }
        }

        // Device then pickup, each a depth prepass and its colour draw.
        let mut cleared = false;
        for layer in [1u8, 3] {
            let members: Vec<usize> = surfaces.iter().enumerate().filter(|(_, s)| s.layer == layer).map(|(i, _)| i).collect();
            if members.is_empty() { continue; }
            {
                let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                    label: Some("dark vision layer depth"),
                    color_attachments: &[],
                    depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                        view: depth_view,
                        depth_ops: Some(Operations { load: if cleared { LoadOp::Load } else { LoadOp::Clear(0.0) }, store: StoreOp::Store }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                cleared = true;
                for &member in &members {
                    let (layout, vertex, index, format) = &prepared[member];
                    pass.set_pipeline(&self.depth_pipelines[&(layout.clone(), surfaces[member].depth_state)]);
                    pass.set_bind_group(0, &binding, &[(member as u64 * DRAW_STRIDE) as u32]);
                    pass.set_vertex_buffer(0, *vertex.buffer.slice(..));
                    pass.set_index_buffer(*index.buffer.slice(..), *format);
                    pass.draw_indexed(index.range.clone(), vertex.range.start as i32, 0..1);
                }
            }
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("dark vision layer colour"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: inputs.colour, depth_slice: None, resolve_target: None,
                    ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(Operations { load: LoadOp::Load, store: StoreOp::Store }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            for &member in &members {
                let (layout, vertex, index, format) = &prepared[member];
                pass.set_pipeline(&self.colour_pipelines[&(layout.clone(), inputs.format)]);
                pass.set_bind_group(0, &binding, &[(member as u64 * DRAW_STRIDE) as u32]);
                pass.set_vertex_buffer(0, *vertex.buffer.slice(..));
                pass.set_index_buffer(*index.buffer.slice(..), *format);
                pass.draw_indexed(index.range.clone(), vertex.range.start as i32, 0..1);
            }
        }
        Ok(())
    }

    fn depth_pipeline(&self, device: &RenderDevice, layout: &MeshVertexBufferLayoutRef, state: &RenderState)
        -> Result<RenderPipeline, String> {
        let buffer = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0)])
            .map_err(|error| format!("dark vision depth vertex layout: {error}"))?;
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(state, false);
        Ok(device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("dark vision layer depth"),
            layout: Some(&self.pipeline_layout),
            vertex: RawVertexState {
                module: &self.module, entry_point: Some("dark_vision_depth_vertex"),
                compilation_options: Default::default(),
                buffers: &[RawVertexBufferLayout { array_stride: buffer.array_stride, step_mode: buffer.step_mode, attributes: &buffer.attributes }],
            },
            fragment: None,
            primitive: PrimitiveState { front_face, cull_mode, ..Default::default() },
            depth_stencil: Some(super::pipeline_state::depth_stencil_gpu(state, DEPTH_FORMAT)?),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        }))
    }

    fn colour_pipeline(&self, device: &RenderDevice, layout: &MeshVertexBufferLayoutRef, state: &RenderState,
        format: TextureFormat) -> Result<RenderPipeline, String> {
        let buffer = layout.0.get_layout(&[Mesh::ATTRIBUTE_POSITION.at_shader_location(0), Mesh::ATTRIBUTE_NORMAL.at_shader_location(1)])
            .map_err(|error| format!("dark vision colour vertex layout: {error}"))?;
        let target = crate::render_state::blend_state(state).targets[0].ok_or("dark vision target state absent")?;
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(state, false);
        Ok(device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("dark vision layer colour"),
            layout: Some(&self.pipeline_layout),
            vertex: RawVertexState {
                module: &self.module, entry_point: Some("dark_vision_layer_vertex"),
                compilation_options: Default::default(),
                buffers: &[RawVertexBufferLayout { array_stride: buffer.array_stride, step_mode: buffer.step_mode, attributes: &buffer.attributes }],
            },
            fragment: Some(RawFragmentState {
                module: &self.module, entry_point: Some("dark_vision_layer_fragment"),
                compilation_options: Default::default(),
                targets: &[Some(ColorTargetState { format, blend: super::pipeline_state::blend_target(target)?,
                    write_mask: ColorWrites::from_bits_truncate(target.write_mask as u32) })],
            }),
            primitive: PrimitiveState { front_face, cull_mode, ..Default::default() },
            depth_stencil: Some(super::pipeline_state::depth_stencil_gpu(state, DEPTH_FORMAT)?),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        }))
    }
}

/// Mask bit 1 draws the device layer (1), bit 4 the pickup layer (3).
fn layer_enabled(mask: u32, layer: u8) -> bool {
    match layer { 1 => mask & 1 != 0, 3 => mask & 4 != 0, _ => false }
}

/// The program's bias term: the projected depth of a point darkVisionDepthBias
/// ahead of the camera ((bias*P22 + P32)/bias), or -1 for a zero bias.
fn depth_at_bias(projection: Mat4, bias: f32) -> Result<f32, String> {
    if bias == 0. { return Ok(-1.); }
    for z in [bias, -bias] {
        let clip = projection * bevy::math::Vec4::new(0., 0., z, 1.);
        if (clip.w - bias).abs() <= f32::EPSILON * bias.abs().max(1.) {
            return Ok(clip.z / clip.w);
        }
    }
    Err("dark vision depth bias: the view projection has no w = view distance row".into())
}

fn bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}
