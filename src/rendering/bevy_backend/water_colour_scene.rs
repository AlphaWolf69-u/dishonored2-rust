//! Placed WORLD water colour between fog production and HDR composition.
//! Uses retained raw meshes, selected material ABIs and current view resources.
use super::{
    coarse_depth::DepthPyramid, layered_images::LayeredImage, noise_textures::NoiseTextures,
    water_hdr::WaterHdr, water_scene::*, water_world_colour::*,
};
use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    prelude::*,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        render_phase::TrackedRenderPass,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
        storage::GpuShaderBuffer,
    },
};
use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
    sync::Arc,
};

pub struct WaterColourResources<'a> {
    pub device: &'a RenderDevice,
    pub queue: &'a RenderQueue,
    pub materials: &'a WaterMaterials,
    pub world: &'a WaterWorldMeshes,
    pub lighting: &'a WaterLighting,
    pub images: &'a WaterImageSources,
    pub image_assets: &'a RenderAssets<bevy::render::texture::GpuImage>,
    pub meshes: &'a RenderAssets<RenderMesh>,
    pub buffers: &'a RenderAssets<GpuShaderBuffer>,
    pub allocator: &'a MeshAllocator,
    pub depth_programs: &'a super::tiled_lighting_gpu::TlfPipelines,
}
pub struct WaterColourView<'a> {
    pub frame: &'a WaterFrame,
    pub white_level: f32,
    pub camera_exposure: f32,
    pub adaptation: &'a Buffer,
    pub shadow: super::water_refraction::WaterShadowParameters,
    pub atlas: &'a TextureView,
    pub fog: &'a TextureView,
    pub hdr: &'a TextureView,
    pub opaque: &'a Texture,
    pub water_depth: &'a TextureView,
    pub pyramid: Option<&'a DepthPyramid>,
    pub shared_pyramid_expected: bool,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    layout: MeshVertexBufferLayoutRef,
    state: crate::render_state::RenderState,
    reversed: bool,
    class: crate::rendering::water::program::WorldWaterColourClass,
}
struct Program {
    colour: WorldProbeColourProgram,
    parameters: Option<([BufferId; 2], BindGroup, BindGroup)>,
    inputs: Option<([BufferId; 5], BindGroup)>,
    probes: bool,
}
struct Draw {
    program: usize,
    material: Arc<str>,
    vertex: Buffer,
    range: Range<u32>,
    indices: Option<(Buffer, IndexFormat, Range<u32>, i32)>,
    vertex_offset: u32,
    fragment_offset: u32,
    sky: usize,
    caustics: usize,
}
struct MaterialBinding {
    images: [TextureViewId; 10],
    exposure: BufferId,
    binding: BindGroup,
}
pub struct WaterColourScene {
    extent: [u32; 2],
    compressed: bool,
    host_format: TextureFormat,
    depth_format: TextureFormat,
    hdr: WaterHdr,
    noise: NoiseTextures,
    programs: Vec<Program>,
    keys: HashMap<Key, usize>,
    draws: Vec<Draw>,
    vertex: Option<Buffer>,
    fragment: Option<Buffer>,
    remap: Option<Buffer>,
    skies: HashMap<
        usize,
        (
            Arc<crate::texture::InlineCubeChain>,
            LayeredImage,
            TextureView,
        ),
    >,
    caustics: HashMap<usize, (Arc<crate::texture::InlineArrayChain>, LayeredImage)>,
    bindings: HashMap<(usize, Arc<str>), MaterialBinding>,
    opaque: Option<(TextureId, TextureView)>,
    pyramid: Option<(TextureId, DepthPyramid, TextureView)>,
    shared_pyramid: Option<(TextureId, TextureView)>,
    linear_clamp: Sampler,
    trilinear_clamp: Sampler,
    trilinear_wrap: Sampler,
    comparison: Sampler,
}
impl WaterColourScene {
    pub(super) fn draw_count(&self) -> usize {
        self.draws.len()
    }

    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        extent: [u32; 2],
        compressed: bool,
        host_format: TextureFormat,
        depth_format: TextureFormat,
    ) -> Result<Self, String> {
        let sampler = |mipmap_filter, address_mode| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address_mode,
                address_mode_v: address_mode,
                address_mode_w: address_mode,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter,
                lod_max_clamp: 13.,
                ..default()
            })
        };
        Ok(Self {
            extent,
            compressed,
            host_format,
            depth_format,
            hdr: WaterHdr::new(device, queue, extent, compressed, host_format)?,
            noise: NoiseTextures::new(device, queue),
            programs: default(),
            keys: default(),
            draws: default(),
            vertex: None,
            fragment: None,
            remap: None,
            skies: default(),
            caustics: default(),
            bindings: default(),
            opaque: None,
            pyramid: None,
            shared_pyramid: None,
            // Consumed sampler declarations: bilinearClamp, iblSampler and
            // trilinearSampler. The latter is also the source sky sampler.
            linear_clamp: sampler(MipmapFilterMode::Nearest, AddressMode::ClampToEdge),
            trilinear_clamp: sampler(MipmapFilterMode::Linear, AddressMode::ClampToEdge),
            trilinear_wrap: sampler(MipmapFilterMode::Linear, AddressMode::Repeat),
            comparison: super::shadow_pipeline::shadow_comparison_sampler(device),
        })
    }
    pub fn matches(
        &self,
        extent: [u32; 2],
        compressed: bool,
        host: TextureFormat,
        depth: TextureFormat,
    ) -> bool {
        self.extent == extent
            && self.compressed == compressed
            && self.host_format == host
            && self.depth_format == depth
    }

    /// Returns explicit unsupported classes while submitting all admitted draws.
    /// Missing resources for an admitted draw are errors, never fabricated inputs.
    pub fn record<'a>(
        &mut self,
        r: WaterColourResources<'_>,
        view: WaterColourView<'_>,
        instances: impl IntoIterator<Item = &'a WaterInstance>,
        encoder: &mut CommandEncoder,
    ) -> Result<BTreeSet<String>, String> {
        self.draws.clear();
        let mut pending = BTreeSet::new();
        let mut needs_reflections = false;
        let alignment = u64::from(r.device.limits().min_uniform_buffer_offset_alignment);
        let vertex_stride = 80u64.next_multiple_of(alignment) as usize;
        let fragment_stride = 1008u64.next_multiple_of(alignment) as usize;
        let mut vertex_bytes = Vec::new();
        let mut fragment_bytes = Vec::new();
        let mut remap = Vec::new();
        let mut ordered: Vec<_> = instances.into_iter().collect();
        ordered.sort_by_key(|i| i.order);
        for instance in ordered {
            let Some(WaterGeometry::World { object, matrix }) = instance.geometry else {
                pending.insert("model water colour ABI is not submitted".into());
                continue;
            };
            let material = r
                .materials
                .0
                .get(&instance.material)
                .ok_or("water colour material unavailable")?
                .as_ref()
                .map_err(Clone::clone)?;
            let Some(source) = material.world_colour.as_ref().map_err(Clone::clone)? else {
                continue;
            };
            let class = match &source.class {
                Ok(class) => *class,
                Err(error) => {
                    pending.insert(error.clone());
                    continue;
                }
            };
            if !source.colour_options.horizontal {
                return Err("water colour admission disagrees with horizontal option".into());
            }
            let settings = source.settings(&view.frame.values)?;
            needs_reflections |=
                source.colour_options.reflections && view.frame.ocean_environment.view.enable_ssr;
            let world = r
                .world
                .objects
                .get(&object)
                .ok_or("water colour raw world mesh unavailable")?
                .as_ref()
                .map_err(Clone::clone)?;
            let mesh_id = world.mesh.id();
            let mesh = r
                .meshes
                .get(mesh_id)
                .ok_or("water colour mesh awaits GPU preparation")?;
            let vertex = r
                .allocator
                .mesh_vertex_slice(&mesh_id)
                .ok_or("water colour vertex allocation unavailable")?;
            let state =
                crate::render_state::prepared_surface_state(material.state, world.surface_flags);
            let key = Key {
                layout: mesh.layout.clone(),
                state,
                reversed: world.indices_reversed,
                class,
            };
            let program = if let Some(&i) = self.keys.get(&key) {
                i
            } else {
                let colour = WorldProbeColourProgram::new_for_world_surface(
                    r.device,
                    &mesh.layout,
                    self.depth_format,
                    if self.compressed {
                        TextureFormat::Rg11b10Ufloat
                    } else {
                        TextureFormat::Rgba16Float
                    },
                    state,
                    world.indices_reversed,
                    class,
                )?;
                let i = self.programs.len();
                self.programs.push(Program {
                    colour,
                    parameters: None,
                    inputs: None,
                    probes: class
                        == crate::rendering::water::program::WorldWaterColourClass::ProbeHorizontal,
                });
                self.keys.insert(key, i);
                i
            };
            let instance_offset = u32::try_from(remap.len() / 4)
                .map_err(|_| "water colour instance offset overflow")?;
            let matrix = u32::try_from(matrix).map_err(|_| "water colour matrix index overflow")?;
            if matrix > 0xffffff {
                return Err("water colour matrix index exceeds source remap".into());
            }
            remap.extend_from_slice(&matrix.to_le_bytes());
            let vertex_parameters = WaterWorldColourParameters {
                clip_from_world: view.frame.matrices.current.view_projection,
                instance_buffer_offset: instance_offset,
                first_vertex: vertex.range.start,
                first_instance: 0,
            }
            .uniform_bytes();
            let parameters = parameters(
                &view,
                &settings,
                source.colour_options,
                self.extent,
                self.compressed,
                class == crate::rendering::water::program::WorldWaterColourClass::ProbeHorizontal,
            )?;
            let fragment_parameters = parameters.uniform_bytes()?;
            let sky = r
                .images
                .skies
                .get(&settings.sky_cube)
                .ok_or("water sky cube source unavailable")?;
            let sky_id = Arc::as_ptr(sky) as usize;
            if !self.skies.contains_key(&sky_id) {
                let image = LayeredImage::cube(r.device, r.queue, sky)?;
                // The non-probe specialization never samples the IBL binding.
                // Reuse its real sky allocation for the fixed host layout.
                let array = image.texture().create_view(&TextureViewDescriptor {
                    dimension: Some(TextureViewDimension::CubeArray),
                    ..default()
                });
                self.skies.insert(sky_id, (sky.clone(), image, array));
            }
            let caustics = r
                .images
                .caustics
                .get(&settings.caustics.image)
                .ok_or("water caustics source unavailable")?;
            let caustic_id = Arc::as_ptr(caustics) as usize;
            if !self.caustics.contains_key(&caustic_id) {
                self.caustics.insert(
                    caustic_id,
                    (
                        caustics.clone(),
                        LayeredImage::array(r.device, r.queue, caustics)?,
                    ),
                );
            }
            let vertex_offset = u32::try_from(vertex_bytes.len())
                .map_err(|_| "water vertex parameters overflow")?;
            let fragment_offset = u32::try_from(fragment_bytes.len())
                .map_err(|_| "water fragment parameters overflow")?;
            vertex_bytes.extend(vertex_parameters);
            vertex_bytes.resize(vertex_bytes.len() + vertex_stride - 80, 0);
            fragment_bytes.extend(fragment_parameters);
            fragment_bytes.resize(fragment_bytes.len() + fragment_stride - 1008, 0);
            let indices = if let Some(format) = mesh.index_format() {
                let index = r
                    .allocator
                    .mesh_index_slice(&mesh_id)
                    .ok_or("water colour index allocation unavailable")?;
                Some((
                    index.buffer.clone(),
                    format,
                    index.range.clone(),
                    i32::try_from(vertex.range.start)
                        .map_err(|_| "water colour base vertex overflow")?,
                ))
            } else {
                None
            };
            self.draws.push(Draw {
                program,
                material: instance.material.clone(),
                vertex: vertex.buffer.clone(),
                range: vertex.range.clone(),
                indices,
                vertex_offset,
                fragment_offset,
                sky: sky_id,
                caustics: caustic_id,
            });
        }
        if self.draws.is_empty() {
            return Ok(pending);
        }
        let probes = if self.draws.iter().any(|d| self.programs[d.program].probes) {
            Some(
                r.lighting
                    .0
                    .as_ref()
                    .map_err(Clone::clone)?
                    .as_ref()
                    .ok_or("world water probes unavailable")?,
            )
        } else {
            None
        };
        let buffer = |h: &Handle<bevy::render::storage::ShaderBuffer>| {
            r.buffers
                .get(h)
                .map(|b| &b.buffer)
                .ok_or("water colour buffer awaits GPU preparation")
        };
        let matrices = buffer(
            r.world
                .matrices
                .as_ref()
                .ok_or("water colour world matrices unavailable")?,
        )?;
        let probe_buffers = probes
            .map(|p| {
                Ok::<_, &str>((
                    buffer(&p.vertex_references)?,
                    buffer(&p.probe_info)?,
                    buffer(&p.cube_indices)?,
                ))
            })
            .transpose()?;
        let vertices = upload(
            r.device,
            r.queue,
            &mut self.vertex,
            &vertex_bytes,
            BufferUsages::UNIFORM,
        );
        let fragments = upload(
            r.device,
            r.queue,
            &mut self.fragment,
            &fragment_bytes,
            BufferUsages::UNIFORM,
        );
        let instances = upload(
            r.device,
            r.queue,
            &mut self.remap,
            &remap,
            BufferUsages::STORAGE,
        );
        for (index, p) in self.programs.iter_mut().enumerate() {
            if !self.draws.iter().any(|draw| draw.program == index) {
                continue;
            }
            let ids = [vertices.id(), fragments.id()];
            if p.parameters.as_ref().is_none_or(|v| v.0 != ids) {
                p.parameters = Some((
                    ids,
                    p.colour.vertex.bind_parameters(r.device, vertices),
                    p.colour.parameter_binding(r.device, fragments),
                ));
            }
            let (references, info, cubes) = if p.probes {
                probe_buffers.ok_or("world water probe buffers unavailable")?
            } else {
                (matrices, matrices, matrices)
            };
            let ids = [
                instances.id(),
                matrices.id(),
                references.id(),
                info.id(),
                cubes.id(),
            ];
            if p.inputs.as_ref().is_none_or(|v| v.0 != ids) {
                p.inputs = Some((
                    ids,
                    if !p.probes {
                        p.colour
                            .vertex
                            .bind_world_inputs(r.device, instances, matrices)
                    } else {
                        p.colour.vertex.bind_inputs(
                            r.device,
                            WorldProbeVertexBuffers {
                                instances,
                                matrices,
                                references,
                                probe_info: info,
                                cube_indices: cubes,
                            },
                        )
                    },
                ));
            }
        }
        if self.opaque.as_ref().is_none_or(|v| v.0 != view.opaque.id()) {
            self.opaque = Some((
                view.opaque.id(),
                view.opaque.create_view(&TextureViewDescriptor {
                    aspect: TextureAspect::DepthOnly,
                    ..default()
                }),
            ));
        }
        let reduced = if !needs_reflections {
            // The selected source shaders do not read reduced depth with SSR
            // disabled. Alias an existing sampled image for the host layout;
            // do not allocate or dispatch an unused depth pyramid.
            view.fog
        } else if let Some(pyramid) = view.pyramid {
            if self
                .shared_pyramid
                .as_ref()
                .is_none_or(|v| v.0 != pyramid.texture().id())
            {
                self.shared_pyramid = Some((
                    pyramid.texture().id(),
                    pyramid.texture().create_view(&default()),
                ));
            }
            &self.shared_pyramid.as_ref().unwrap().1
        } else {
            if view.shared_pyramid_expected {
                return Err("water reflections await current shared depth reduction".into());
            }
            if self
                .pyramid
                .as_ref()
                .is_none_or(|v| v.0 != view.opaque.id())
            {
                let pyramid = DepthPyramid::new(r.device, r.depth_programs, view.opaque)?;
                let texture = pyramid.texture().create_view(&default());
                self.pyramid = Some((view.opaque.id(), pyramid, texture));
            }
            let (_, pyramid, texture) = self.pyramid.as_ref().unwrap();
            let projection =
                Mat4::from_cols_array_2d(&view.frame.matrices.current.projection).transpose();
            pyramid.record(
                r.queue,
                encoder,
                r.depth_programs,
                self.extent,
                projection.inverse().transpose().to_cols_array_2d(),
            )?;
            texture
        };
        let probe_cube = probes
            .map(|p| {
                r.image_assets
                    .get(&p.cubes)
                    .ok_or("water probe cube array awaits GPU preparation")
            })
            .transpose()?;
        let native = self.hdr.prepare(
            r.device,
            r.queue,
            encoder,
            view.hdr,
            self.extent,
            view.camera_exposure,
            view.white_level,
            view.adaptation,
        )?;
        for draw in &self.draws {
            let images = [
                self.noise.permutation(),
                self.noise.gradients(),
                &self.opaque.as_ref().unwrap().1,
                reduced,
                native.source(),
                view.fog,
                self.caustics[&draw.caustics].1.view(),
                if self.programs[draw.program].probes {
                    &probe_cube
                        .ok_or("water probe cube array unavailable")?
                        .texture_view
                } else {
                    &self.skies[&draw.sky].2
                },
                self.skies[&draw.sky].1.view(),
                view.atlas,
            ];
            let ids = images.map(|v| v.id());
            let key = (draw.program, draw.material.clone());
            if self
                .bindings
                .get(&key)
                .is_none_or(|v| v.images != ids || v.exposure != view.adaptation.id())
            {
                let binding = self.programs[draw.program]
                    .colour
                    .resource_binding_with_view_exposure(
                        r.device,
                        images,
                        [
                            self.noise.point_wrap(),
                            &self.linear_clamp,
                            &self.trilinear_clamp,
                            &self.trilinear_wrap,
                            &self.trilinear_wrap,
                            &self.comparison,
                        ],
                        view.adaptation,
                    );
                self.bindings.insert(
                    key,
                    MaterialBinding {
                        images: ids,
                        exposure: view.adaptation.id(),
                        binding,
                    },
                );
            }
        }
        {
            let pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("world water colour"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: native.colour(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: view.water_depth,
                    depth_ops: Some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
                    stencil_ops: Some(Operations {
                        load: LoadOp::Load,
                        store: StoreOp::Store,
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = TrackedRenderPass::new(r.device, pass);
            pass.set_viewport(0., 0., self.extent[0] as f32, self.extent[1] as f32, 0., 1.);
            pass.set_scissor_rect(0, 0, self.extent[0], self.extent[1]);
            for draw in &self.draws {
                let program = &self.programs[draw.program];
                let parameters = program.parameters.as_ref().unwrap();
                program.colour.draw(
                    &mut pass,
                    WorldProbeColourDraw {
                        vertex_parameters: &parameters.1,
                        vertex_offset: draw.vertex_offset,
                        vertex_sources: &program.inputs.as_ref().unwrap().1,
                        fragment_parameters: &parameters.2,
                        fragment_offset: draw.fragment_offset,
                        resources: &self.bindings[&(draw.program, draw.material.clone())].binding,
                        vertices: &draw.vertex,
                        indices: draw
                            .indices
                            .as_ref()
                            .map(|(b, f, r, base)| (b, *f, r.clone(), *base)),
                        vertex_range: draw.range.clone(),
                        instances: 0..1,
                    },
                    0,
                );
            }
        }
        native.finish(encoder);
        Ok(pending)
    }
}

fn upload<'a>(
    device: &RenderDevice,
    queue: &RenderQueue,
    buffer: &'a mut Option<Buffer>,
    bytes: &[u8],
    usage: BufferUsages,
) -> &'a Buffer {
    if buffer
        .as_ref()
        .is_none_or(|b| b.size() < bytes.len() as u64)
    {
        *buffer = Some(device.create_buffer(&BufferDescriptor {
            label: Some("water colour draw data"),
            size: (bytes.len() as u64).next_power_of_two(),
            usage: usage | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    let target = buffer.as_ref().unwrap();
    queue.write_buffer(target, 0, bytes);
    target
}

fn parameters(
    view: &WaterColourView<'_>,
    settings: &crate::rendering::water::WaterSettings,
    options: crate::rendering::water::program::WaterColourOptions,
    extent: [u32; 2],
    compressed: bool,
    use_probes: bool,
) -> Result<WorldProbeColourParameters, String> {
    let matrices = view.frame.matrices.current;
    let parallel = view
        .frame
        .parallel
        .ok_or("water view sun inputs unavailable")?;
    let to_columns =
        |rows: &[[f32; 4]; 4]| Mat4::from_cols_array_2d(rows).transpose().to_cols_array();
    let mut inverse = [[0.; 4]; 4];
    if !crate::render_shadow::invert_rows(&matrices.projection, &mut inverse) {
        return Err("water projection is singular".into());
    }
    let unproject = [-inverse[2][3], inverse[3][2], inverse[3][3], 0.];
    let mut reflection = super::water_reflection::WaterReflectionParameters {
        view_from_world: to_columns(&matrices.view),
        camera_to_clip: to_columns(&matrices.projection),
        unproject,
        thickness: [0.; 4],
        trace: [0.; 4],
        screen_hdr: [1., 1., f32::NAN, view.white_level],
        options: [
            u32::from(options.reflections),
            u32::from(view.frame.ocean_environment.view.enable_ssr),
            0,
            u32::from(compressed),
        ],
    };
    reflection.update_settings(&settings.reflection);
    let refracted = crate::rendering::water::refracted_light_direction(parallel.0);
    Ok(WorldProbeColourParameters {
        reflection,
        shadow: view.shadow,
        refraction: super::water_refraction::WaterRefractionParameters {
            clock_caustics: [
                view.frame.local_time[0],
                settings.caustics.speed,
                settings.caustics.tiling,
                settings.caustics.chromatic_aberration,
            ],
            screen: [1., 1., 1. / extent[0] as f32, 1. / extent[1] as f32],
            unproject: [
                unproject[0],
                unproject[1],
                unproject[2],
                settings.refraction,
            ],
            light_level: [refracted[0], refracted[1], settings.caustics.fade, 0.],
            // The placed fragment supplies its own surface Z and extinction.
            extinction: [0., 0., 0., u32::from(compressed) as f32],
            hdr: [f32::NAN, view.white_level, 0., 0.],
        },
        surface: super::water_surface::WaterSurfaceParameters {
            underwater_visibility_boost: settings.underwater_visibility_boost,
            shore_foam_luminance: settings.shore_foam_luminance,
            use_sun: options.sunlight,
            disable_ssr_on_rough_water: options.disable_rough_reflections,
        },
        colour: super::water_colour::WaterColourParameters::from_settings(
            settings,
            view.frame.local_time[0],
            matrices.position,
            to_columns(&matrices.view),
            parallel,
            options.horizontal,
        ),
        output: super::colour_output::ColourOutputParameters {
            tint_exposure: [0., 0., 0., f32::NAN],
            white_level: [view.white_level, 0., 0., 0.],
            options: [0, 0, u32::from(compressed), 0],
        },
        sky_radius: [
            settings.sky_colour[0],
            settings.sky_colour[1],
            settings.sky_colour[2],
            settings.sky_cube_radius,
        ],
        use_sky_cube: options.sky_cube,
        use_probes,
    })
}
