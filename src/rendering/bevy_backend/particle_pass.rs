//! Instanced particle colour submission. The scene owner supplies effective
//! draw state, clipping, selected volume fog and current view resources.
use super::fog_objects::FogObjectParameters;
use crate::rendering::particles::{
    geometry::SpriteGrid, material::ParticleMaterial, SpriteInstance,
};
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::ShaderBuffer,
    },
};
use std::{collections::HashMap, sync::Arc};
mod transparency;

#[derive(Clone, Copy, ShaderType)]
struct TransparencyParameters {
    weights: Vec4,
    parameters: Vec4,
    mode: UVec4,
}

#[derive(Clone, Copy, ShaderType)]
struct ViewParameters {
    clip_from_game_world: Mat4,
    camera_time: Vec4,
    inverse_z: Vec4,
    exposure_extent: Vec4,
}
#[derive(Clone, Copy, ShaderType)]
struct FogViewParameters {
    world_position: Vec3,
    exposure: f32,
}
#[derive(Clone, Copy, ShaderType)]
struct TextureLayer {
    tiling_pan: Vec4,
    strength: Vec4,
    rotation_seed: Vec4,
}
#[derive(Clone, Copy, ShaderType)]
struct MaterialParameters {
    layers: [TextureLayer; 3],
    colour_adjust: Vec4,
    depth_fade_lookup: Vec4,
    radial_exposure_fog: Vec4,
    distort_options: UVec4,
    colour_options: UVec4,
    fade_options: UVec4,
}
impl MaterialParameters {
    fn resolve(source: &ParticleMaterial) -> Result<Self, String> {
        for name in [
            "fx/opt/use_lighting",
            "fx/opt/fresnel",
            "fx/opt/use_macroUV",
            "fx/opt/use_flipbook",
            "fx/opt/opacity_scroll_mask",
        ] {
            if source.option(name)? != 0 {
                return Err(format!("particle {name} draw producer is not connected"));
            }
        }
        let option = |name: &str| source.option(name).map(|v| v as u32);
        let scalar = |name: &str| source.uniform(name).map(|v| v[0]);
        let mut layers = [TextureLayer {
            tiling_pan: Vec4::ZERO,
            strength: Vec4::ZERO,
            rotation_seed: Vec4::ZERO,
        }; 3];
        for (layer, prefix) in layers.iter_mut().zip(["disto1", "disto2", "opacity"]) {
            let tile = source.uniform(&format!("{prefix}_Tilling"))?;
            let pan = source.uniform(&format!("{prefix}_PanningSpeed"))?;
            layer.tiling_pan = Vec4::new(tile[0], tile[1], pan[0], pan[1]);
            layer.rotation_seed = Vec4::new(
                scalar(&format!("{prefix}_RotationSpeed"))?,
                scalar("material_seed")?,
                0.,
                0.,
            );
            if prefix != "opacity" {
                layer.strength = Vec4::from_array(source.uniform(&format!("{prefix}_Strength"))?);
            }
        }
        let soft = option("fx/opt/use_softcontact")?;
        if soft > 2 {
            return Err("spherical particle soft contact is not connected".into());
        }
        Ok(Self {
            layers,
            colour_adjust: Vec4::from_array(source.uniform("color_Adjust")?),
            depth_fade_lookup: Vec4::new(
                scalar("depth_Threshold")?,
                scalar("particleDistanceFadeIn")?,
                scalar("particleDistanceFadeOut")?,
                scalar("lookup_adjust")?,
            ),
            radial_exposure_fog: Vec4::new(
                scalar("radial_RampPow")?,
                scalar("fx/auto_exposure_amount")?,
                scalar("particleFogScatterPower")?,
                0.,
            ),
            distort_options: UVec4::new(
                option("fx/opt/uv_disto1")?,
                option("fx/opt/uv_disto2")?,
                option("fx/opt/uv_disto1_uniform")?,
                option("fx/opt/uv_disto2_uniform")?,
            ),
            colour_options: UVec4::new(
                option("fx/opt/uv_Disto2xDisto1")?,
                option("fx/opt/use_diffuse")?,
                option("fx/opt/use_lookup")?,
                option("fx/opt/radial_ramp")?,
            ),
            fade_options: UVec4::new(
                option("fx/opt/use_disto_has_mask")?,
                option("fx/opt/view_dependent_fade")?,
                soft,
                option("fx/opt/no_alpha_lookup_modulate")?,
            ),
        })
    }
}

pub struct ParticleDraw {
    pub material: Arc<ParticleMaterial>,
    /// The selected colour-pass state, after the owning cache's overrides.
    pub state: crate::render_state::RenderState,
    pub stencil_reference: u32,
    pub grid_level: u32,
    pub instances: Vec<SpriteInstance>,
    pub clip_plane: Vec4,
    pub clip_fade_distance: f32,
    pub fog: FogObjectParameters,
    pub transparency: crate::rendering::particles::transparency::TransparencyParameters,
}
pub struct ParticleFogInputs<'a> {
    pub parameters: &'a Buffer,
    pub world: &'a TextureView,
    pub canvas: &'a TextureView,
    pub curve: &'a TextureView,
    pub depth: &'a TextureView,
}
pub struct ParticleViewInputs<'a> {
    pub matrices: &'a crate::rendering::view_history::ViewMatrices,
    pub local_time: f32,
    pub camera_exposure: f32,
    pub extent: [u32; 2],
    pub inverse_z: [f32; 3],
    pub adaptation: &'a Buffer,
    pub colour: &'a TextureView,
    pub depth: &'a TextureView,
    pub fog: ParticleFogInputs<'a>,
    pub compressed_hdr: bool,
    pub white_level: f32,
    pub resolve_state: crate::render_state::RenderState,
}
struct GridGpu {
    vertices: Buffer,
    indices: Buffer,
    count: u32,
}
struct MaterialGpu {
    _source: Arc<ParticleMaterial>,
    _parameters: UniformBuffer<MaterialParameters>,
    _images: Vec<Texture>,
    binding: BindGroup,
    pipeline: RenderPipeline,
}

/// Allocations are bounded by the admitted declaration's capacity and sprite
/// count. Construct a new owner when changing level or target formats.
pub struct ParticlePass {
    view: UniformBuffer<ViewParameters>,
    fog_view: UniformBuffer<FogViewParameters>,
    instances: Buffer,
    objects: Buffer,
    transparency_parameters: Buffer,
    transparency: Option<transparency::TransparencyPass>,
    max_instances: usize,
    max_draws: usize,
    grids: Vec<GridGpu>,
    view_layout: BindGroupLayout,
    material_layout: BindGroupLayout,
    fog_layout: BindGroupLayout,
    pipeline_layout: PipelineLayout,
    shader: ShaderModule,
    wrap: Sampler,
    clamp: Sampler,
    mirror: Sampler,
    bilinear: Sampler,
    materials: HashMap<(String, crate::render_state::RenderState), MaterialGpu>,
    view_bindings: Option<(BufferId, Vec<BindGroup>)>,
    fog_binding: Option<((BufferId, [TextureViewId; 4]), BindGroup)>,
    colour_format: TextureFormat,
    depth_format: TextureFormat,
    instance_bytes: Vec<u8>,
}
fn buffer_entry(binding: u32, uniform: bool) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT,
        ty: BindingType::Buffer {
            ty: if uniform {
                BufferBindingType::Uniform
            } else {
                BufferBindingType::Storage { read_only: true }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
fn texture_entry(
    binding: u32,
    dimension: TextureViewDimension,
    depth: bool,
) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT,
        ty: BindingType::Texture {
            sample_type: if depth {
                TextureSampleType::Depth
            } else {
                TextureSampleType::Float { filterable: true }
            },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}
fn sampler_entry(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::VERTEX_FRAGMENT,
        ty: BindingType::Sampler(SamplerBindingType::Filtering),
        count: None,
    }
}
impl ParticlePass {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        max_instances: usize,
        max_draws: usize,
        colour_format: TextureFormat,
        depth_format: TextureFormat,
    ) -> Result<Self, String> {
        if max_instances == 0 || max_draws == 0 {
            return Err("particle GPU capacity must be nonzero".into());
        }
        let view_layout = device.create_bind_group_layout(
            Some("particle view"),
            &[
                buffer_entry(0, true),
                buffer_entry(1, false),
                buffer_entry(2, false),
                buffer_entry(3, false),
                buffer_entry(4, true),
                buffer_entry(5, false),
            ],
        );
        let material_layout = device.create_bind_group_layout(
            Some("particle material"),
            &std::iter::once(buffer_entry(0, true))
                .chain((1..=5).map(|b| texture_entry(b, TextureViewDimension::D2, false)))
                .chain((6..=8).map(sampler_entry))
                .collect::<Vec<_>>(),
        );
        let fog_layout = device.create_bind_group_layout(
            Some("particle shared fog"),
            &[
                buffer_entry(161, false),
                texture_entry(162, TextureViewDimension::D2, false),
                texture_entry(163, TextureViewDimension::D2, false),
                texture_entry(164, TextureViewDimension::D2Array, false),
                texture_entry(165, TextureViewDimension::D2, true),
                sampler_entry(166),
                buffer_entry(167, false),
            ],
        );
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("particle colour"),
            bind_group_layouts: &[
                Some(&view_layout),
                Some(&material_layout),
                Some(&fog_layout),
            ],
            immediate_size: 0,
        });
        let fog = include_str!("../../../assets/shaders/dh2_fog_material.wgsl")
            .lines()
            .filter(|line| !line.trim_start().starts_with("#import"))
            .collect::<Vec<_>>()
            .join("\n")
            .replace("@group(#{MATERIAL_BIND_GROUP})", "@group(2)");
        let source = format!(
            "{}\n{}\n{}\n{fog}",
            include_str!("../../../assets/shaders/dh2_fog_math.wgsl"),
            include_str!("../../../assets/shaders/dh2_fog_alpha.wgsl"),
            include_str!("../../../assets/shaders/dh2_particles.wgsl")
        );
        let shader = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("particle unlit colour"),
            source: ShaderSource::Wgsl(source.into()),
        });
        let storage = |label, size| {
            device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let instances = storage("particle instances", (max_instances * 96) as u64);
        let objects = storage(
            "particle volume fog",
            (max_draws * FogObjectParameters::SIZE) as u64,
        );
        let transparency_parameters =
            storage("particle transparency parameters", (max_draws * 48) as u64);
        let mut grids = Vec::new();
        for level in 0..6 {
            let grid = SpriteGrid::new(level);
            let vertices = grid
                .vertices
                .iter()
                .flat_map(|v| {
                    v.position
                        .into_iter()
                        .chain([0.])
                        .chain(v.uv)
                        .chain([0., 0.])
                })
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>();
            let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("particle grid vertices"),
                contents: &vertices,
                usage: BufferUsages::STORAGE,
            });
            let bytes = grid
                .indices
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>();
            let indices = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("particle grid indices"),
                contents: &bytes,
                usage: BufferUsages::INDEX,
            });
            grids.push(GridGpu {
                vertices,
                indices,
                count: grid.indices.len() as u32,
            });
        }
        let sampler = |mode, mipmap| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: mipmap,
                lod_max_clamp: 13.,
                ..default()
            })
        };
        let mut view = UniformBuffer::from(ViewParameters {
            clip_from_game_world: Mat4::IDENTITY,
            camera_time: Vec4::ZERO,
            inverse_z: Vec4::ZERO,
            exposure_extent: Vec4::ONE,
        });
        view.write_buffer(device, queue);
        let mut fog_view = UniformBuffer::from(FogViewParameters {
            world_position: Vec3::ZERO,
            exposure: 1.,
        });
        fog_view.write_buffer(device, queue);
        Ok(Self {
            view,
            fog_view,
            instances,
            objects,
            transparency_parameters,
            transparency: None,
            max_instances,
            max_draws,
            grids,
            view_layout,
            material_layout,
            fog_layout,
            pipeline_layout,
            shader,
            wrap: sampler(AddressMode::Repeat, MipmapFilterMode::Linear),
            clamp: sampler(AddressMode::ClampToEdge, MipmapFilterMode::Nearest),
            mirror: sampler(AddressMode::MirrorRepeat, MipmapFilterMode::Nearest),
            bilinear: sampler(AddressMode::Repeat, MipmapFilterMode::Nearest),
            materials: HashMap::new(),
            view_bindings: None,
            fog_binding: None,
            colour_format,
            depth_format,
            instance_bytes: Vec::with_capacity(max_instances * 96),
        })
    }

    fn material(
        &self,
        device: &RenderDevice,
        queue: &RenderQueue,
        source: Arc<ParticleMaterial>,
        state: crate::render_state::RenderState,
        accumulation: bool,
    ) -> Result<MaterialGpu, String> {
        let mut parameters = UniformBuffer::from(MaterialParameters::resolve(&source)?);
        parameters.write_buffer(device, queue);
        let mut images = Vec::new();
        let mut views = HashMap::new();
        for (name, image) in &source.images {
            let texture = device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some(&source.name),
                    size: Extent3d {
                        width: image.info.width,
                        height: image.info.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: image.info.mip_count,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: super::image_formats::gpu_image_format(image.info.format),
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                &image.payload,
            );
            views.insert(name.as_str(), texture.create_view(&default()));
            images.push(texture);
        }
        let opacity = views
            .get("fx/opacity")
            .ok_or("particle opacity image absent")?;
        let surface_sampler = if source.option("fx/opt/use_mirror_sampler")? != 0 {
            &self.mirror
        } else if source.option("fx/opt/use_clamp_sampler")? != 0 {
            &self.clamp
        } else if source.option("fx/opt/use_bilinear_sampler")? != 0 {
            &self.bilinear
        } else {
            &self.wrap
        };
        let binding = device.create_bind_group(
            Some("particle material"),
            &self.material_layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: parameters.binding().ok_or("particle uniforms absent")?,
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(opacity),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(
                        views.get("fx/disto1").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::TextureView(
                        views.get("fx/disto2").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 4,
                    resource: BindingResource::TextureView(
                        views.get("fx/diffuse").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 5,
                    resource: BindingResource::TextureView(
                        views.get("fx/lookup").unwrap_or(opacity),
                    ),
                },
                BindGroupEntry {
                    binding: 6,
                    resource: BindingResource::Sampler(&self.wrap),
                },
                BindGroupEntry {
                    binding: 7,
                    resource: BindingResource::Sampler(surface_sampler),
                },
                BindGroupEntry {
                    binding: 8,
                    resource: BindingResource::Sampler(&self.clamp),
                },
            ],
        );
        let blends = crate::render_state::blend_state(&state);
        let formats = [
            if accumulation {
                TextureFormat::Rgba16Float
            } else {
                self.colour_format
            },
            TextureFormat::Rg16Float,
        ];
        let targets = (0..if accumulation { 2 } else { 1 })
            .map(|index| {
                let blend = blends.targets[index].ok_or("particle colour target absent")?;
                Ok(Some(ColorTargetState {
                    format: formats[index],
                    blend: super::pipeline_state::blend_target(blend)?,
                    write_mask: ColorWrites::from_bits_truncate(blend.write_mask as u32),
                }))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let depth = crate::render_state::depth_stencil_state(&state);
        if depth.depth_write {
            return Err("particle alpha colour state must disable depth writes".into());
        }
        if depth.stencil_enabled && !self.depth_format.has_stencil_aspect() {
            return Err(
                "particle alpha colour state requires the gameplay stencil attachment".into(),
            );
        }
        let (front_face, cull_mode) = super::shadow_pipeline::shadow_mesh_raster(&state, false);
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("particle sprite colour"),
            layout: Some(&self.pipeline_layout),
            vertex: RawVertexState {
                module: &self.shader,
                entry_point: Some("particle_vertex"),
                compilation_options: default(),
                buffers: &[],
            },
            fragment: Some(RawFragmentState {
                module: &self.shader,
                entry_point: Some(if accumulation {
                    "particle_accumulation_fragment"
                } else {
                    "particle_fragment"
                }),
                compilation_options: default(),
                targets: &targets,
            }),
            primitive: PrimitiveState {
                front_face,
                cull_mode,
                ..default()
            },
            depth_stencil: Some(super::pipeline_state::depth_stencil_gpu(
                &state,
                self.depth_format,
            )?),
            multisample: default(),
            multiview_mask: None,
            cache: None,
        });
        Ok(MaterialGpu {
            _source: source,
            _parameters: parameters,
            _images: images,
            binding,
            pipeline,
        })
    }

    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: ParticleViewInputs<'_>,
        draws: &[ParticleDraw],
    ) -> Result<(), String> {
        if draws.is_empty() {
            return Ok(());
        }
        let accumulation = draws[0].transparency.mode != 0;
        if draws
            .iter()
            .any(|draw| (draw.transparency.mode != 0) != accumulation)
        {
            return Err("particle transparency mode changed within a frame".into());
        }
        if accumulation
            && self
                .transparency
                .as_ref()
                .is_none_or(|p| !p.matches(input.extent, input.compressed_hdr, input.resolve_state))
        {
            self.transparency = Some(transparency::TransparencyPass::new(
                device,
                queue,
                input.extent,
                input.compressed_hdr,
                self.colour_format,
                self.depth_format,
                input.resolve_state,
            )?);
        }
        let count = draws.iter().map(|d| d.instances.len()).sum::<usize>();
        if count > self.max_instances || draws.len() > self.max_draws {
            return Err("particle draw exceeds admitted declaration capacity".into());
        }
        for draw in draws {
            let key = (draw.material.name.clone(), draw.state);
            if !self.materials.contains_key(&key) {
                self.materials.insert(
                    key,
                    self.material(
                        device,
                        queue,
                        draw.material.clone(),
                        draw.state,
                        accumulation,
                    )?,
                );
            }
        }
        let position = input.matrices.position;
        self.view.set(ViewParameters {
            clip_from_game_world: Mat4::from_cols_array_2d(&input.matrices.view_projection)
                .transpose(),
            camera_time: Vec3::from_array(position).extend(input.local_time),
            inverse_z: Vec3::from_array(input.inverse_z).extend(0.),
            exposure_extent: Vec4::new(
                139.26 * input.camera_exposure,
                input.camera_exposure,
                input.extent[0] as f32,
                input.extent[1] as f32,
            ),
        });
        self.view.write_buffer(device, queue);
        self.fog_view.set(FogViewParameters {
            world_position: Vec3::new(position[0], position[2], -position[1]),
            exposure: input.camera_exposure,
        });
        self.fog_view.write_buffer(device, queue);
        self.instance_bytes.clear();
        for (tag, draw) in draws.iter().enumerate() {
            for instance in &draw.instances {
                for v in [
                    Vec3::from_array(instance.position).extend(0.),
                    Vec3::from_array(instance.axes[1]).extend(0.),
                    Vec3::from_array(instance.axes[2]).extend(0.),
                ] {
                    self.instance_bytes
                        .extend(v.to_array().into_iter().flat_map(f32::to_le_bytes));
                }
                self.instance_bytes.extend(
                    [
                        instance.parameters.colour,
                        instance.parameters.flipbook_seed_uv,
                        instance.parameters.size,
                        tag as u32,
                    ]
                    .into_iter()
                    .flat_map(u32::to_le_bytes),
                );
                self.instance_bytes.extend(
                    draw.clip_plane
                        .to_array()
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
                self.instance_bytes.extend(
                    [draw.clip_fade_distance, 0., 0., 0.]
                        .into_iter()
                        .flat_map(f32::to_le_bytes),
                );
            }
        }
        if count == 0 {
            return Ok(());
        }
        queue.write_buffer(&self.instances, 0, &self.instance_bytes);
        let transparency = ShaderBuffer::from(
            draws
                .iter()
                .map(|draw| TransparencyParameters {
                    weights: Vec4::from_array(draw.transparency.weights),
                    parameters: Vec4::from_array(draw.transparency.parameters),
                    mode: UVec4::new(draw.transparency.mode as u32, 0, 0, 0),
                })
                .collect::<Vec<_>>(),
        );
        queue.write_buffer(
            &self.transparency_parameters,
            0,
            transparency
                .data
                .as_ref()
                .ok_or("particle transparency records absent")?,
        );
        let objects = ShaderBuffer::from(draws.iter().map(|d| d.fog).collect::<Vec<_>>());
        queue.write_buffer(
            &self.objects,
            0,
            objects.data.as_ref().ok_or("particle fog records absent")?,
        );
        if self
            .view_bindings
            .as_ref()
            .is_none_or(|v| v.0 != input.adaptation.id())
        {
            let bindings = self
                .grids
                .iter()
                .map(|grid| {
                    device.create_bind_group(
                        Some("particle view"),
                        &self.view_layout,
                        &[
                            BindGroupEntry {
                                binding: 0,
                                resource: self.view.binding().unwrap(),
                            },
                            BindGroupEntry {
                                binding: 1,
                                resource: self.instances.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 2,
                                resource: grid.vertices.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 3,
                                resource: input.adaptation.as_entire_binding(),
                            },
                            BindGroupEntry {
                                binding: 4,
                                resource: self.fog_view.binding().unwrap(),
                            },
                            BindGroupEntry {
                                binding: 5,
                                resource: self.transparency_parameters.as_entire_binding(),
                            },
                        ],
                    )
                })
                .collect();
            self.view_bindings = Some((input.adaptation.id(), bindings));
        }
        let fog_key = (
            input.fog.parameters.id(),
            [
                input.fog.world.id(),
                input.fog.canvas.id(),
                input.fog.curve.id(),
                input.fog.depth.id(),
            ],
        );
        if self.fog_binding.as_ref().is_none_or(|v| v.0 != fog_key) {
            self.fog_binding = Some((
                fog_key,
                device.create_bind_group(
                    Some("particle shared fog"),
                    &self.fog_layout,
                    &[
                        BindGroupEntry {
                            binding: 161,
                            resource: input.fog.parameters.as_entire_binding(),
                        },
                        BindGroupEntry {
                            binding: 162,
                            resource: BindingResource::TextureView(input.fog.world),
                        },
                        BindGroupEntry {
                            binding: 163,
                            resource: BindingResource::TextureView(input.fog.canvas),
                        },
                        BindGroupEntry {
                            binding: 164,
                            resource: BindingResource::TextureView(input.fog.curve),
                        },
                        BindGroupEntry {
                            binding: 165,
                            resource: BindingResource::TextureView(input.fog.depth),
                        },
                        BindGroupEntry {
                            binding: 166,
                            resource: BindingResource::Sampler(&self.wrap),
                        },
                        BindGroupEntry {
                            binding: 167,
                            resource: self.objects.as_entire_binding(),
                        },
                    ],
                ),
            ));
        }
        let colour_attachments = if accumulation {
            let targets = self.transparency.as_ref().unwrap();
            vec![
                Some(RenderPassColorAttachment {
                    view: &targets.accumulation_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Default::default()),
                        store: StoreOp::Store,
                    },
                }),
                Some(RenderPassColorAttachment {
                    view: &targets.opacity_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(bevy::color::LinearRgba::new(1., 0., 0., 1.).into()),
                        store: StoreOp::Store,
                    },
                }),
            ]
        } else {
            vec![Some(RenderPassColorAttachment {
                view: input.colour,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Load,
                    store: StoreOp::Store,
                },
            })]
        };
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("particle colour"),
            color_attachments: &colour_attachments,
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: input.depth,
                depth_ops: None,
                stencil_ops: self
                    .depth_format
                    .has_stencil_aspect()
                    .then_some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(2, &self.fog_binding.as_ref().unwrap().1, &[]);
        let mut first = 0u32;
        for draw in draws {
            let level = draw.grid_level.min(5) as usize;
            let material = &self.materials[&(draw.material.name.clone(), draw.state)];
            pass.set_pipeline(&material.pipeline);
            pass.set_stencil_reference(draw.stencil_reference);
            pass.set_bind_group(0, &self.view_bindings.as_ref().unwrap().1[level], &[]);
            pass.set_bind_group(1, &material.binding, &[]);
            pass.set_index_buffer(*self.grids[level].indices.slice(..), IndexFormat::Uint16);
            let count = draw.instances.len() as u32;
            pass.draw_indexed(0..self.grids[level].count, 0, first..first + count);
            first += count;
        }
        drop(pass);
        if accumulation {
            self.transparency
                .as_mut()
                .unwrap()
                .record_resolve(device, queue, encoder, &input)?;
        }
        Ok(())
    }
}
