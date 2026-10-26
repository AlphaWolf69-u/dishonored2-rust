//! Gameplay owner for the existing ocean grid, wave simulation and raster stages.
use super::{
    ocean_depth::OceanDepthProgram,
    ocean_mesh::{OceanMeshBuffers, OceanPatchDraw},
    ocean_surface::OceanSurfaceParameters,
    ocean_vertex::{OceanVertexParameters, OceanVertexView},
    ocean_waves::{FoamSettings, OceanWaves},
    water_scene::{WaterFrame, WaterImageSources},
};
use crate::rendering::ocean::*;
use bevy::{
    prelude::*,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
use std::{collections::HashMap, sync::Arc};

/// 302960's seven program slots, loaded from the installed material declarations.
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct OceanSources(pub Option<Arc<OceanPrograms>>);
pub struct OceanPrograms {
    pub states: [crate::render_state::RenderState; 7],
    pub sunlight: [bool; 7],
    pub water_line_state: crate::render_state::RenderState,
    pub underwater_state: crate::render_state::RenderState,
    pub volume_underwater_state: crate::render_state::RenderState,
    pub underwater_effect: super::underwater_scattering::UnderwaterProgram,
}
impl OceanSources {
    pub fn load(
        catalog: &crate::archive::Catalog,
        library: &mut crate::shader_cache::ProgramLibrary,
    ) -> Result<Self, String> {
        let programs = [
            OceanProgram::Depth,
            OceanProgram::Surface,
            OceanProgram::Reflection,
            OceanProgram::ReflectionRoughCull,
            OceanProgram::SkyCube,
            OceanProgram::ReflectionSkyCube,
            OceanProgram::ReflectionRoughCullSkyCube,
        ];
        let mut states = [[0; 9]; 7];
        let mut sunlight = [false; 7];
        for (index, program) in programs.into_iter().enumerate() {
            let path = format!("generated/decls/material/{}.decl", program.material());
            let bytes = catalog
                .read(&path)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("ocean material absent: {path}"))?;
            let field = if index == 0 {
                "zPrepassProgram"
            } else {
                "mainProgram"
            };
            let name = crate::material::authored_program_override(&bytes, field)
                .ok_or_else(|| format!("{path} has no {field}"))?;
            let definition = library
                .definition(name.as_bytes())?
                .ok_or_else(|| format!("ocean program absent: {name}"))?;
            states[index] = crate::render_state::material_render_state(
                &crate::entities::declaration_text(&bytes),
                definition.render_state(),
            )?;
            let options = crate::material::material_option_parameters(&bytes, |n| {
                library.parameter_kinds.get(n.as_bytes())
                    == Some(&crate::render_program::RenderParameterKind::Option)
            })?;
            let options = crate::render_program::SerializedParameterBlock {
                flags: 0,
                parameters: options
                    .into_iter()
                    .map(|(name, value)| crate::render_program::SerializedParameter {
                        name: name.into_bytes(),
                        kind: crate::render_program::RenderParameterKind::Option,
                        value: crate::render_program::SerializedParameterValue::Words([
                            value as u32,
                            0,
                            0,
                            0,
                        ]),
                    })
                    .collect(),
            };
            sunlight[index] =
                library.option_value(name.as_bytes(), &options, b"water/UseSunLight")? != 0;
        }
        let water_line_state = library
            .definition(b"water/DepthWaterLine")?
            .ok_or("ocean water-line program absent")?
            .render_state();
        let underwater_state = library
            .definition(b"water/UnderWaterStencil")?
            .ok_or("ocean underwater program absent")?
            .render_state();
        let volume_underwater_state = library
            .definition(b"water/UnderWaterVolumeStencil")?
            .ok_or("water volume underwater program absent")?
            .render_state();
        Ok(Self(Some(Arc::new(OceanPrograms {
            states,
            sunlight,
            water_line_state,
            underwater_state,
            volume_underwater_state,
            underwater_effect:super::underwater_scattering::UnderwaterProgram::load(catalog,library)?,
        }))))
    }
}

struct Image2d {
    _texture: Texture,
    view: TextureView,
}
struct DepthProgram {
    program: OceanDepthProgram,
    binding: BindGroup,
}
struct ColourProgram {
    program: super::ocean_colour::OceanColourProgram,
}
struct ColourTargets {
    extent: [u32; 2],
    compressed: bool,
    format: TextureFormat,
    hdr: super::water_hdr::WaterHdr,
    foam: super::water_foam_depth::WaterFoamDepth,
}
struct UnderwaterTargets {
    extent: UVec2,
    full_format: TextureFormat,
    line: super::ocean_water_line::OceanWaterLine,
    stencil: super::underwater_stencil::UnderwaterStencilPass,
}
/// One camera owns simulation history; resizing its targets does not restart FFT.
pub struct OceanScene {
    pub state: OceanUpdateState,
    waves: OceanWaves,
    mesh: OceanMeshBuffers,
    noise: super::noise_textures::NoiseTextures,
    images: HashMap<String, Image2d>,
    wrap: Sampler,
    clamp: Sampler,
    depth: HashMap<crate::render_state::RenderState, DepthProgram>,
    uniforms: Option<Buffer>,
    records: Vec<usize>,
    vertices: Vec<OceanVertexParameters>,
    surface: Option<OceanSurfaceParameters>,
    last_frame: Option<u32>,
    colour_targets: Option<ColourTargets>,
    colour_programs:
        HashMap<(usize, TextureFormat, crate::render_state::RenderState), ColourProgram>,
    colour_uniform: Option<Buffer>,
    skies: HashMap<String, (super::layered_images::LayeredImage, TextureView)>,
    caustics: HashMap<String, super::layered_images::LayeredImage>,
    comparison: Sampler,
    underwater: Option<UnderwaterTargets>,
}
impl OceanScene {
    /// Borrow the actual gameplay simulation outputs only when F8 requests them.
    pub(super) fn capture_planes(&self) -> Vec<(&'static str, &Texture)> {
        let mut planes = vec![
            ("ocean_displacement", self.waves.displacement()),
            ("ocean_gradients", self.waves.gradients()),
            ("ocean_foam", self.waves.foam_mask()),
        ];
        if let Some(targets) = &self.colour_targets {
            planes.push(("ocean_foam_depth", targets.foam.texture()));
            planes.extend(targets.hdr.capture_planes());
        }
        planes
    }
    pub fn new(device: &RenderDevice, queue: &RenderQueue) -> Result<Self, String> {
        let sampler = |address| {
            device.create_sampler(&SamplerDescriptor {
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                mipmap_filter: MipmapFilterMode::Linear,
                lod_max_clamp: 13.,
                ..default()
            })
        };
        Ok(Self {
            state: default(),
            waves: OceanWaves::for_view(device, queue)?,
            // 302960 writes owner+8c=16 before 301780 uploads the shared grid.
            mesh: OceanMeshBuffers::new(device, build_ocean_geometry(16)?),
            noise: super::noise_textures::NoiseTextures::new(device, queue),
            images: default(),
            wrap: sampler(AddressMode::Repeat),
            clamp: sampler(AddressMode::ClampToEdge),
            depth: default(),
            uniforms: None,
            records: Vec::new(),
            vertices: Vec::new(),
            surface: None,
            last_frame: None,
            colour_targets: None,
            colour_programs: default(),
            colour_uniform: None,
            skies: default(),
            caustics: default(),
            comparison: super::shadow_pipeline::shadow_comparison_sampler(device),
            underwater: None,
        })
    }
    fn admit_images(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        sources: &WaterImageSources,
        settings: &OceanSettings,
    ) -> Result<(), String> {
        for name in [
            &settings.surface.shore_distance_image,
            &settings.surface.wave_amplitude_image,
            &settings.surface.foam_image,
            &settings.projected_shadow.image,
        ] {
            if self.images.contains_key(name) {
                continue;
            }
            let image = sources
                .ocean
                .get(name)
                .ok_or_else(|| format!("ocean image not prepared: {name}"))?;
            let format = super::image_formats::gpu_image_format(image.info.format);
            if !device.features().contains(format.required_features()) {
                return Err(format!("ocean image needs unsupported {format:?}"));
            }
            let texture = device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("ocean authored mask"),
                    size: Extent3d {
                        width: image.info.width,
                        height: image.info.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: image.info.mip_count,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                &image.payload,
            );
            self.images.insert(
                name.clone(),
                Image2d {
                    view: texture.create_view(&default()),
                    _texture: texture,
                },
            );
        }
        Ok(())
    }
    /// 302d50 -> 300af0, then305300/305270. The pending contact query is
    /// represented explicitly, using88d060's existing invalid-query behavior.
    /// This is not a fabricated completed query or a second physics world.
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &WaterFrame,
        images: &WaterImageSources,
        extent: [u32; 2],
    ) -> Result<(), String> {
        self.prepare_with_selection(device,queue,encoder,frame,images,extent,None)
    }

    pub fn prepare_with_selection(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &WaterFrame,
        images: &WaterImageSources,
        extent: [u32;2],
        selection: Option<&crate::rendering::environment_contacts::WaterViewPublication>,
    ) -> Result<(), String> {
        self.records.clear();
        self.vertices.clear();
        let settings = &frame.ocean_environment;
        if self.last_frame != Some(frame.matrices.frame) {
            self.waves.begin_world_frame(encoder, &mut self.state);
            self.last_frame = Some(frame.matrices.frame);
        }
        self.state.prepare(
            &settings.view,
            frame.ocean_render_settings.tile_size,
            selection.map_or(true,|selection|selection.update_ocean_height),
        );
        if !self.state.view.enabled || frame.ocean_render_settings.skip {
            return Ok(());
        }
        self.admit_images(device, queue, images, settings)?;
        let matrices = frame.matrices.current;
        let clip = Mat4::from_cols_array_2d(&matrices.view_projection)
            .transpose()
            .to_cols_array();
        let console = crate::platform::console::shared().read();
        let threshold = console
            .cvars
            .get("r_oceanGeometricDetails")
            .ok_or("ocean geometric details cvar absent")?
            .float;
        let patch_view = OceanPatchView {
            eye: matrices.position,
            surface_height: self.state.view.water_level,
            grid_size: 16,
            projection_scale: [matrices.projection[0][0], matrices.projection[1][1]],
            pixel_area: (extent[0] as f32) * (extent[1] as f32),
        };
        // 302d50: root side=2*zfar, origin=(-zfar,-zfar), independent of eye XY.
        let root = OceanPatch {
            origin: [-frame.near_far[1]; 2],
            size: frame.near_far[1] * 2.,
            lod: 0,
            children: [-1; 4],
        };
        let mut patches = Vec::new();
        build_ocean_patches(
            &mut patches,
            root,
            self.state.view.tile_size,
            threshold,
            |p| {
                !self.state.view.water_volume
                    && ocean_patch_visible(
                        ocean_patch_bounds(
                            p,
                            self.state.view.water_level,
                            self.state.view.max_wave_amplitude,
                            self.state.view.tile_size,
                        ),
                        &matrices.view_projection,
                    )
            },
            |p| ocean_projected_cell_area(p, patch_view),
        )?;
        let vertex_view = OceanVertexView {
            clip_from_world: clip,
            previous_clip_from_world: Mat4::from_cols_array_2d(
                &frame.matrices.previous.view_projection,
            )
            .transpose()
            .to_cols_array(),
            view_row: matrices.view[2],
            jitter_relative_to_previous: [0.; 2],
        };
        for patch in patches.iter().filter(|p| p.children == [-1; 4]) {
            self.records.push(ocean_geometry_record(
                *patch,
                &patches,
                self.state.view.tile_size,
                16,
            )?);
            self.vertices.push(OceanVertexParameters {
                view: vertex_view,
                patch_transform: ocean_patch_transform(*patch, self.state.view.water_level, 16),
                perlin_blend_end: settings.surface.perlin.blend_end,
            });
        }
        self.surface = Some(OceanSurfaceParameters::from_settings(
            &settings.surface,
            frame.local_time[0],
            matrices.position,
            frame.near_far,
        ));
        self.waves
            .prepare_time(queue, frame.local_time[0], settings.surface.time_scale);
        self.waves.prepare_foam(
            queue,
            FoamSettings::from_parameters(settings.foam, frame.delta_seconds),
        );
        self.waves.record_view_spectrum(
            queue,
            encoder,
            &mut self.state,
            self.records.len() as u32,
        )?;
        self.waves
            .record_view_surfaces(queue, encoder, &mut self.state, self.records.len() as u32);
        Ok(())
    }
    pub fn draw_count(&self) -> usize {
        self.records.len()
    }
    pub fn record_depth(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &WaterFrame,
        sources: &OceanPrograms,
        depth: &TextureView,
        motion: &TextureView,
        extent: [u32; 2],
    ) -> Result<(), String> {
        if self.records.is_empty() {
            return Ok(());
        }
        let (state, reference) = ocean_draw_state(
            sources.states[0],
            OceanDrawInputs {
                depth_pass: true,
                eye_height: frame.matrices.current.position[2],
                surface_height: self.state.view.water_level,
                height_extent: self.state.view.max_wave_amplitude,
                wireframe: false,
                mirrored: false,
            },
        );
        let stride = 288u64.next_multiple_of(u64::from(
            device.limits().min_uniform_buffer_offset_alignment,
        )) as usize;
        let mut bytes = vec![0; stride * self.vertices.len()];
        for (slot, vertex) in bytes.chunks_exact_mut(stride).zip(&self.vertices) {
            slot[..288].copy_from_slice(&super::ocean_depth::uniform_bytes(
                self.surface.as_ref().unwrap(),
                vertex,
            ));
        }
        let changed = self
            .uniforms
            .as_ref()
            .is_none_or(|b| b.size() < bytes.len() as u64);
        if changed {
            self.uniforms = Some(device.create_buffer(&BufferDescriptor {
                label: Some("ocean depth patches"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let uniform = self.uniforms.as_ref().unwrap();
        queue.write_buffer(uniform, 0, &bytes);
        if changed {
            for p in self.depth.values_mut() {
                p.binding = p.program.parameter_binding(device, uniform);
            }
        }
        if !self.depth.contains_key(&state) {
            let program = OceanDepthProgram::new(
                device,
                TextureFormat::Depth32FloatStencil8,
                state,
                reference,
                false,
            )?;
            let binding = program.parameter_binding(device, uniform);
            self.depth.insert(state, DepthProgram { program, binding });
        }
        let settings = &frame.ocean_environment;
        let p = &self.depth[&state];
        let waves = p.program.texture_binding(
            device,
            [
                self.noise.permutation(),
                self.noise.gradients(),
                &self.images[&settings.surface.shore_distance_image].view,
                self.waves.displacement_view(),
                self.waves.previous_displacement_view(),
                self.waves.collision_height_view(),
                self.waves.foam_view(),
                &self.images[&settings.surface.wave_amplitude_image].view,
            ],
            [self.noise.point_wrap(), &self.wrap, &self.clamp],
        );
        let draws = self
            .records
            .iter()
            .enumerate()
            .map(|(i, &record)| OceanPatchDraw {
                record,
                parameters: &p.binding,
                parameter_offset: (i * stride) as u32,
            })
            .collect::<Vec<_>>();
        p.program.record(
            encoder,
            depth,
            Some(motion),
            [0., 0., extent[0] as f32, extent[1] as f32],
            [0, 0, extent[0], extent[1]],
            &self.mesh,
            &waves,
            &draws,
        )
    }

    /// 2fdea0's ocean branch: a wave-height mask, then full/half underwater
    /// classification. Admission depends on the enabled ocean, not visible
    /// surface patches (a camera below water can have no visible leaves).
    pub fn record_underwater(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &WaterFrame,
        sources: &OceanPrograms,
        depth: &super::water_depth_scene::WaterDepthScene,
        main: &bevy::render::view::ViewDepthTexture,
    ) -> Result<(), String> {
        use super::{
            ocean_water_line::{OceanWaterLine, WaterLineInputs, WaterLineParameters},
            underwater_stencil::{
                UnderwaterStencilParameters, UnderwaterStencilPass, UnderwaterStencilTarget,
            },
        };
        if !self.state.view.enabled
            || frame.ocean_render_settings.skip
            || self.state.view.water_volume
        {
            return Ok(());
        }
        let full = UVec2::new(depth.alpha.full.width(), depth.alpha.full.height());
        let half = UVec2::new(depth.alpha.reduced.width(), depth.alpha.reduced.height());
        // The source allocates ceil(half/2), while its quarter viewport uses
        // integer division of the full viewport. Keep padding out of sampling.
        let extent = (half + UVec2::ONE) / 2;
        let quarter = full / 4;
        if quarter.min_element() == 0 {
            return Err("underwater quarter viewport is empty".into());
        }
        if self.underwater.as_ref().is_none_or(|t| t.extent != extent || t.full_format!=main.texture.format()) {
            self.underwater = Some(UnderwaterTargets {
                extent,
                full_format:main.texture.format(),
                line: OceanWaterLine::new(device, extent, sources.water_line_state)?,
                stencil: UnderwaterStencilPass::with_target_formats(
                    device,
                    [main.texture.format(),depth.alpha.reduced.format()],
                    sources.underwater_state,
                )?,
            });
        }
        let world_from_clip = Mat4::from_cols_array_2d(&frame.matrices.current.view_projection)
            .transpose()
            .inverse();
        let view_parameters = |size: UVec2| UnderwaterStencilParameters {
            world_from_clip,
            // Ocean fragment uses the wave mask, not the volume height varying.
            water_matrix: Mat4::ZERO,
            viewport: Vec4::new(0., 0., size.x as f32, size.y as f32),
            resolution_scale: Vec4::new(
                quarter.x as f32 / extent.x as f32,
                quarter.y as f32 / extent.y as f32,
                0.,
                0.,
            ),
        };
        let settings = &frame.ocean_environment;
        let targets = self.underwater.as_mut().unwrap();
        targets.line.record_underwater(
            &mut targets.stencil,
            device,
            queue,
            encoder,
            WaterLineParameters {
                world_from_clip,
                viewport: Vec4::new(0., 0., quarter.x as f32, quarter.y as f32),
                water_level: Vec4::new(self.state.view.water_level, 0., 0., 0.),
            },
            self.surface
                .as_ref()
                .ok_or("underwater ocean surface was not prepared")?,
            WaterLineInputs {
                amplitude_mask: &self.images[&settings.surface.wave_amplitude_image].view,
                shore_distance: &self.images[&settings.surface.shore_distance_image].view,
                collision_height: self.waves.collision_height_view(),
            },
            [
                UnderwaterStencilTarget {
                    attachment: main.view(),
                    parameters: view_parameters(full),
                },
                UnderwaterStencilTarget {
                    attachment: &depth.alpha.reduced_attachment,
                    parameters: view_parameters(half),
                },
            ],
            &depth.water.vertices,
            0..3,
        )
    }

    pub fn record_colour(
        &mut self,
        r: &super::water_colour_scene::WaterColourResources<'_>,
        view: &super::water_colour_scene::WaterColourView<'_>,
        sources: &OceanPrograms,
        extent: [u32; 2],
        compressed: bool,
        host_format: TextureFormat,
        water_depth: &TextureView,
        encoder: &mut CommandEncoder,
    ) -> Result<(), String> {
        if self.records.is_empty() {
            return Ok(());
        }
        let frame = view.frame;
        let settings = &frame.ocean_environment;
        if self.colour_targets.as_ref().is_none_or(|t| {
            t.extent != extent || t.compressed != compressed || t.format != host_format
        }) {
            self.colour_targets = Some(ColourTargets {
                extent,
                compressed,
                format: host_format,
                hdr: super::water_hdr::WaterHdr::new(
                    r.device,
                    r.queue,
                    extent,
                    compressed,
                    host_format,
                )?,
                foam: super::water_foam_depth::WaterFoamDepth::new(
                    r.device,
                    r.queue,
                    UVec2::from_array(extent),
                )?,
            });
        }
        if !self.skies.contains_key(&settings.water.sky_cube) {
            let source = r
                .images
                .skies
                .get(&settings.water.sky_cube)
                .ok_or("ocean sky source unavailable")?;
            let image = super::layered_images::LayeredImage::cube(r.device, r.queue, source)?;
            let array = image.texture().create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::CubeArray),
                ..default()
            });
            self.skies
                .insert(settings.water.sky_cube.clone(), (image, array));
        }
        if !self.caustics.contains_key(&settings.water.caustics.image) {
            let source = r
                .images
                .caustics
                .get(&settings.water.caustics.image)
                .ok_or("ocean caustics source unavailable")?;
            self.caustics.insert(
                settings.water.caustics.image.clone(),
                super::layered_images::LayeredImage::array(r.device, r.queue, source)?,
            );
        }
        let console = crate::platform::console::shared().read();
        let skip_ssr = console
            .cvars
            .get("r_skipOceanSSR")
            .ok_or("ocean SSR cvar absent")?
            .int
            != 0;
        let selected = ocean_program(
            false,
            self.state.view.enable_ssr && !skip_ssr,
            self.state.view.disable_ssr_on_rough_water,
            self.state.view.use_sky_cube_map,
        ) as usize;
        let (state, reference) = ocean_draw_state(
            sources.states[selected],
            OceanDrawInputs {
                depth_pass: false,
                eye_height: frame.matrices.current.position[2],
                surface_height: self.state.view.water_level,
                height_extent: self.state.view.max_wave_amplitude,
                wireframe: false,
                mirrored: false,
            },
        );
        let colour_format = if compressed {
            TextureFormat::Rg11b10Ufloat
        } else {
            TextureFormat::Rgba16Float
        };
        let key = (selected, colour_format, state);
        if !self.colour_programs.contains_key(&key) {
            self.colour_programs.insert(
                key,
                ColourProgram {
                    program: super::ocean_colour::OceanColourProgram::with_view_exposure(
                        r.device,
                        TextureFormat::Depth32FloatStencil8,
                        colour_format,
                        state,
                        reference,
                    )?,
                },
            );
        }
        let options = crate::rendering::water::program::WaterColourOptions {
            horizontal: true,
            sunlight: sources.sunlight[selected],
            reflections: self.state.view.enable_ssr && !skip_ssr,
            sky_cube: self.state.view.use_sky_cube_map,
            disable_rough_reflections: self.state.view.disable_ssr_on_rough_water,
        };
        let water = super::water_colour_scene::parameters(
            view,
            &settings.water,
            options,
            extent,
            compressed,
            false,
        )?;
        let parallel = frame.parallel.ok_or("ocean sunlight inputs unavailable")?;
        let mut parameters = super::ocean_colour::OceanColourView {
            surface: self.surface.ok_or("ocean surface was not prepared")?,
            vertex: self.vertices[0],
            reflection: water.reflection,
            refraction: water.refraction,
            shadow: water.shadow,
            water: water.surface,
            colour: super::ocean_colour::OceanColourParameters {
                projected: [0.; 4],
                water: [0.; 4],
                light: [
                    parallel.0[0],
                    parallel.0[1],
                    parallel.0[2],
                    settings.water.inscattering,
                ],
                sun: [
                    parallel.1[0],
                    parallel.1[1],
                    parallel.1[2],
                    settings.water.specular_power,
                ],
                pixel_to_texture: [0., 0., 1. / extent[0] as f32, 1. / extent[1] as f32],
            },
            output: water.output,
            sky_radius: water.sky_radius,
            use_sky_cube: options.sky_cube,
        };
        parameters.update_environment(settings, &self.state.view);
        let stride = 1200u64.next_multiple_of(u64::from(
            r.device.limits().min_uniform_buffer_offset_alignment,
        )) as usize;
        let mut bytes = vec![0; stride * self.vertices.len()];
        for (slot, vertex) in bytes.chunks_exact_mut(stride).zip(&self.vertices) {
            parameters.vertex = *vertex;
            slot[..1200].copy_from_slice(&parameters.uniform_bytes());
        }
        if self
            .colour_uniform
            .as_ref()
            .is_none_or(|b| b.size() < bytes.len() as u64)
        {
            self.colour_uniform = Some(r.device.create_buffer(&BufferDescriptor {
                label: Some("ocean colour patches"),
                size: (bytes.len() as u64).next_power_of_two(),
                usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let uniform = self.colour_uniform.as_ref().unwrap();
        r.queue.write_buffer(uniform, 0, &bytes);
        let program = &self.colour_programs[&key].program;
        let parameters_binding =
            program.parameter_binding_with_exposure(r.device, uniform, view.adaptation);
        let waves = program.wave_binding(
            r.device,
            [
                self.noise.permutation(),
                self.noise.gradients(),
                &self.images[&settings.surface.shore_distance_image].view,
                self.waves.displacement_view(),
                self.waves.previous_displacement_view(),
                self.waves.collision_height_view(),
                self.waves.foam_view(),
                &self.images[&settings.surface.wave_amplitude_image].view,
            ],
            [self.noise.point_wrap(), &self.wrap, &self.clamp],
        );
        let opaque = view.opaque.create_view(&TextureViewDescriptor {
            aspect: TextureAspect::DepthOnly,
            ..default()
        });
        let reduced = if options.reflections {
            view.pyramid
                .ok_or("ocean reflections await current depth pyramid")?
                .texture()
                .create_view(&default())
        } else {
            // The non-SSR shader does not read reduced depth. Alias a real
            // same-view sample instead of allocating a dummy depth pyramid.
            view.fog.clone()
        };
        let targets = self.colour_targets.as_mut().unwrap();
        let matrices = frame.matrices.current;
        let projection = Mat4::from_cols_array_2d(&matrices.projection).transpose();
        let inverse_projection = projection.inverse().transpose().to_cols_array_2d();
        let foam_extent = targets.foam.extent();
        targets.foam.record(
            r.device,
            r.queue,
            encoder,
            &opaque,
            water_depth,
            super::water_foam_depth::WaterFoamView {
                inverse_view_projection: Mat4::from_cols_array_2d(&matrices.view_projection)
                    .transpose()
                    .inverse(),
                camera: Vec4::new(
                    matrices.position[0],
                    matrices.position[1],
                    matrices.position[2],
                    1.,
                ),
                linear_depth: Vec4::new(
                    -inverse_projection[2][3],
                    inverse_projection[3][2],
                    inverse_projection[3][3],
                    matrices.view[2][2],
                ),
                near_far: Vec4::new(frame.near_far[0], frame.near_far[1], 0., 0.),
            },
            Vec4::new(1. / foam_extent.x as f32, 1. / foam_extent.y as f32, 1., 1.),
            super::water_foam_depth::WaterFoamRegion {
                viewport: [0., 0., foam_extent.x as f32, foam_extent.y as f32],
                scissor: [0, 0, foam_extent.x, foam_extent.y],
            },
        )?;
        let hdr = targets.hdr.prepare(
            r.device,
            r.queue,
            encoder,
            view.hdr,
            extent,
            view.camera_exposure,
            view.white_level,
            view.adaptation,
        )?;
        let sky = &self.skies[&settings.water.sky_cube];
        let scene = program.scene_binding(
            r.device,
            [
                &self.images[&settings.projected_shadow.image].view,
                &self.images[&settings.surface.foam_image].view,
                targets.foam.view(),
                &opaque,
                &reduced,
                hdr.source(),
                view.fog,
                self.caustics[&settings.water.caustics.image].view(),
                view.atlas,
                sky.0.view(),
                &sky.1,
            ],
            &self.comparison,
        );
        let draws = self
            .records
            .iter()
            .enumerate()
            .map(|(i, &record)| OceanPatchDraw {
                record,
                parameters: &parameters_binding,
                parameter_offset: (i * stride) as u32,
            })
            .collect::<Vec<_>>();
        program.record(
            encoder,
            view.water_depth,
            hdr.colour(),
            [0., 0., extent[0] as f32, extent[1] as f32],
            [0, 0, extent[0], extent[1]],
            &self.mesh,
            &waves,
            &scene,
            &draws,
        )?;
        hdr.finish(encoder);
        Ok(())
    }
}
