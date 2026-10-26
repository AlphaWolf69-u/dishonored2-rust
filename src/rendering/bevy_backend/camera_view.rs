//! Per-view ownership of the ordered autofocus, masks, opaque blur and combine.
//! Transparent preparation remains an explicit input of the surrounding scene;
//! opaque passes never sample their unused refraction binding.
use super::{
    autofocus::{AutofocusParameters, AutofocusPass},
    camera_alpha_colours::{CameraAlphaColours, CameraAlphaConfig, CameraAlphaInputs},
    camera_alpha_composite::{AlphaCompositeInputs, AlphaHostInputs, CameraAlphaComposite},
    camera_blur::BlurKernelBuffer,
    camera_colours::{CameraColourConfig, CameraColourInputs, CameraColourSource, CameraColours},
    camera_effects::{CameraCombineInputs, CameraCombineParameters, CameraCombinePass},
    camera_fuzziness::{CameraFuzzinessInputs, CameraFuzzinessParameters, CameraFuzzinessPass},
    camera_masks::{CameraMaskInputs, CameraMasks},
    colour_downscale::HdrDownscale,
};
use crate::{
    assets::camera_blur::BlurResources,
    rendering::{camera_effects::CameraEffectsFrame, depth_of_field::DofPass},
};
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CameraViewConfig {
    pub extent: [u32; 2],
    pub reduced_extent: [u32; 2],
    pub host_format: TextureFormat,
    pub depth_format: TextureFormat,
    pub compressed: bool,
}
pub struct CameraViewInputs<'a> {
    pub effects: &'a CameraEffectsFrame,
    pub resources: &'a BlurResources,
    pub colour: &'a TextureView,
    pub depth: &'a TextureView,
    pub alpha_depth: &'a TextureView,
    pub alpha_attachment: &'a TextureView,
    pub linear_water_depth: &'a TextureView,
    pub linear_depth_extent: [u32; 2],
    pub adaptation: &'a Buffer,
    /// The image manager's `_black` (2f5630), used by simple DoF.
    pub black: &'a TextureView,
    pub camera_exposure: f32,
    pub adapt_max: f32,
    pub white_level: f32,
    pub projection: [[f32; 4]; 4],
    pub inverse_z: [f32; 3],
    pub local_time: [f32; 2],
    pub scattering_colour: [f32; 3],
    pub destination: &'a TextureView,
    pub alpha: Option<CameraAlphaViewInputs<'a>>,
}
/// Present only after the scene has submitted real opacity and alpha geometry.
pub struct CameraAlphaViewInputs<'a> {
    pub refraction: &'a TextureView,
    pub full_depth_attachment: &'a TextureView,
    pub reduction_shift: u32,
    pub clear_colour: [f32; 3],
    pub special_alpha: bool,
}
pub struct CameraView {
    config: CameraViewConfig,
    half_extent: [u32; 2],
    half_colour: HdrDownscale,
    far_colour: HdrDownscale,
    masks: CameraMasks,
    colours: Option<CameraColours>,
    alpha_colours: Option<(CameraAlphaConfig, CameraAlphaColours, CameraAlphaComposite)>,
    alpha_mask: CameraFuzzinessPass,
    kernels: [BlurKernelBuffer; 6],
    _focus: [Texture; 2],
    focus_views: [TextureView; 2],
    focus_index: usize,
    autofocus: AutofocusPass,
    combine: CameraCombinePass,
    vertices: Buffer,
}
impl CameraView {
    pub fn new(
        device: &RenderDevice,
        config: CameraViewConfig,
        resources: &BlurResources,
    ) -> Result<Self, String> {
        if config.extent.contains(&0) || config.reduced_extent.contains(&0) {
            return Err("camera view requires a nonempty target".into());
        }
        let half_extent = config.reduced_extent;
        let focus = std::array::from_fn(|_| {
            device.create_texture(&TextureDescriptor {
                label: Some("camera autofocus history"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: TextureFormat::Rg32Float,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::STORAGE_BINDING
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        });
        let focus_views = focus.each_ref().map(|t| t.create_view(&Default::default()));
        let kernels = (0..6)
            .map(|i| {
                resources
                    .kernels
                    .get(i)
                    .map(|k| BlurKernelBuffer::new(device, k))
                    .ok_or_else(|| format!("missing camera kernel {i}"))
            })
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "camera kernel bank size")?;
        let vertices = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("camera fullscreen geometry"),
            contents: &[
                [-1.0f32, 1., 0., 0., 0.],
                [3., 1., 0., 2., 0.],
                [-1., -3., 0., 0., 2.],
            ]
            .into_iter()
            .flatten()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
            usage: BufferUsages::VERTEX,
        });
        Ok(Self {
            config,
            half_extent,
            half_colour: HdrDownscale::new(device, half_extent, config.compressed)?,
            far_colour: HdrDownscale::new(device, half_extent, config.compressed)?,
            masks: CameraMasks::new(device, half_extent)?,
            colours: None,
            alpha_colours: None,
            alpha_mask: CameraFuzzinessPass::new(device, TextureFormat::Rg8Unorm),
            kernels,
            _focus: focus,
            focus_views,
            focus_index: 0,
            autofocus: AutofocusPass::new(device),
            combine: CameraCombinePass::with_hardware_depth(device, config.host_format),
            vertices,
        })
    }
    pub fn matches(&self, config: CameraViewConfig) -> bool {
        self.config == config
    }
    /// The flare source remains the unfiltered colour even after final combine.
    pub fn half_colour(&self) -> &TextureView {
        self.half_colour.output()
    }
    pub fn capture_planes(&self) -> Vec<(&'static str, &Texture)> {
        let mut planes = vec![
            ("dof_focus", &self._focus[self.focus_index]),
            ("dof_diffuse", self.half_colour.texture()),
            ("dof_far", self.far_colour.texture()),
            ("dof_mask", self.masks.far_texture()),
            ("dof_near_mask", self.masks.near_texture()),
        ];
        if let Some(colours) = &self.colours {
            planes.extend([
                ("dof_near", colours.near_colour_texture()),
                ("dof_near_alpha", colours.near_alpha_texture()),
            ]);
        }
        planes
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        input: CameraViewInputs<'_>,
    ) -> Result<(), String> {
        if input.linear_depth_extent.contains(&0) {
            return Err("camera autofocus needs completed linear water depth".into());
        }
        let mut view_settings = input.effects.settings.clone();
        if let Some(alpha) = &input.alpha {
            view_settings.set_special_alpha(alpha.special_alpha);
        }
        let settings = &view_settings;
        let optics = settings.combine(input.effects.far_clip);
        let words = settings.words();
        let value = |i: usize| f32::from_bits(words[i]);
        let current = 1 - self.focus_index;
        //3150d0 also updates history when the combine does not use autofocus.
        {
            self.autofocus.record(
                device,
                queue,
                encoder,
                AutofocusParameters {
                    centre: optics.centre,
                    resolution_scale: [1.; 2],
                    texel: input.linear_depth_extent.map(|v| 1. / v as f32),
                    radius: value(30),
                    far_clip: input.effects.far_clip,
                    speeds: [value(31), value(32), value(33), value(34)],
                    local_delta: input.local_time[1],
                },
                input.linear_water_depth,
                &self.focus_views[self.focus_index],
                &self.focus_views[current],
            )?;
            self.focus_index = current;
        }
        let autofocus = &self.focus_views[self.focus_index];
        let region = |extent: [u32; 2]| [0, 0, extent[0], extent[1]];
        let high = region(self.half_extent);
        let low = region(self.half_extent.map(|v| v.div_ceil(2).max(2)));
        let near = region(self.half_extent.map(|v| v.div_ceil(2).max(4)));
        let engine_luminance = 139.26 * input.camera_exposure;
        if let Some(alpha) = input.alpha.as_ref().filter(|alpha|
            alpha.special_alpha || settings.far_sharp_distance() < input.effects.far_clip) {
            let material = input
                .resources
                .materials
                .resolve(settings.blur_variant(DofPass::Alpha))
                .ok_or("camera transparent material unavailable")?;
            let config = CameraAlphaConfig {
                extent: self.half_extent,
                colour_format: self.half_colour.texture().format(),
                depth_format: self.config.depth_format,
                reduction_shift: alpha.reduction_shift,
                host_source: true,
                material_state: material.state,
                stencil: input.effects.stencil_enabled,
                special_alpha: settings.special_alpha(),
            };
            if self
                .alpha_colours
                .as_ref()
                .is_none_or(|(old, _, _)| old != &config)
            {
                self.alpha_colours = Some((
                    config.clone(),
                    CameraAlphaColours::new(device, config)?,
                    CameraAlphaComposite::with_host_colour(
                        device,
                        self.config.host_format,
                        self.config.depth_format,
                        input.effects.stencil_enabled,
                        settings.special_alpha(),
                    )?,
                ));
            }
            //318390 uses reduced alpha depth. The later near/far task reuses
            //this mask target only after the alpha composite has consumed it.
            self.alpha_mask.record_region(
                device,
                queue,
                encoder,
                &CameraFuzzinessParameters {
                    pass: DofPass::Alpha,
                    near: optics.near,
                    alpha: [optics.far[1], optics.far[0], optics.far[2]],
                    far: optics.far,
                    inverse_z: input.inverse_z,
                    centre: optics.centre,
                    resolution_scale: [1.; 2],
                    autofocus: settings.autofocus_enabled(),
                },
                CameraFuzzinessInputs {
                    alpha_depth: input.alpha_depth,
                    near_mask: self.half_colour.output(),
                    autofocus,
                },
                self.masks.far_view(),
                Some((high, high)),
            );
            let (_, colours, composite) = self.alpha_colours.as_mut().unwrap();
            colours.record(
                device,
                queue,
                encoder,
                CameraAlphaInputs {
                    settings,
                    material,
                    kernel: &self.kernels[usize::from(material.filter)],
                    source: CameraColourSource::Host {
                        image: input.colour,
                        extent: self.config.extent,
                        adaptation: input.adaptation,
                        radiance_units: engine_luminance,
                    },
                    // The clear value follows the same host-to-game radiance
                    // conversion as fetched HDR pixels before range compression.
                    source_clear: alpha.clear_colour.map(|value| value / engine_luminance),
                    mask: self.masks.far_view(),
                    refraction: alpha.refraction,
                    depth_alpha: input.alpha_attachment,
                    dof_colour: self.far_colour.output(),
                    adapt_max: input.adapt_max,
                    engine_luminance,
                    white_level: input.white_level,
                    compressed: self.config.compressed,
                    texel_scale: [1.; 2],
                    resolution_scale: [1.; 2],
                    position_to_texture: [
                        1. / self.half_extent[0] as f32,
                        1. / self.half_extent[1] as f32,
                    ],
                    viewport: high,
                    scissor: high,
                    vertices: &self.vertices,
                    vertex_range: 0..3,
                    far_clip: input.effects.far_clip,
                },
            )?;
            composite.record(
                device,
                queue,
                encoder,
                AlphaCompositeInputs {
                    refraction: alpha.refraction,
                    blurred: self.far_colour.output(),
                    near_alpha: colours.alpha(),
                    destination: input.colour,
                    depth_alpha: alpha.full_depth_attachment,
                    full_extent: self.config.extent,
                    viewport: region(self.config.extent),
                    scissor: region(self.config.extent),
                    vertices: &self.vertices,
                    vertex_range: 0..3,
                    host: Some(AlphaHostInputs {
                        adaptation: input.adaptation,
                        radiance_units: engine_luminance,
                        white_level: input.white_level,
                        compressed: self.config.compressed,
                    }),
                },
            )?;
        } else {
            if settings.special_alpha() {
                return Err("special-alpha DoF requires completed opacity geometry".into());
            }
            //314b10's ordinary branch prepares dofDiv2 independently of the
            //316670 hrBufferDiv2 reduction. Disabled Far retains this image.
            self.far_colour.record(
                device,
                queue,
                encoder,
                input.colour,
                self.config.extent,
                input.camera_exposure,
                input.adapt_max,
                input.white_level,
                input.adaptation,
            )?;
        }
        self.masks.record(
            device,
            queue,
            encoder,
            CameraMaskInputs {
                settings,
                alpha_depth: input.alpha_depth,
                autofocus,
                inverse_z: input.inverse_z,
                resolution_scale: [1.; 2],
                near_mask_gain: input.effects.near_mask_gain,
                far_clip: input.effects.far_clip,
                near_viewport: near,
                near_scissor: near,
                far_viewport: high,
                far_scissor: high,
            },
        );
        if input.effects.opaque_enabled {
            let materials = [DofPass::Near, DofPass::Far].map(|pass| {
                input
                    .resources
                    .materials
                    .resolve(settings.blur_variant(pass))
                    .ok_or_else(|| format!("camera {pass:?} material unavailable"))
            });
            let [near_material, far_material] = materials;
            let materials = [near_material?, far_material?];
            let config = CameraColourConfig {
                extent: self.half_extent,
                hdr_format: self.half_colour.texture().format(),
                depth_format: self.config.depth_format,
                stencil: input.effects.stencil_enabled,
                host_source: true,
                material_states: materials.map(|m| m.state),
            };
            if self
                .colours
                .as_ref()
                .is_none_or(|owner| !owner.matches(&config))
            {
                self.colours = Some(CameraColours::new(device, config)?);
            }
            self.colours.as_mut().unwrap().record(
                device,
                queue,
                encoder,
                CameraColourInputs {
                    settings,
                    materials,
                    kernels: materials.map(|m| &self.kernels[usize::from(m.filter)]),
                    source: CameraColourSource::Host {
                        image: input.colour,
                        extent: self.config.extent,
                        adaptation: input.adaptation,
                        radiance_units: engine_luminance,
                    },
                    half_colour: self.half_colour.output(),
                    high_mask: self.masks.far_view(),
                    //Near/Far do not read the refraction slot. Use their actual
                    //colour input in that inactive binding, not a fabricated image.
                    refraction: input.colour,
                    depth_stencil: input.alpha_attachment,
                    autofocus,
                    far_colour: self.far_colour.output(),
                    projection: input.projection,
                    near_clip: input.effects.near_clip,
                    far_clip: input.effects.far_clip,
                    adapt_max: input.adapt_max,
                    engine_luminance,
                    white_level: input.white_level,
                    compressed: self.config.compressed,
                    texel_scale: [1.; 2],
                    resolution_scale: [1.; 2],
                    position_to_texture: [
                        1. / self.half_extent[0] as f32,
                        1. / self.half_extent[1] as f32,
                    ],
                    high_viewport: high,
                    high_scissor: high,
                    low_viewport: low,
                    low_scissor: low,
                    vertices: &self.vertices,
                    vertex_range: 0..3,
                },
            )?;
        } else {
            //313980's simple path retains hrBufferDiv2 and uses `_black`
            //for near alpha; no altered material switches are needed.
            self.colours = None;
            self.half_colour.record(
                device,
                queue,
                encoder,
                input.colour,
                self.config.extent,
                input.camera_exposure,
                input.adapt_max,
                input.white_level,
                input.adaptation,
            )?;
        }
        let (near_alpha, near_colour) = self
            .colours
            .as_ref()
            .map_or((input.black, self.half_colour.output()), |c| {
                (c.near_alpha(), c.near_colour())
            });
        self.combine.record(
            device,
            queue,
            encoder,
            &CameraCombineParameters {
                optics,
                scattering_colour: input.scattering_colour,
                white_level: input.white_level,
                resolution_scale: [1.; 2],
                inverse_z_time: [
                    input.inverse_z[0],
                    input.inverse_z[1],
                    input.inverse_z[2],
                    input.local_time[0],
                ],
                host_radiance_units: Some(engine_luminance),
                compressed: self.config.compressed,
                exposure_entry: 0,
            },
            CameraCombineInputs {
                images: [
                    input.colour,
                    input.depth,
                    self.far_colour.output(),
                    near_alpha,
                    near_colour,
                    self.half_colour.output(),
                    autofocus,
                ],
                adaptation: input.adaptation,
            },
            input.destination,
            self.config.extent,
        )
    }
}
