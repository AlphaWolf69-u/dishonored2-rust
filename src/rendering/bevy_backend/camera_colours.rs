//! 316670's ordered opaque camera-blur producers. Alpha/refraction preparation
//! and the final camera combine remain the surrounding view owner's work.
use super::{
    camera_blur::*,
    camera_stencil::*,
    colour_downscale::{ColourDownscalePass, DownscaleParameters},
};
use crate::{
    assets::camera_blur::BlurMaterial,
    rendering::{
        camera_effects::CameraSettings,
        colour_downscale::{DownscaleInputs, DownscaleMode},
        depth_of_field::DofPass,
    },
};
use bevy::render::{
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraColourConfig {
    /// Allocated dofDiv2 dimensions, independently of the active view rectangle.
    pub extent: [u32; 2],
    pub hdr_format: TextureFormat,
    pub depth_format: TextureFormat,
    pub stencil: bool,
    pub host_source: bool,
    /// Actual selected Near/Far material states before the stencil override.
    pub material_states: [crate::render_state::RenderState; 2],
}
pub enum CameraColourSource<'a> {
    Game {
        image: &'a TextureView,
        extent: [u32; 2],
    },
    Host {
        image: &'a TextureView,
        extent: [u32; 2],
        adaptation: &'a Buffer,
        radiance_units: f32,
    },
}
pub struct CameraColourInputs<'a> {
    pub settings: &'a CameraSettings,
    pub materials: [&'a BlurMaterial; 2],
    pub kernels: [&'a BlurKernelBuffer; 2],
    pub source: CameraColourSource<'a>,
    /// The real hrBufferDiv2 target, written here before either blur pass.
    pub half_colour: &'a TextureView,
    pub high_mask: &'a TextureView,
    pub refraction: &'a TextureView,
    pub depth_stencil: &'a TextureView,
    pub autofocus: &'a TextureView,
    pub far_colour: &'a TextureView,
    pub projection: [[f32; 4]; 4],
    pub near_clip: f32,
    pub far_clip: f32,
    pub adapt_max: f32,
    pub engine_luminance: f32,
    pub white_level: f32,
    pub compressed: bool,
    /// Native view +78c0/+78c4 numerator for high/low texel-size uniforms.
    pub texel_scale: [f32; 2],
    pub resolution_scale: [f32; 2],
    pub position_to_texture: [f32; 2],
    pub high_viewport: [u32; 4],
    pub high_scissor: [u32; 4],
    pub low_viewport: [u32; 4],
    pub low_scissor: [u32; 4],
    /// Actual fullscreen mesh, including its view texture coordinates.
    pub vertices: &'a Buffer,
    pub vertex_range: std::ops::Range<u32>,
}
struct ColourTarget {
    texture: Texture,
    view: TextureView,
}
pub struct CameraColours {
    config: CameraColourConfig,
    low_extent: [u32; 2],
    low_colour: ColourTarget,
    low_mask: ColourTarget,
    near_colour: ColourTarget,
    near_alpha: ColourTarget,
    downscale: [ColourDownscalePass; 3],
    blur: [CameraBlurPass; 2],
    stencil: Option<[CameraStencilPass; 3]>,
}
impl CameraColours {
    pub fn new(device: &RenderDevice, config: CameraColourConfig) -> Result<Self, String> {
        if config.extent.contains(&0) {
            return Err("camera colours require nonempty targets".into());
        }
        if !config.depth_format.has_depth_aspect()
            || (config.stencil && !config.depth_format.has_stencil_aspect())
        {
            return Err("camera colour attachment lacks the required depth/stencil aspects".into());
        }
        let low_extent = config.extent.map(|v| v.div_ceil(2).max(2));
        let target = |label, extent: [u32; 2], format| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::TEXTURE_BINDING
                    | TextureUsages::RENDER_ATTACHMENT
                    | TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&Default::default());
            ColourTarget { texture, view }
        };
        let hdr = ColourDownscalePass::new_with_source_geometry(
            device,
            config.hdr_format,
            config.host_source,
        );
        let low = if !config.host_source && config.hdr_format == TextureFormat::Rg11b10Ufloat {
            hdr.sibling()
        } else {
            ColourDownscalePass::new_with_source_geometry(
                device,
                TextureFormat::Rg11b10Ufloat,
                false,
            )
        };
        let colour = |index, pass, format, alpha| {
            CameraBlurPass::new(
                device,
                CameraBlurConfig {
                    pass,
                    colour_format: format,
                    alpha_format: alpha,
                    depth_format: Some(config.depth_format),
                    state: if config.stencil {
                        crate::render_state::camera_blur_colour_state(
                            config.material_states[index],
                            index == 1,
                        )
                    } else {
                        config.material_states[index]
                    },
                },
            )
        };
        let blur = [
            colour(
                0,
                DofPass::Near,
                TextureFormat::Rg11b10Ufloat,
                Some(TextureFormat::R8Unorm),
            )?,
            colour(1, DofPass::Far, config.hdr_format, None)?,
        ];
        let stencil = if config.stencil {
            Some([
                CameraStencilPass::new(
                    device,
                    CameraStencilKind::Near,
                    config.depth_format,
                    crate::render_state::camera_blur_stencil_state(false),
                )?,
                CameraStencilPass::new(
                    device,
                    CameraStencilKind::Far,
                    config.depth_format,
                    crate::render_state::camera_blur_stencil_state(true),
                )?,
                CameraStencilPass::new(
                    device,
                    CameraStencilKind::Autofocus,
                    config.depth_format,
                    crate::render_state::camera_blur_stencil_state(true),
                )?,
            ])
        } else {
            None
        };
        Ok(Self {
            low_colour: target(
                "camera quarter colour",
                low_extent,
                TextureFormat::Rg11b10Ufloat,
            ),
            low_mask: target("camera quarter mask", low_extent, TextureFormat::Rg8Unorm),
            near_colour: target(
                "camera near colour",
                config.extent,
                TextureFormat::Rg11b10Ufloat,
            ),
            near_alpha: target("camera near alpha", config.extent, TextureFormat::R8Unorm),
            downscale: [
                hdr,
                low,
                ColourDownscalePass::new_with_source_geometry(
                    device,
                    TextureFormat::Rg8Unorm,
                    false,
                ),
            ],
            blur,
            stencil,
            config,
            low_extent,
        })
    }
    pub fn matches(&self, config: &CameraColourConfig) -> bool {
        self.config == *config
    }
    pub fn near_colour(&self) -> &TextureView {
        &self.near_colour.view
    }
    pub fn near_alpha(&self) -> &TextureView {
        &self.near_alpha.view
    }
    pub fn near_colour_texture(&self) -> &Texture {
        &self.near_colour.texture
    }
    pub fn near_alpha_texture(&self) -> &Texture {
        &self.near_alpha.texture
    }
    pub fn low_colour_texture(&self) -> &Texture {
        &self.low_colour.texture
    }
    pub fn low_mask_texture(&self) -> &Texture {
        &self.low_mask.texture
    }

    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        inputs: CameraColourInputs<'_>,
    ) -> Result<(), String> {
        let (source, source_extent, adaptation) = match inputs.source {
            CameraColourSource::Game { image, extent } => (image, extent, None),
            CameraColourSource::Host {
                image,
                extent,
                adaptation,
                radiance_units,
            } => {
                if !radiance_units.is_finite() || radiance_units <= 0. {
                    return Err("camera host radiance units must be positive and finite".into());
                }
                (image, extent, Some((adaptation, radiance_units)))
            }
        };
        if adaptation.is_some() != self.config.host_source {
            return Err("camera HDR source differs from its prepared adapter".into());
        }
        for (index, pass) in [DofPass::Near, DofPass::Far].into_iter().enumerate() {
            if inputs.materials[index].pass != pass
                || inputs.materials[index].state != self.config.material_states[index]
                || u32::from(inputs.materials[index].filter) != inputs.kernels[index].kind()
            {
                return Err(
                    "camera material/kernel differs from its prepared colour pipeline".into(),
                );
            }
        }
        if source.id() == inputs.half_colour.id()
            || [
                source,
                inputs.half_colour,
                inputs.high_mask,
                inputs.refraction,
                inputs.depth_stencil,
                inputs.autofocus,
            ]
            .iter()
            .any(|v| v.id() == inputs.far_colour.id())
        {
            return Err("camera colour sources alias a written target".into());
        }
        let mut hdr = DownscaleParameters::new(
            DownscaleInputs::new(source_extent, [0.; 3])?,
            DownscaleMode::from_selector(5, false).unwrap(),
            inputs.adapt_max,
            inputs.engine_luminance,
            inputs.white_level,
            inputs.compressed,
        );
        if let Some((_, units)) = adaptation {
            hdr.limits.w = units;
        }
        self.downscale[0].record_with_geometry(
            device,
            queue,
            encoder,
            hdr,
            source,
            inputs.half_colour,
            inputs.high_viewport,
            inputs.high_scissor,
            inputs.vertices,
            inputs.vertex_range.clone(),
            adaptation.map(|v| v.0),
        );
        let parameters = |mode| {
            DownscaleParameters::new(
                DownscaleInputs::new(self.config.extent, [0.; 3]).unwrap(),
                mode,
                inputs.adapt_max,
                inputs.engine_luminance,
                inputs.white_level,
                inputs.compressed,
            )
        };
        self.downscale[1].record_with_geometry(
            device,
            queue,
            encoder,
            parameters(DownscaleMode::from_selector(3, false).unwrap()),
            inputs.half_colour,
            &self.low_colour.view,
            inputs.low_viewport,
            inputs.low_scissor,
            inputs.vertices,
            inputs.vertex_range.clone(),
            None,
        );
        self.downscale[2].record_with_geometry(
            device,
            queue,
            encoder,
            parameters(DownscaleMode::from_selector(0, true).unwrap()),
            inputs.high_mask,
            &self.low_mask.view,
            inputs.low_viewport,
            inputs.low_scissor,
            inputs.vertices,
            inputs.vertex_range.clone(),
            None,
        );
        // 316670 clears both near outputs even when the distance disables Near.
        for target in [&self.near_colour.view, &self.near_alpha.view] {
            let _clear = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("clear camera near outputs"),
                color_attachments: &[Some(RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(Default::default()),
                        store: StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        let optics = inputs.settings.combine(inputs.far_clip);
        for (index, enabled) in [
            inputs.settings.near_sharp_distance() < inputs.far_clip,
            inputs.settings.far_sharp_distance() < inputs.far_clip,
        ]
        .into_iter()
        .enumerate()
        {
            if !enabled {
                continue;
            }
            if let Some(stencil) = &mut self.stencil {
                let (classifier, source) = if index == 0 {
                    (0, CameraStencilSource::Near(&self.low_mask.view))
                } else if inputs.settings.autofocus_enabled() {
                    (
                        2,
                        CameraStencilSource::Autofocus {
                            mask: inputs.high_mask,
                            focus: inputs.autofocus,
                        },
                    )
                } else {
                    (1, CameraStencilSource::Far(inputs.high_mask))
                };
                stencil[classifier].record(
                    device,
                    queue,
                    encoder,
                    &CameraStencilParameters {
                        projection: inputs.projection,
                        near_clip: inputs.near_clip,
                        clip_distance: inputs.settings.far_sharp_distance(),
                    },
                    source,
                    CameraStencilTarget {
                        attachment: inputs.depth_stencil,
                        viewport: inputs.high_viewport,
                        scissor: inputs.high_scissor,
                        stencil_reference: if index == 0 { 4 } else { 8 },
                    },
                    inputs.vertices,
                    inputs.vertex_range.clone(),
                )?;
            }
            let parameters = CameraBlurParameters {
                target_size: [
                    inputs.texel_scale[0] / self.config.extent[0] as f32,
                    inputs.texel_scale[1] / self.config.extent[1] as f32,
                    inputs.texel_scale[0] / self.low_extent[0] as f32,
                    inputs.texel_scale[1] / self.low_extent[1] as f32,
                ],
                fringing: inputs.settings.fringing(),
                bokeh_size: inputs.settings.bokeh_size(),
                vignette: optics.vignette,
                centre: optics.centre,
                resolution_scale: inputs.resolution_scale,
                position_to_texture: inputs.position_to_texture,
                use_fringing: inputs.materials[index].fringing,
                cat_eye: inputs.materials[index].cat_eye,
            };
            self.blur[index].record(
                device,
                queue,
                encoder,
                &parameters,
                CameraBlurInputs {
                    images: [
                        inputs.refraction,
                        inputs.high_mask,
                        &self.low_mask.view,
                        inputs.half_colour,
                        &self.low_colour.view,
                    ],
                    kernel: inputs.kernels[index],
                },
                CameraBlurTargets {
                    colour: if index == 0 {
                        &self.near_colour.view
                    } else {
                        inputs.far_colour
                    },
                    alpha: (index == 0).then_some(&self.near_alpha.view),
                    depth_stencil: Some(RenderPassDepthStencilAttachment {
                        view: inputs.depth_stencil,
                        depth_ops: Some(Operations {
                            load: LoadOp::Load,
                            store: StoreOp::Store,
                        }),
                        stencil_ops: self.config.depth_format.has_stencil_aspect().then_some(
                            Operations {
                                load: LoadOp::Load,
                                store: StoreOp::Store,
                            },
                        ),
                    }),
                    stencil_reference: if self.config.stencil {
                        if index == 0 {
                            4
                        } else {
                            8
                        }
                    } else {
                        0
                    },
                    viewport: inputs.high_viewport,
                    scissor: inputs.high_scissor,
                },
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path="camera_colours_tests.rs"]
mod tests;
