//! Ocean colour raster over the shared grid and explicit same-view resources.
use super::{
    colour_output::ColourOutputParameters,
    ocean_mesh::{OceanMeshBuffers, OceanPatchDraw},
    ocean_surface::OceanSurfaceParameters,
    ocean_vertex::OceanVertexParameters,
    water_reflection::WaterReflectionParameters,
    water_refraction::{WaterRefractionParameters, WaterShadowParameters},
    water_surface::WaterSurfaceParameters,
};
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
    include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_texture_array_sampling.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_lighting.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_environment.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_screen_ray.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_refraction.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_reflection.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_water_surface.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_colour_output.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_colour.wgsl"),
    "\n",
    include_str!("../../../assets/shaders/dh2_ocean_colour_pass.wgsl"),
);

#[derive(Clone, Copy, Debug)]
pub struct OceanColourParameters {
    pub projected: [f32; 4],
    pub water: [f32; 4],
    pub light: [f32; 4],
    pub sun: [f32; 4],
    pub pixel_to_texture: [f32; 4],
}
impl OceanColourParameters {
    pub fn uniform_bytes(&self) -> [u8; 80] {
        let mut bytes = [0; 80];
        for (i, value) in [
            self.projected,
            self.water,
            self.light,
            self.sun,
            self.pixel_to_texture,
        ]
        .into_iter()
        .flatten()
        .enumerate()
        {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

pub struct OceanColourView {
    pub surface: OceanSurfaceParameters,
    pub vertex: OceanVertexParameters,
    pub reflection: WaterReflectionParameters,
    pub refraction: WaterRefractionParameters,
    pub shadow: WaterShadowParameters,
    pub water: WaterSurfaceParameters,
    pub colour: OceanColourParameters,
    pub output: ColourOutputParameters,
    pub sky_radius: [f32; 4],
    pub use_sky_cube: bool,
}
impl OceanColourView {
    /// Refresh authored uniforms while retaining the actual view, clocks,
    /// exposure, shadow packet and selected program options. The level comes
    /// from 3014b0's prepared state, including its water-volume/retention rule.
    pub fn update_environment(
        &mut self,
        settings: &crate::rendering::ocean::OceanSettings,
        state: &crate::rendering::ocean::OceanViewState,
    ) {
        let water = &settings.water;
        self.surface = OceanSurfaceParameters::from_settings(
            &settings.surface,
            self.surface.clock_view[0],
            self.surface.camera[..3].try_into().unwrap(),
            self.surface.clock_view[2..4].try_into().unwrap(),
        );
        self.vertex.perlin_blend_end = settings.surface.perlin.blend_end;
        self.reflection.update_settings(&water.reflection);
        self.refraction.clock_caustics[1..].copy_from_slice(&[
            water.caustics.speed,
            water.caustics.tiling,
            water.caustics.chromatic_aberration,
        ]);
        self.refraction.unproject[3] = water.refraction;
        self.refraction.light_level[2] = water.caustics.fade;
        self.refraction.light_level[3] = state.water_level;
        self.water.underwater_visibility_boost = water.underwater_visibility_boost;
        self.water.shore_foam_luminance = water.shore_foam_luminance;
        let shadow = &settings.projected_shadow;
        self.colour.projected = [
            shadow.extent,
            shadow.start_height,
            shadow.end_height,
            state.water_level,
        ];
        self.colour.water = [
            water.colour[0],
            water.colour[1],
            water.colour[2],
            water.extinction,
        ];
        self.colour.light[3] = water.inscattering;
        self.colour.sun[3] = water.specular_power;
        self.sky_radius = [
            water.sky_colour[0],
            water.sky_colour[1],
            water.sky_colour[2],
            water.sky_cube_radius,
        ];
    }

    pub fn uniform_bytes(&self) -> [u8; 1200] {
        let mut bytes = [0; 1200];
        bytes[..112].copy_from_slice(&self.surface.uniform_bytes());
        bytes[112..288].copy_from_slice(&self.vertex.uniform_bytes());
        bytes[288..496].copy_from_slice(&self.reflection.uniform_bytes());
        bytes[496..1008].copy_from_slice(&super::water_refraction::uniform_bytes(
            &self.refraction,
            &self.shadow,
        ));
        bytes[1008..1040].copy_from_slice(&self.water.uniform_bytes());
        bytes[1040..1120].copy_from_slice(&self.colour.uniform_bytes());
        bytes[1120..1168].copy_from_slice(&self.output.uniform_bytes());
        for (i, value) in self.sky_radius.into_iter().enumerate() {
            bytes[1168 + i * 4..1172 + i * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[1184..1188].copy_from_slice(&u32::from(self.use_sky_cube).to_le_bytes());
        bytes
    }
}

pub struct OceanColourProgram {
    pub parameters: BindGroupLayout,
    pub waves: BindGroupLayout,
    pub scene: BindGroupLayout,
    strip: RenderPipeline,
    triangles: RenderPipeline,
    stencil_reference: u32,
}
impl OceanColourProgram {
    /// State is the selected installed program AFTER ocean_draw_state overrides.
    pub fn new(
        device: &RenderDevice,
        depth_format: TextureFormat,
        colour_format: TextureFormat,
        state: crate::render_state::RenderState,
        stencil_reference: u32,
    ) -> Result<Self, String> {
        Self::create(
            device,
            depth_format,
            colour_format,
            state,
            stencil_reference,
            false,
        )
    }
    pub fn with_view_exposure(
        device: &RenderDevice,
        depth_format: TextureFormat,
        colour_format: TextureFormat,
        state: crate::render_state::RenderState,
        stencil_reference: u32,
    ) -> Result<Self, String> {
        Self::create(
            device,
            depth_format,
            colour_format,
            state,
            stencil_reference,
            true,
        )
    }
    fn create(
        device: &RenderDevice,
        depth_format: TextureFormat,
        colour_format: TextureFormat,
        state: crate::render_state::RenderState,
        stencil_reference: u32,
        view_exposure: bool,
    ) -> Result<Self, String> {
        if !depth_format.has_stencil_aspect() {
            return Err("ocean colour requires the owner's depth/stencil target".into());
        }
        let limits = device.limits();
        if limits.max_sampled_textures_per_shader_stage < 18 {
            return Err("ocean colour requires 18 sampled fragment textures".into());
        }
        let raster = crate::render_state::raster_state(&state);
        if raster.antialiased_line || raster.multisample {
            return Err("ocean colour requires source single-sample raster state".into());
        }
        let features = device.features();
        for (required, feature) in [
            (
                depth_format == TextureFormat::Depth32FloatStencil8,
                bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
            ),
            (
                !raster.depth_clip,
                bevy::render::settings::WgpuFeatures::DEPTH_CLIP_CONTROL,
            ),
            (
                raster.fill_mode == 2,
                bevy::render::settings::WgpuFeatures::POLYGON_MODE_LINE,
            ),
        ] {
            if required && !features.contains(feature) {
                return Err(format!("ocean colour requires device feature {feature:?}"));
            }
        }
        let mut parameter_entries = vec![BindGroupLayoutEntry {
            binding: 0,
            visibility: ShaderStages::VERTEX_FRAGMENT,
            count: None,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: BufferSize::new(1200),
            },
        }];
        if view_exposure {
            parameter_entries.push(BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::FRAGMENT,
                count: None,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(32),
                },
            });
        }
        let parameters =
            device.create_bind_group_layout(Some("ocean colour constants"), &parameter_entries);
        let wave_entries = (0..11)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                visibility: if binding == 4 {
                    ShaderStages::VERTEX
                } else {
                    ShaderStages::VERTEX_FRAGMENT
                },
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
        let waves = device.create_bind_group_layout(Some("ocean waves"), &wave_entries);
        let scene_entries = (0..12)
            .map(|binding| BindGroupLayoutEntry {
                binding,
                visibility: ShaderStages::FRAGMENT,
                count: None,
                ty: if binding == 11 {
                    BindingType::Sampler(SamplerBindingType::Comparison)
                } else {
                    BindingType::Texture {
                        sample_type: match binding {
                            3 | 8 => TextureSampleType::Depth,
                            4 => TextureSampleType::Float { filterable: false },
                            _ => TextureSampleType::Float { filterable: true },
                        },
                        view_dimension: match binding {
                            7 | 8 => TextureViewDimension::D2Array,
                            9 => TextureViewDimension::Cube,
                            10 => TextureViewDimension::CubeArray,
                            _ => TextureViewDimension::D2,
                        },
                        multisampled: false,
                    }
                },
            })
            .collect::<Vec<_>>();
        let scene = device.create_bind_group_layout(Some("ocean scene inputs"), &scene_entries);
        let layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("ocean colour"),
            bind_group_layouts: &[Some(&parameters), Some(&waves), Some(&scene)],
            immediate_size: 0,
        });
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("arkOcean colour"),
            source: ShaderSource::Wgsl(if view_exposure {
                format!(
                    "{SHADER_SOURCE}\n{}",
                    include_str!("../../../assets/shaders/dh2_ocean_colour_exposure.wgsl")
                )
                .into()
            } else {
                SHADER_SOURCE.into()
            }),
        });
        let (front_face, cull_mode) = super::pipeline_state::mesh_raster(&state, false);
        let depth = super::pipeline_state::depth_stencil_gpu(&state, depth_format)?;
        let target = crate::render_state::blend_state(&state).targets[0]
            .ok_or("ocean colour has no target state")?;
        let colour = [Some(ColorTargetState {
            format: colour_format,
            blend: super::pipeline_state::blend_target(target)?,
            write_mask: ColorWrites::from_bits_truncate(u32::from(target.write_mask)),
        })];
        let pipeline = |topology| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some("ocean colour"),
                layout: Some(&layout),
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("ocean_colour_vertex"),
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
                fragment: Some(RawFragmentState {
                    module: &shader,
                    entry_point: Some(if view_exposure {
                        "ocean_colour_fragment_with_exposure"
                    } else {
                        "ocean_colour_fragment"
                    }),
                    compilation_options: default(),
                    targets: &colour,
                }),
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
            waves,
            scene,
            strip: pipeline(PrimitiveTopology::TriangleStrip),
            triangles: pipeline(PrimitiveTopology::TriangleList),
            stencil_reference,
        })
    }
    pub fn parameter_binding(&self, device: &RenderDevice, buffer: &Buffer) -> BindGroup {
        device.create_bind_group(
            Some("ocean colour patch constants"),
            &self.parameters,
            &[BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer,
                    offset: 0,
                    size: BufferSize::new(1200),
                }),
            }],
        )
    }
    pub fn parameter_binding_with_exposure(
        &self,
        device: &RenderDevice,
        buffer: &Buffer,
        adaptation: &Buffer,
    ) -> BindGroup {
        device.create_bind_group(
            Some("ocean patch constants and camera adaptation"),
            &self.parameters,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: BindingResource::Buffer(BufferBinding {
                        buffer,
                        offset: 0,
                        size: BufferSize::new(1200),
                    }),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: adaptation.as_entire_binding(),
                },
            ],
        )
    }
    /// Same eight wave images and three samplers as the depth program.
    pub fn wave_binding(
        &self,
        device: &RenderDevice,
        images: [&TextureView; 8],
        samplers: [&Sampler; 3],
    ) -> BindGroup {
        let mut entries = images
            .into_iter()
            .enumerate()
            .map(|(i, v)| BindGroupEntry {
                binding: i as u32,
                resource: BindingResource::TextureView(v),
            })
            .collect::<Vec<_>>();
        entries.extend(
            samplers
                .into_iter()
                .enumerate()
                .map(|(i, v)| BindGroupEntry {
                    binding: 8 + i as u32,
                    resource: BindingResource::Sampler(v),
                }),
        );
        device.create_bind_group(Some("ocean waves"), &self.waves, &entries)
    }
    /// Projected height, foam, foam depth, opaque depth, reduced depth, HDR, fog,
    /// caustics, shadow atlas, sky cube, probe array; then shadow comparison.
    pub fn scene_binding(
        &self,
        device: &RenderDevice,
        images: [&TextureView; 11],
        comparison: &Sampler,
    ) -> BindGroup {
        let mut entries = images
            .into_iter()
            .enumerate()
            .map(|(i, v)| BindGroupEntry {
                binding: i as u32,
                resource: BindingResource::TextureView(v),
            })
            .collect::<Vec<_>>();
        entries.push(BindGroupEntry {
            binding: 11,
            resource: BindingResource::Sampler(comparison),
        });
        device.create_bind_group(Some("ocean scene"), &self.scene, &entries)
    }
    pub fn record(
        &self,
        encoder: &mut CommandEncoder,
        depth: &TextureView,
        colour: &TextureView,
        viewport: [f32; 4],
        scissor: [u32; 4],
        mesh: &OceanMeshBuffers,
        waves: &BindGroup,
        scene: &BindGroup,
        draws: &[OceanPatchDraw<'_>],
    ) -> Result<(), String> {
        if draws.iter().any(|d| d.record >= mesh.records.len()) {
            return Err("ocean geometry record out of range".into());
        }
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("ocean colour"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: colour,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
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
        pass.set_bind_group(1, waves, &[]);
        pass.set_bind_group(2, scene, &[]);
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
#[path = "ocean_colour/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ocean_colour/raster_tests.rs"]
mod raster_tests;
