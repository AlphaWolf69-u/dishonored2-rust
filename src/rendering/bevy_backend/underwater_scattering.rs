//! Underwater radiance into the fog source, before its filtering and upscale.
use super::{
    coarse_depth::DepthPyramid,
    layered_images::LayeredImage,
    water_hdr::WaterHdr,
    water_refraction::{WaterRefractionParameters, WaterShadowParameters},
    water_scene::*,
};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        view::ViewTarget,
    },
};
use std::{collections::HashMap, sync::Arc};

pub struct UnderwaterProgram {
    pub state: crate::render_state::RenderState,
    pub parameters: [crate::render_program::SerializedParameterBlock; 2],
    pub ray_states: [crate::render_state::RenderState; 2],
    pub ray_parameters: [crate::render_program::SerializedParameterBlock; 2],
    pub chromatic: bool,
}
impl UnderwaterProgram {
    pub fn load(
        catalog: &crate::archive::Catalog,
        library: &mut crate::shader_cache::ProgramLibrary,
    ) -> Result<Self, String> {
        let path = "generated/decls/material/water/underwaterpostfx.material.decl";
        let bytes = catalog
            .read(path)
            .map_err(|e| e.to_string())?
            .ok_or("underwater material absent")?;
        let name = crate::material::authored_program_override(&bytes, "mainProgram")
            .ok_or("underwater program absent")?;
        let state = crate::render_state::material_render_state(
            &crate::entities::declaration_text(&bytes),
            library
                .definition(name.as_bytes())?
                .ok_or("underwater definition absent")?
                .render_state(),
        )?;
        let mut ray_states = [[0; 9]; 2];
        let mut ray_parameters = None;
        let mut chromatic = false;
        for (i, material) in ["computegodrays", "computeshadowedgodrays"]
            .into_iter()
            .enumerate()
        {
            let bytes = catalog
                .read(&format!(
                    "generated/decls/material/water/{material}.material.decl"
                ))
                .map_err(|e| e.to_string())?
                .ok_or("underwater ray material absent")?;
            let name = crate::material::authored_program_override(&bytes, "mainProgram")
                .ok_or("underwater ray program absent")?;
            ray_states[i] = crate::render_state::material_render_state(
                &crate::entities::declaration_text(&bytes),
                library
                    .definition(name.as_bytes())?
                    .ok_or("underwater ray definition absent")?
                    .render_state(),
            )?;
            let options = crate::render_program::SerializedParameterBlock {
                flags: 0,
                parameters: Vec::new(),
            };
            chromatic = library.option_value(
                name.as_bytes(),
                &options,
                b"water/EnableGodRaysChromaticAberrationsOption",
            )? != 0;
            if i == 0 {
                ray_parameters = Some(crate::rendering::water::program::parameters(
                    library, &bytes, &name,
                )?);
            }
        }
        Ok(Self {
            state,
            parameters: crate::rendering::water::program::parameters(library, &bytes, &name)?,
            ray_states,
            ray_parameters: ray_parameters.unwrap(),
            chromatic,
        })
    }
}

pub struct UnderwaterInputs<'a> {
    pub frame: &'a WaterFrame,
    pub settings: &'a crate::rendering::water::WaterSettings,
    pub water_level: f32,
    pub white_level: f32,
    pub camera_exposure: f32,
    pub adaptation: &'a Buffer,
    pub shadow: WaterShadowParameters,
    pub atlas: &'a TextureView,
    pub opaque: &'a Texture,
    pub alpha: &'a super::alpha_depth::AlphaDepthPass,
    pub hdr: &'a ViewTarget,
    pub images: &'a WaterImageSources,
    pub depth_programs: &'a super::tiled_lighting_gpu::TlfPipelines,
    pub extent: [u32; 2],
    pub rays: crate::rendering::water::rays::WaterRaySettings,
    pub ray_depth: &'a TextureView,
    pub ray_shadow: &'a TextureView,
    pub jitter: [f32; 2],
    pub temporal: bool,
    pub downsample: i32,
    pub scattering: bool,
}
pub struct UnderwaterScatteringPass {
    full: [u32; 2],
    compressed: bool,
    hdr_format: TextureFormat,
    pipeline: RenderPipeline,
    ray_pipelines: [RenderPipeline; 2],
    visibility_pipeline: RenderPipeline,
    ray_binding: Option<(usize, [TextureViewId; 4], BindGroup)>,
    visibility_binding: Option<(TextureViewId, BindGroup)>,
    visibility: Option<super::fog_targets::FogAttachment>,
    ray_active: Option<usize>,
    chromatic: bool,
    scattering_active: bool,
    parameters: Buffer,
    hdr: WaterHdr,
    depth: Option<(TextureId, DepthPyramid, TextureView)>,
    opaque: Option<(TextureId, TextureView)>,
    stencil: Option<(TextureId, TextureView)>,
    caustics: HashMap<usize, (Arc<crate::texture::InlineArrayChain>, LayeredImage)>,
    binding: Option<([TextureViewId; 6], BufferId, BindGroup)>,
    clamp: Sampler,
    wrap: Sampler,
    comparison: Sampler,
}
impl UnderwaterScatteringPass {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        full: [u32; 2],
        compressed: bool,
        hdr_format: TextureFormat,
        program: &UnderwaterProgram,
    ) -> Result<Self, String> {
        let source = concat!(
            include_str!("../../../assets/shaders/dh2_hdr.wgsl"),
            "\n",
            include_str!("../../../assets/shaders/dh2_point_sampling.wgsl"),
            "\n",
            include_str!("../../../assets/shaders/dh2_texture_array_sampling.wgsl"),
            "\n",
            include_str!("../../../assets/shaders/dh2_shadow.wgsl"),
            "\n",
            include_str!("../../../assets/shaders/dh2_water_refraction.wgsl"),
            "\n",
            include_str!("../../../assets/shaders/dh2_water_lighting.wgsl"),
            "\n",
            include_str!("../../../assets/shaders/dh2_underwater_scattering.wgsl")
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("underwater scattering"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let blend = crate::render_state::blend_state(&program.state).targets[0]
            .ok_or("underwater blend state absent")?;
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("underwater fog radiance"),
            layout: None,
            vertex: RawVertexState {
                module: &shader,
                entry_point: Some("underwater_vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &shader,
                entry_point: Some("underwater_fragment"),
                compilation_options: default(),
                targets: &[Some(ColorTargetState {
                    format: TextureFormat::Rgba16Float,
                    blend: super::pipeline_state::blend_target(blend)?,
                    write_mask: ColorWrites::from_bits_truncate(u32::from(blend.write_mask)),
                })],
            }),
            primitive: PrimitiveState {
                cull_mode: None,
                ..default()
            },
            depth_stencil: None,
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        let make = |entry: &'static str, format, blend, write_mask| {
            device.create_render_pipeline(&RawRenderPipelineDescriptor {
                label: Some(entry),
                layout: None,
                vertex: RawVertexState {
                    module: &shader,
                    entry_point: Some("underwater_vertex"),
                    compilation_options: default(),
                    buffers: &[],
                },
                fragment: Some(RawFragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: default(),
                    targets: &[Some(ColorTargetState {
                        format,
                        blend,
                        write_mask,
                    })],
                }),
                primitive: PrimitiveState {
                    cull_mode: None,
                    ..default()
                },
                depth_stencil: None,
                multisample: default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let ray_pipelines = program
            .ray_states
            .map(|state| {
                let target = crate::render_state::blend_state(&state).targets[0]
                    .ok_or("underwater ray blend absent")?;
                Ok(make(
                    "underwater_rays_fragment",
                    TextureFormat::Rgba16Float,
                    super::pipeline_state::blend_target(target)?,
                    ColorWrites::from_bits_truncate(u32::from(target.write_mask)),
                ))
            })
            .into_iter()
            .collect::<Result<Vec<_>, String>>()?
            .try_into()
            .map_err(|_| "underwater ray pipeline count")?;
        let visibility_pipeline = make(
            "underwater_visibility_fragment",
            TextureFormat::R8Unorm,
            Some(BlendState {
                color: BlendComponent {
                    src_factor: BlendFactor::One,
                    dst_factor: BlendFactor::One,
                    operation: BlendOperation::Add,
                },
                alpha: BlendComponent::REPLACE,
            }),
            ColorWrites::RED,
        );
        let sampler = |address| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                min_filter: FilterMode::Linear,
                mag_filter: FilterMode::Linear,
                ..default()
            })
        };
        Ok(Self {
            full,
            compressed,
            hdr_format,
            pipeline,
            ray_pipelines,
            visibility_pipeline,
            ray_binding: None,
            visibility_binding: None,
            visibility: None,
            ray_active: None,
            chromatic: program.chromatic,
            scattering_active: false,
            parameters: device.create_buffer(&BufferDescriptor {
                label: Some("underwater parameters"),
                size: 672,
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            hdr: WaterHdr::new(device, queue, full, compressed, hdr_format)?,
            depth: None,
            opaque: None,
            stencil: None,
            caustics: default(),
            binding: None,
            clamp: sampler(AddressMode::ClampToEdge),
            wrap: sampler(AddressMode::Repeat),
            comparison: super::shadow_pipeline::shadow_comparison_sampler(device),
        })
    }
    pub fn matches(&self, full: [u32; 2], compressed: bool, format: TextureFormat) -> bool {
        self.full == full && self.compressed == compressed && self.hdr_format == format
    }
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: UnderwaterInputs<'_>,
    ) -> Result<(), String> {
        let frame = input.frame;
        let settings = input.settings;
        self.scattering_active = input.scattering;
        self.ray_active = input
            .rays
            .enabled
            .then_some(usize::from(input.rays.shadowing));
        let visibility_extent = ((UVec2::from_array(self.full) + UVec2::splat(15))
            >> (input.downsample as u32 + 3))
            .max(UVec2::ONE);
        if self.ray_active.is_some()
            && self.visibility.as_ref().is_none_or(|v| {
                v.texture.width() != visibility_extent.x
                    || v.texture.height() != visibility_extent.y
            })
        {
            self.visibility = Some(super::fog_targets::FogAttachment::new(
                device,
                "underwater filter visibility",
                visibility_extent,
                TextureFormat::R8Unorm,
            ));
        }
        let matrices = frame.matrices.current;
        let projection = Mat4::from_cols_array_2d(&matrices.projection)
            .transpose()
            .inverse()
            .transpose()
            .to_cols_array_2d();
        let parallel = frame
            .parallel
            .ok_or("underwater parallel light inputs unavailable")?;
        let refracted = crate::rendering::water::refracted_light_direction(parallel.0);
        let water = WaterRefractionParameters {
            clock_caustics: [
                frame.local_time[0],
                settings.caustics.speed,
                settings.caustics.tiling,
                settings.caustics.chromatic_aberration,
            ],
            screen: [1., 1., 1. / self.full[0] as f32, 1. / self.full[1] as f32],
            unproject: [-projection[2][3], projection[3][2], projection[3][3], 0.],
            light_level: [
                refracted[0],
                refracted[1],
                settings.caustics.fade,
                input.water_level,
            ],
            extinction: [
                (1. - settings.colour[0]) * settings.extinction
                    / settings.underwater_visibility_boost,
                (1. - settings.colour[1]) * settings.extinction
                    / settings.underwater_visibility_boost,
                (1. - settings.colour[2]) * settings.extinction
                    / settings.underwater_visibility_boost,
                u32::from(self.compressed) as f32,
            ],
            hdr: [0., input.white_level, 0., 0.],
        };
        let alpha = &input.alpha.full;
        if self.depth.as_ref().is_none_or(|v| v.0 != alpha.id()) {
            let pyramid = DepthPyramid::new(device, input.depth_programs, alpha)?;
            let view = pyramid.texture().create_view(&TextureViewDescriptor {
                mip_level_count: Some(1),
                ..default()
            });
            self.depth = Some((alpha.id(), pyramid, view));
        }
        let (_, pyramid, reduced) = self.depth.as_ref().unwrap();
        pyramid.record(queue, encoder, input.depth_programs, self.full, projection)?;
        let mut bytes = super::water_refraction::uniform_bytes(&water, &input.shadow).to_vec();
        bytes.extend(
            Mat4::from_cols_array_2d(&matrices.view_projection)
                .transpose()
                .inverse()
                .to_cols_array()
                .into_iter()
                .flat_map(f32::to_le_bytes),
        );
        for vector in [
            [
                matrices.position[0],
                matrices.position[1],
                matrices.position[2],
                frame.near_far[1],
            ],
            [settings.extinction, 0., 0., settings.inscattering],
            [
                parallel.1[0],
                parallel.1[1],
                parallel.1[2],
                settings.underwater_visibility_boost,
            ],
            [
                input.extent[0] as f32,
                input.extent[1] as f32,
                self.full[0] as f32 * 0.5 / pyramid.texture().width() as f32,
                self.full[1] as f32 * 0.5 / pyramid.texture().height() as f32,
            ],
            [
                input.rays.intensity,
                u32::from(input.rays.shadowing) as f32,
                u32::from(self.chromatic) as f32,
                u32::from(input.temporal) as f32,
            ],
            [
                input.jitter[0],
                input.jitter[1],
                visibility_extent.x as f32,
                visibility_extent.y as f32,
            ],
        ] {
            bytes.extend(vector.into_iter().flat_map(f32::to_le_bytes));
        }
        queue.write_buffer(&self.parameters, 0, &bytes);
        if self
            .opaque
            .as_ref()
            .is_none_or(|v| v.0 != input.opaque.id())
        {
            self.opaque = Some((
                input.opaque.id(),
                input.opaque.create_view(&TextureViewDescriptor {
                    aspect: TextureAspect::DepthOnly,
                    ..default()
                }),
            ));
        }
        if self
            .stencil
            .as_ref()
            .is_none_or(|v| v.0 != input.alpha.reduced.id())
        {
            self.stencil = Some((
                input.alpha.reduced.id(),
                input.alpha.reduced.create_view(&TextureViewDescriptor {
                    aspect: TextureAspect::StencilOnly,
                    ..default()
                }),
            ));
        }
        let chain = input
            .images
            .caustics
            .get(&settings.caustics.image)
            .ok_or("underwater caustics not prepared")?;
        let id = Arc::as_ptr(chain) as usize;
        if !self.caustics.contains_key(&id) {
            self.caustics.insert(
                id,
                (chain.clone(), LayeredImage::array(device, queue, chain)?),
            );
        }
        let snapshot = self.hdr.prepare(
            device,
            queue,
            encoder,
            input.hdr.main_texture_view(),
            self.full,
            input.camera_exposure,
            input.white_level,
            input.adaptation,
        )?;
        let views = [
            snapshot.source(),
            &self.opaque.as_ref().unwrap().1,
            reduced,
            self.caustics[&id].1.view(),
            input.atlas,
            &self.stencil.as_ref().unwrap().1,
        ];
        let ids = views.map(TextureView::id);
        if self
            .binding
            .as_ref()
            .is_none_or(|v| v.0 != ids || v.1 != input.adaptation.id())
        {
            let mut entries = vec![BindGroupEntry {
                binding: 0,
                resource: self.parameters.as_entire_binding(),
            }];
            entries.extend(views.into_iter().enumerate().map(|(i, v)| BindGroupEntry {
                binding: i as u32 + 1,
                resource: BindingResource::TextureView(v),
            }));
            entries.extend(
                [&self.clamp, &self.wrap, &self.comparison]
                    .into_iter()
                    .enumerate()
                    .map(|(i, v)| BindGroupEntry {
                        binding: i as u32 + 7,
                        resource: BindingResource::Sampler(v),
                    }),
            );
            entries.push(BindGroupEntry {
                binding: 10,
                resource: input.adaptation.as_entire_binding(),
            });
            self.binding = Some((
                ids,
                input.adaptation.id(),
                device.create_bind_group(
                    Some("underwater inputs"),
                    &self.pipeline.get_bind_group_layout(0).into(),
                    &entries,
                ),
            ));
        }
        // This consumer only reads the pre-water snapshot. Its colour mirror
        // remains untouched; fog composition, rather than a host HDR copy, owns
        // the underwater result.
        drop(snapshot);
        if let Some(index) = self.ray_active {
            let caustics = self.caustics[&id].1.view();
            let stencil = &self.stencil.as_ref().unwrap().1;
            let ids = [
                caustics.id(),
                stencil.id(),
                input.ray_depth.id(),
                input.ray_shadow.id(),
            ];
            if self
                .ray_binding
                .as_ref()
                .is_none_or(|v| v.0 != index || v.1 != ids)
            {
                let entries = [
                    BindGroupEntry {
                        binding: 0,
                        resource: self.parameters.as_entire_binding(),
                    },
                    BindGroupEntry {
                        binding: 4,
                        resource: BindingResource::TextureView(caustics),
                    },
                    BindGroupEntry {
                        binding: 6,
                        resource: BindingResource::TextureView(stencil),
                    },
                    BindGroupEntry {
                        binding: 7,
                        resource: BindingResource::Sampler(&self.clamp),
                    },
                    BindGroupEntry {
                        binding: 8,
                        resource: BindingResource::Sampler(&self.wrap),
                    },
                    BindGroupEntry {
                        binding: 11,
                        resource: BindingResource::TextureView(input.ray_depth),
                    },
                    BindGroupEntry {
                        binding: 12,
                        resource: BindingResource::TextureView(input.ray_shadow),
                    },
                ];
                self.ray_binding = Some((
                    index,
                    ids,
                    device.create_bind_group(
                        Some("underwater rays inputs"),
                        &self.ray_pipelines[index].get_bind_group_layout(0).into(),
                        &entries,
                    ),
                ));
            }
            if self
                .visibility_binding
                .as_ref()
                .is_none_or(|v| v.0 != stencil.id())
            {
                self.visibility_binding = Some((
                    stencil.id(),
                    device.create_bind_group(
                        Some("underwater visibility inputs"),
                        &self.visibility_pipeline.get_bind_group_layout(0).into(),
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: self.parameters.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 6,
                                resource: BindingResource::TextureView(stencil),
                            },
                        ],
                    ),
                ));
            }
        }
        Ok(())
    }
    pub fn record(&self, encoder: &mut CommandEncoder, target: &TextureView) {
        self.record_with_visibility(encoder, target, None);
    }
    pub fn record_with_visibility(
        &self,
        encoder: &mut CommandEncoder,
        target: &TextureView,
        visibility: Option<&TextureView>,
    ) {
        let Some((_, _, binding)) = &self.binding else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("underwater fog scattering"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })],
            ..default()
        });
        if self.scattering_active {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, binding, &[]);
            pass.draw(0..3, 0..1);
        }
        drop(pass);
        if let Some(index) = self.ray_active {
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("underwater rays"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(&self.ray_pipelines[index]);
            pass.set_bind_group(0, &self.ray_binding.as_ref().unwrap().2, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("underwater filter visibility"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: visibility.unwrap_or(&self.visibility.as_ref().unwrap().view),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: if visibility.is_some() {
                            LoadOp::Load
                        } else {
                            LoadOp::Clear(LinearRgba::NONE.into())
                        },
                        store: StoreOp::Store,
                    },
                })],
                ..default()
            });
            pass.set_pipeline(&self.visibility_pipeline);
            pass.set_bind_group(0, &self.visibility_binding.as_ref().unwrap().1, &[]);
            pass.draw(0..3, 0..1);
        }
    }
    pub fn visibility(&self) -> Option<&TextureView> {
        self.ray_active
            .and_then(|_| self.visibility.as_ref().map(|v| &v.view))
    }
    pub fn shadowed_rays(&self) -> bool {
        self.ray_active == Some(1)
    }
}
