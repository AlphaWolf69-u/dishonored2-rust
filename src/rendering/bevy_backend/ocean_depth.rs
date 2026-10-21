//! Displaced arkOcean grid rasterization, including normal-depth motion output.
//! View admission, prepared 303500 state, patch uniforms, target preservation
//! and wave-history ownership remain explicit responsibilities of the caller.
use super::ocean_mesh::{OceanMeshBuffers, OceanPatchDraw};
use bevy::{
    prelude::default,
    render::{render_resource::*, renderer::RenderDevice},
};

pub const SHADER_SOURCE: &str = concat!(
    include_str!("../../../assets/shaders/dh2_ocean_noise.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_surface.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_motion_vector.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_vertex.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_depth.wgsl"),
);

/// PerViewCB/PerInstanceCB transport assembled from the two verified layouts.
pub fn uniform_bytes(
    surface: &super::ocean_surface::OceanSurfaceParameters,
    vertex: &super::ocean_vertex::OceanVertexParameters,
) -> [u8; 288] {
    let mut bytes = [0; 288];
    bytes[..112].copy_from_slice(&surface.uniform_bytes());
    bytes[112..].copy_from_slice(&vertex.uniform_bytes());
    bytes
}

pub struct OceanDepthProgram {
    pub parameters: BindGroupLayout,
    pub textures: BindGroupLayout,
    strip: RenderPipeline,
    triangles: RenderPipeline,
    stencil_reference: u32,
    alpha_depth: bool,
}

impl OceanDepthProgram {
    /// `state` is the actual selected program state AFTER ocean_draw_state.
    /// Alpha depth has no colour attachment or fragment stage (cached 14076).
    /// Normal depth uses cached 14068/31 and the source RG16F motion target.
    pub fn new(
        device: &RenderDevice,
        depth_format: TextureFormat,
        state: crate::render_state::RenderState,
        stencil_reference: u32,
        alpha_depth: bool,
    ) -> Result<Self, String> {
        if !depth_format.has_stencil_aspect() {
            return Err("ocean water depth requires the owner's depth/stencil attachment".into());
        }
        let raster = crate::render_state::raster_state(&state);
        if raster.antialiased_line || raster.multisample {
            return Err(
                "ocean depth transport requires the source single-sample solid/line raster state"
                    .into(),
            );
        }
        let features = device.wgpu_device().features();
        if depth_format == TextureFormat::Depth32FloatStencil8
            && !features.contains(bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8)
        {
            return Err("ocean depth requires enabled DEPTH32FLOAT_STENCIL8".into());
        }
        if !raster.depth_clip
            && !features.contains(bevy::render::settings::WgpuFeatures::DEPTH_CLIP_CONTROL)
        {
            return Err("ocean depth requires enabled DEPTH_CLIP_CONTROL".into());
        }
        if raster.fill_mode == 2
            && !features.contains(bevy::render::settings::WgpuFeatures::POLYGON_MODE_LINE)
        {
            return Err("ocean wireframe requires enabled POLYGON_MODE_LINE".into());
        }
        let parameters = device.create_bind_group_layout(
            Some("ocean depth parameters"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: BufferSize::new(288),
                },
                count: None,
            }],
        );
        let entries = (0..11)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::VERTEX,
                count: None,
                ty: if binding < 8 {
                    BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    }
                } else {
                    BindingType::Sampler(SamplerBindingType::Filtering)
                },
            })
            .collect::<Vec<_>>();
        let textures = device.create_bind_group_layout(Some("ocean depth wave inputs"), &entries);
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("ocean depth"),
            bind_group_layouts: &[Some(&parameters), Some(&textures)],
            immediate_size: 0,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("arkOcean depth"),
            source: ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&state, false);
        let depth = super::pipeline_state::depth_stencil_gpu(&state, depth_format)?;
        let target = crate::render_state::blend_state(&state).targets[0].unwrap();
        let motion = ColorTargetState {
            // 36d0c0's _MotionVectors descriptor: packed0x5c5 -> DXGI34.
            format: TextureFormat::Rg16Float,
            blend: super::pipeline_state::blend_target(target)?,
            write_mask: ColorWrites::from_bits_truncate(u32::from(target.write_mask)),
        };
        let motion_targets = [Some(motion)];
        let pipeline = |topology| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("ocean displaced depth"),
                layout: Some(&layout),
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("ocean_depth_vertex"),
                    compilation_options: default(),
                    buffers: &[RawVertexBufferLayout {
                        array_stride: 8,
                        step_mode: VertexStepMode::Vertex,
                        attributes: &[VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        }],
                    }],
                },
                fragment: if alpha_depth {
                    None
                } else {
                    Some(RawFragmentState {
                        module: &shader,
                        entry_point: Some("ocean_depth_fragment"),
                        compilation_options: default(),
                        targets: &motion_targets,
                    })
                },
                primitive: PrimitiveState {
                    topology,
                    strip_index_format: if topology == PrimitiveTopology::TriangleStrip {
                        Some(IndexFormat::Uint16)
                    } else {
                        None
                    },
                    front_face,
                    cull_mode,
                    unclipped_depth: !raster.depth_clip,
                    polygon_mode: if raster.fill_mode == 2 {
                        PolygonMode::Line
                    } else {
                        PolygonMode::Fill
                    },
                    ..default()
                },
                depth_stencil: Some(depth.clone()),
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        };
        Ok(Self {
            parameters,
            textures,
            strip: pipeline(PrimitiveTopology::TriangleStrip),
            triangles: pipeline(PrimitiveTopology::TriangleList),
            stencil_reference,
            alpha_depth,
        })
    }

    pub fn parameter_binding(&self, device: &RenderDevice, buffer: &Buffer) -> BindGroup {
        device.create_bind_group(
            Some("ocean depth patch constants"),
            &self.parameters,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer,
                    offset: 0,
                    size: BufferSize::new(288),
                }),
            }],
        )
    }

    /// Inputs in shader binding order: noise permutation/gradient, shore,
    /// current/previous displacement, collision height, FFT foam, amplitude.
    /// The owner caches this binding until an input allocation/history changes.
    pub fn texture_binding(
        &self,
        device: &RenderDevice,
        images: [&TextureView; 8],
        samplers: [&Sampler; 3],
    ) -> BindGroup {
        let mut entries = images
            .into_iter()
            .enumerate()
            .map(|(i, image)| BindGroupEntry {
                binding: i as u32,
                resource: BindingResource::TextureView(image),
            })
            .collect::<Vec<_>>();
        entries.extend(
            samplers
                .into_iter()
                .enumerate()
                .map(|(i, sampler)| BindGroupEntry {
                    binding: 8 + i as u32,
                    resource: BindingResource::Sampler(sampler),
                }),
        );
        device.create_bind_group(Some("ocean depth wave inputs"), &self.textures, &entries)
    }

    /// Record the admitted leaves in source order, strips before edge triangles.
    /// Attachments were initialized by the water-depth owner and are preserved.
    pub fn record(
        &self,
        encoder: &mut CommandEncoder,
        depth: &TextureView,
        motion: Option<&TextureView>,
        viewport: [f32; 4],
        scissor: [u32; 4],
        mesh: &OceanMeshBuffers,
        textures: &BindGroup,
        draws: &[OceanPatchDraw<'_>],
    ) -> Result<(), String> {
        if self.alpha_depth != motion.is_none() {
            return Err("ocean depth mode and motion attachment differ".into());
        }
        if draws.iter().any(|draw| draw.record >= mesh.records.len()) {
            return Err("ocean geometry record out of range".into());
        }
        let color = [motion.map(|view| RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Load,
                store: StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("ocean displaced depth"),
            color_attachments: if self.alpha_depth { &[] } else { &color },
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
        pass.set_bind_group(1, textures, &[]);
        pass.set_stencil_reference(self.stencil_reference);
        pass.set_vertex_buffer(0, *mesh.vertices.slice(..));
        pass.set_index_buffer(*mesh.indices.slice(..), IndexFormat::Uint16);
        for draw in draws {
            let record = &mesh.records[draw.record];
            pass.set_bind_group(0, draw.parameters, &[draw.parameter_offset]);
            if record.strip_primitive_count != 0 {
                pass.set_pipeline(&self.strip);
                let start = record.strip_index_offset;
                pass.draw_indexed(start..start + record.strip_primitive_count + 2, 0, 0..1);
            }
            if record.edge_triangle_count != 0 {
                pass.set_pipeline(&self.triangles);
                let start = record.edge_index_offset;
                pass.draw_indexed(start..start + record.edge_triangle_count * 3, 0, 0..1);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "ocean_depth/tests.rs"]
mod tests;
