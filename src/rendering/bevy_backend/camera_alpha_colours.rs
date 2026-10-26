//! 317890's transparent DoF preparation, before 313730's HDR composite.
use super::{
    camera_blur::*,
    camera_colours::CameraColourSource,
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
use bevy::{
    prelude::default,
    render::{
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CameraAlphaConfig {
    pub extent: [u32; 2],
    pub colour_format: TextureFormat,
    pub depth_format: TextureFormat,
    pub reduction_shift: u32,
    pub host_source: bool,
    pub material_state: crate::render_state::RenderState,
    pub stencil: bool,
    pub special_alpha: bool,
}
pub struct CameraAlphaInputs<'a> {
    pub settings: &'a CameraSettings,
    pub material: &'a BlurMaterial,
    pub kernel: &'a BlurKernelBuffer,
    pub source: CameraColourSource<'a>,
    pub source_clear: [f32; 3],
    pub mask: &'a TextureView,
    pub refraction: &'a TextureView,
    pub depth_alpha: &'a TextureView,
    pub dof_colour: &'a TextureView,
    pub adapt_max: f32,
    pub engine_luminance: f32,
    pub white_level: f32,
    pub compressed: bool,
    pub texel_scale: [f32; 2],
    pub resolution_scale: [f32; 2],
    pub position_to_texture: [f32; 2],
    pub viewport: [u32; 4],
    pub scissor: [u32; 4],
    pub vertices: &'a Buffer,
    pub vertex_range: std::ops::Range<u32>,
    pub far_clip: f32,
}
struct Target {
    _texture: Texture,
    view: TextureView,
}
pub struct CameraAlphaColours {
    config: CameraAlphaConfig,
    low_extent: [u32; 2],
    low_colour: Target,
    low_mask: Target,
    alpha: Target,
    downscale: [ColourDownscalePass; 3],
    blur: CameraBlurPass,
    reference: u32,
}
impl CameraAlphaColours {
    pub fn new(device: &RenderDevice, config: CameraAlphaConfig) -> Result<Self, String> {
        if config.extent.contains(&0) || !(1..=2).contains(&config.reduction_shift) {
            return Err(
                "transparent DoF requires nonempty extents and admitted reduction quality".into(),
            );
        }
        let divisor = 1u32 << config.reduction_shift;
        let low_extent = config.extent.map(|v| v.div_ceil(divisor).max(divisor));
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
            let view = texture.create_view(&default());
            Target {
                _texture: texture,
                view,
            }
        };
        let (state, reference) = crate::render_state::camera_alpha_blur_state(
            config.material_state,
            config.stencil,
            config.special_alpha,
        );
        let blur = CameraBlurPass::new(
            device,
            CameraBlurConfig {
                pass: DofPass::Alpha,
                colour_format: config.colour_format,
                alpha_format: Some(TextureFormat::R8Unorm),
                depth_format: Some(config.depth_format),
                state,
            },
        )?;
        Ok(Self {
            low_extent,
            low_colour: target(
                "transparent DoF reduced colour",
                low_extent,
                TextureFormat::Rg11b10Ufloat,
            ),
            low_mask: target(
                "transparent DoF reduced mask",
                low_extent,
                TextureFormat::Rg8Unorm,
            ),
            alpha: target(
                "transparent DoF composite alpha",
                config.extent,
                TextureFormat::R8Unorm,
            ),
            downscale: [
                ColourDownscalePass::new_with_source_geometry(
                    device,
                    config.colour_format,
                    config.host_source,
                ),
                ColourDownscalePass::new_with_source_geometry(
                    device,
                    TextureFormat::Rg11b10Ufloat,
                    false,
                ),
                ColourDownscalePass::new_with_source_geometry(
                    device,
                    TextureFormat::Rg8Unorm,
                    false,
                ),
            ],
            blur,
            reference,
            config,
        })
    }
    pub fn matches(&self, config: &CameraAlphaConfig) -> bool {
        self.config == *config
    }
    pub fn alpha(&self) -> &TextureView {
        &self.alpha.view
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        inputs: CameraAlphaInputs<'_>,
    ) -> Result<(), String> {
        if inputs.material.pass != DofPass::Alpha
            || inputs.material.state != self.config.material_state
            || u32::from(inputs.material.filter) != inputs.kernel.kind()
        {
            return Err("transparent DoF material/kernel differs from prepared pipeline".into());
        }
        let (source, extent, adaptation) = match inputs.source {
            CameraColourSource::Game { image, extent } => (image, extent, None),
            CameraColourSource::Host {
                image,
                extent,
                adaptation,
                radiance_units,
            } => {
                if !radiance_units.is_finite() || radiance_units <= 0. {
                    return Err("invalid transparent DoF host radiance units".into());
                }
                (image, extent, Some((adaptation, radiance_units)))
            }
        };
        if adaptation.is_some() != self.config.host_source {
            return Err("transparent DoF HDR source adapter differs".into());
        }
        if [source, inputs.mask, inputs.refraction, inputs.depth_alpha]
            .iter()
            .any(|v| v.id() == inputs.dof_colour.id())
        {
            return Err("transparent DoF source aliases its colour output".into());
        }
        let mut high = DownscaleParameters::new(
            DownscaleInputs::new(extent, inputs.source_clear)?,
            DownscaleMode::from_selector(4, false).unwrap(),
            inputs.adapt_max,
            inputs.engine_luminance,
            inputs.white_level,
            inputs.compressed,
        );
        if let Some((_, units)) = adaptation {
            high.limits.w = units;
        }
        self.downscale[0].record_with_geometry(
            device,
            queue,
            encoder,
            high,
            source,
            inputs.dof_colour,
            inputs.viewport,
            inputs.scissor,
            inputs.vertices,
            inputs.vertex_range.clone(),
            adaptation.map(|v| v.0),
        );
        let divide = |region: [u32; 4]| region.map(|v| v / (1 << self.config.reduction_shift));
        let low_viewport = divide(inputs.viewport);
        let low_scissor = divide(inputs.scissor);
        if low_viewport[2] == 0
            || low_viewport[3] == 0
            || low_scissor[2] == 0
            || low_scissor[3] == 0
        {
            return Err("transparent DoF reduced view is empty".into());
        }
        for (index, selector, rg, source, destination) in [
            (1, 3, false, inputs.dof_colour, &self.low_colour.view),
            (2, 0, true, inputs.mask, &self.low_mask.view),
        ] {
            let parameters = DownscaleParameters::new(
                DownscaleInputs::new(self.config.extent, [0.; 3])?,
                DownscaleMode::from_selector(selector, rg).unwrap(),
                inputs.adapt_max,
                inputs.engine_luminance,
                inputs.white_level,
                if index == 1 { inputs.compressed } else { false },
            );
            self.downscale[index].record_with_geometry(
                device,
                queue,
                encoder,
                parameters,
                source,
                destination,
                low_viewport,
                low_scissor,
                inputs.vertices,
                inputs.vertex_range.clone(),
                None,
            );
        }
        // Alpha samples only `blurred`, not `diffuse`. Bind the same real
        // reduced colour in the inactive diffuse slot, never its output.
        let optics = inputs.settings.combine(inputs.far_clip);
        self.blur.record(
            device,
            queue,
            encoder,
            &CameraBlurParameters {
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
                use_fringing: inputs.material.fringing,
                cat_eye: inputs.material.cat_eye,
            },
            CameraBlurInputs {
                images: [
                    inputs.refraction,
                    inputs.mask,
                    &self.low_mask.view,
                    &self.low_colour.view,
                    &self.low_colour.view,
                ],
                kernel: inputs.kernel,
            },
            CameraBlurTargets {
                colour: inputs.dof_colour,
                alpha: Some(&self.alpha.view),
                depth_stencil: Some(RenderPassDepthStencilAttachment {
                    view: inputs.depth_alpha,
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
                stencil_reference: self.reference,
                viewport: inputs.viewport,
                scissor: inputs.scissor,
            },
        )?;
        Ok(())
    }
}
