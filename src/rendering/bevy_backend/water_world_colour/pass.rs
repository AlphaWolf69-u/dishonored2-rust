//! Horizontal world water with per-vertex probes and the complete shared shading.
use super::super::{
    colour_output::ColourOutputParameters,
    water_colour::WaterColourParameters,
    water_reflection::WaterReflectionParameters,
    water_refraction::{WaterRefractionParameters, WaterShadowParameters},
    water_surface::WaterSurfaceParameters,
};
use super::*;
use bevy::{mesh::MeshVertexBufferLayoutRef, prelude::default};
use std::ops::Range;

pub struct WorldProbeColourParameters {
    pub reflection: WaterReflectionParameters,
    pub refraction: WaterRefractionParameters,
    pub shadow: WaterShadowParameters,
    pub surface: WaterSurfaceParameters,
    pub colour: WaterColourParameters,
    pub output: ColourOutputParameters,
    pub sky_radius: [f32; 4],
    pub use_sky_cube: bool,
}
impl WorldProbeColourParameters {
    pub fn uniform_bytes(&self) -> Result<[u8; 1008], String> {
        if !self.colour.horizontal {
            return Err("world probe colour stage requires the horizontal specialization".into());
        }
        let mut bytes = Vec::with_capacity(1008);
        bytes.extend(self.reflection.uniform_bytes());
        bytes.extend(super::super::water_refraction::uniform_bytes(
            &self.refraction,
            &self.shadow,
        ));
        bytes.extend(self.surface.uniform_bytes());
        bytes.extend(self.colour.uniform_bytes());
        bytes.extend(self.output.uniform_bytes());
        bytes.extend(self.sky_radius.into_iter().flat_map(f32::to_le_bytes));
        bytes.extend(
            [u32::from(self.use_sky_cube), 0, 0, 0]
                .into_iter()
                .flat_map(u32::to_le_bytes),
        );
        Ok(bytes.try_into().expect("world probe colour uniform layout"))
    }
}

pub struct WorldProbeColourProgram {
    pub vertex: WorldProbeVertexStage,
    pub parameters: BindGroupLayout,
    pub resources: BindGroupLayout,
    pipeline: RenderPipeline,
    view_exposure: bool,
}
impl WorldProbeColourProgram {
    /// State must include the world's material/surface overrides. This is the
    /// VS14494 horizontal IBL colour path, not a generic water-sort program.
    pub fn new(
        device: &RenderDevice,
        mesh: &MeshVertexBufferLayoutRef,
        depth_format: TextureFormat,
        colour_format: TextureFormat,
        state: crate::render_state::RenderState,
    ) -> Result<Self, String> {
        Self::create(
            device,
            mesh,
            depth_format,
            colour_format,
            state,
            false,
            false,
        )
    }

    /// Gameplay variant: every water HDR decode/encode reads the actual
    /// MaterialExposure buffer, without a CPU readback or another GPU pass.
    pub fn new_with_view_exposure(
        device: &RenderDevice,
        mesh: &MeshVertexBufferLayoutRef,
        depth_format: TextureFormat,
        colour_format: TextureFormat,
        state: crate::render_state::RenderState,
    ) -> Result<Self, String> {
        Self::create(
            device,
            mesh,
            depth_format,
            colour_format,
            state,
            true,
            false,
        )
    }

    pub fn new_for_world_surface(
        device: &RenderDevice,
        mesh: &MeshVertexBufferLayoutRef,
        depth_format: TextureFormat,
        colour_format: TextureFormat,
        state: crate::render_state::RenderState,
        indices_reversed: bool,
    ) -> Result<Self, String> {
        Self::create(
            device,
            mesh,
            depth_format,
            colour_format,
            state,
            true,
            indices_reversed,
        )
    }

    fn create(
        device: &RenderDevice,
        mesh: &MeshVertexBufferLayoutRef,
        depth_format: TextureFormat,
        colour_format: TextureFormat,
        state: crate::render_state::RenderState,
        view_exposure: bool,
        indices_reversed: bool,
    ) -> Result<Self, String> {
        let raster = crate::render_state::raster_state(&state);
        if raster.antialiased_line || raster.multisample {
            return Err("world water requires single-sample raster state".into());
        }
        for (needed, feature) in [
            (
                !raster.depth_clip,
                bevy::render::settings::WgpuFeatures::DEPTH_CLIP_CONTROL,
            ),
            (
                raster.fill_mode == 2,
                bevy::render::settings::WgpuFeatures::POLYGON_MODE_LINE,
            ),
            (
                depth_format == TextureFormat::Depth32FloatStencil8,
                bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
            ),
        ] {
            if needed && !device.features().contains(feature) {
                return Err(format!("world water requires device feature {feature:?}"));
            }
        }
        let vertex = WorldProbeVertexStage::new(device);
        let parameters = device.create_bind_group_layout(
            Some("world water fragment parameters"),
            &[BindGroupLayoutEntry {
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                count: None,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: BufferSize::new(1008),
                },
            }],
        );
        let mut entries: Vec<_> = (0..16)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::FRAGMENT,
                count: None,
                ty: if binding >= 10 {
                    BindingType::Sampler(if binding == 15 {
                        SamplerBindingType::Comparison
                    } else {
                        SamplerBindingType::Filtering
                    })
                } else {
                    BindingType::Texture {
                        sample_type: match binding {
                            2 | 9 => TextureSampleType::Depth,
                            3 => TextureSampleType::Float { filterable: false },
                            _ => TextureSampleType::Float { filterable: true },
                        },
                        view_dimension: match binding {
                            6 | 9 => TextureViewDimension::D2Array,
                            7 => TextureViewDimension::CubeArray,
                            8 => TextureViewDimension::Cube,
                            _ => TextureViewDimension::D2,
                        },
                        multisampled: false,
                    }
                },
            })
            .collect();
        if view_exposure {
            entries.push(BindGroupLayoutEntry {
                binding: 16,
                visibility: ShaderStages::FRAGMENT,
                count: None,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(32),
                },
            });
        }
        let resources =
            device.create_bind_group_layout(Some("world water fragment sources"), &entries);
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("world water colour"),
            bind_group_layouts: &[
                Some(&vertex.parameters),
                Some(&vertex.inputs),
                Some(&parameters),
                Some(&resources),
            ],
            immediate_size: 0,
        });
        let source = format!(
            "{}\n{}\n{}\n{}\n{}",
            super::super::water_colour::shader_source(),
            super::super::colour_output::SHADER_SOURCE,
            include_str!("../../../../assets/shaders/dh2_water_world_colour_parameters.wgsl"),
            include_str!("../../../../assets/shaders/dh2_water_world_colour_pass.wgsl"),
            if view_exposure {
                include_str!("../../../../assets/shaders/dh2_water_world_colour_exposure.wgsl")
            } else {
                ""
            },
        );
        let fragment = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("arkWater world probe colour"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let mesh = vertex_buffer_layout(mesh).map_err(|e| e.to_string())?;
        let (front_face, cull_mode) =
            super::super::pipeline_state::mesh_raster(&state, indices_reversed);
        let depth = super::super::pipeline_state::depth_stencil_gpu(&state, depth_format)?;
        let target = crate::render_state::blend_state(&state).targets[0]
            .ok_or("world water has no colour target state")?;
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("world water colour"),
            layout: Some(&layout),
            vertex: RawVertexState {
                module: &vertex.module,
                entry_point: Some("water_world_colour_vertex"),
                compilation_options: default(),
                buffers: &[RawVertexBufferLayout {
                    array_stride: mesh.array_stride,
                    step_mode: mesh.step_mode,
                    attributes: &mesh.attributes,
                }],
            },
            fragment: Some(RawFragmentState {
                module: &fragment,
                entry_point: Some(if view_exposure {
                    "water_world_colour_fragment_with_exposure"
                } else {
                    "water_world_colour_fragment"
                }),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: colour_format,
                    blend: super::super::pipeline_state::blend_target(target)?,
                    write_mask: ColorWrites::from_bits_truncate(target.write_mask.into()),
                })],
            }),
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
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
            depth_stencil: Some(depth),
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            vertex,
            parameters,
            resources,
            pipeline,
            view_exposure,
        })
    }
    pub fn parameter_binding(&self, device: &RenderDevice, buffer: &Buffer) -> BindGroup {
        device.create_bind_group(
            Some("world water fragment parameters"),
            &self.parameters,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer,
                    offset: 0,
                    size: BufferSize::new(1008),
                }),
            }],
        )
    }
    /// Permutation, gradients, opaque depth, reduction, HDR, fog, caustics,
    /// probe array, sky, shadow atlas. Samplers: point wrap, linear clamp,
    /// probes, sky, caustics, comparison; each is the effective material sampler.
    pub fn resource_binding(
        &self,
        device: &RenderDevice,
        images: [&TextureView; 10],
        samplers: [&Sampler; 6],
    ) -> BindGroup {
        self.bind_resources(device, images, samplers, None)
    }

    pub fn resource_binding_with_view_exposure(
        &self,
        device: &RenderDevice,
        images: [&TextureView; 10],
        samplers: [&Sampler; 6],
        adaptation: &Buffer,
    ) -> BindGroup {
        self.bind_resources(device, images, samplers, Some(adaptation))
    }

    fn bind_resources(
        &self,
        device: &RenderDevice,
        images: [&TextureView; 10],
        samplers: [&Sampler; 6],
        adaptation: Option<&Buffer>,
    ) -> BindGroup {
        assert_eq!(
            self.view_exposure,
            adaptation.is_some(),
            "water exposure binding does not match its pipeline"
        );
        let mut entries: Vec<_> = images
            .into_iter()
            .enumerate()
            .map(|(i, v)| BindGroupEntry {
                binding: i as u32,
                resource: BindingResource::TextureView(v),
            })
            .collect();
        entries.extend(
            samplers
                .into_iter()
                .enumerate()
                .map(|(i, v)| BindGroupEntry {
                    binding: 10 + i as u32,
                    resource: BindingResource::Sampler(v),
                }),
        );
        if let Some(adaptation) = adaptation {
            entries.push(BindGroupEntry {
                binding: 16,
                resource: adaptation.as_entire_binding(),
            });
        }
        device.create_bind_group(
            Some("world water fragment sources"),
            &self.resources,
            &entries,
        )
    }
    /// The graph owner supplies attachments, viewport/scissor and original sort.
    pub fn draw<'a>(
        &'a self,
        pass: &mut bevy::render::render_phase::TrackedRenderPass<'a>,
        draw: WorldProbeColourDraw<'a>,
        stencil_reference: u32,
    ) {
        pass.set_render_pipeline(&self.pipeline);
        pass.set_bind_group(0, draw.vertex_parameters, &[draw.vertex_offset]);
        pass.set_bind_group(1, draw.vertex_sources, &[]);
        pass.set_bind_group(2, draw.fragment_parameters, &[draw.fragment_offset]);
        pass.set_bind_group(3, draw.resources, &[]);
        pass.set_stencil_reference(stencil_reference);
        pass.set_vertex_buffer(0, draw.vertices.slice(..));
        if let Some((indices, format, range, base)) = draw.indices {
            pass.set_index_buffer(indices.slice(..), format);
            pass.draw_indexed(range, base, draw.instances);
        } else {
            pass.draw(draw.vertex_range, draw.instances);
        }
    }
}
pub struct WorldProbeColourDraw<'a> {
    pub vertex_parameters: &'a BindGroup,
    pub vertex_offset: u32,
    pub vertex_sources: &'a BindGroup,
    pub fragment_parameters: &'a BindGroup,
    pub fragment_offset: u32,
    pub resources: &'a BindGroup,
    pub vertices: &'a Buffer,
    pub indices: Option<(&'a Buffer, IndexFormat, Range<u32>, i32)>,
    pub vertex_range: Range<u32>,
    pub instances: Range<u32>,
}
