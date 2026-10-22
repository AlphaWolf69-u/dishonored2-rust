//! Prepared lens-frame transport. Per-view image production is connected by
//! the display owner, which also supplies coordinates and host radiance units.
use bevy::prelude::{Handle, Image};
#[derive(Clone)]
pub struct LensFrame {
    pub black: Handle<Image>,
    pub camera_axis: f32,
    pub white_level: f32,
    pub settings: crate::platform::game_settings::PostProcessSettings,
    pub veil_size: f32,
    pub veil_strength: f32,
    pub dirt_strength: f32,
    pub use_default_dirt: bool,
    pub threshold: [f32; 2],
    pub ghost: [f32; 4],
    pub streak: [f32; 4],
    pub blur_radius: u32,
    pub ghost_samples: u32,
    pub dirt: crate::rendering::lens_dirt::LensDirtBlend<Handle<Image>>,
    pub profile: crate::rendering::environment_tracks::LensProfileBlend<Handle<Image>>,
}

use super::{
    bloom::{BloomChain, BloomViewParameters},
    lens_composite::{LensCompositeInputs, LensCompositeParameters},
    luminance::{LuminancePipelines, LuminancePyramid},
    tone_mapping::{ToneMappingParameters, ToneMappingPass},
};
use bevy::render::{
    render_asset::RenderAssets,
    render_resource::*,
    renderer::{RenderDevice, RenderQueue},
    texture::GpuImage,
};

pub(super) struct LensImages<'a> {
    dirt: [&'a TextureView; 2],
    black: &'a TextureView,
}
impl<'a> LensImages<'a> {
    pub fn resolve(frame: &LensFrame, images: &'a RenderAssets<GpuImage>) -> Result<Self, String> {
        if !frame.settings.skip_lens_flares {
            return Err("enabled lens flares await their full HDR producer".into());
        }
        if !frame.camera_axis.is_finite() {
            return Err("lens camera axis is invalid".into());
        }
        if frame.settings.hdr_buffer_format != 1
            && (!frame.white_level.is_finite() || frame.white_level <= 0.0)
        {
            return Err("compressed lens HDR requires a positive white level".into());
        }
        let image = |handle: &Handle<Image>| {
            images
                .get(handle)
                .map(|v| &v.texture_view)
                .ok_or_else(|| "lens image awaits GPU preparation".to_owned())
        };
        Ok(Self {
            dirt: [image(&frame.dirt.from)?, image(&frame.dirt.to)?],
            black: image(&frame.black)?,
        })
    }
}
pub(super) struct LensView {
    display: ToneMappingPass,
    bloom: Option<BloomChain>,
    bloom_views: Vec<BloomViewParameters>,
    key: Option<([u32; 2], i32, i32, i32, u32)>,
}
impl LensView {
    pub fn new(device: &RenderDevice, format: TextureFormat) -> Self {
        Self {
            display: ToneMappingPass::new_with_lens(device, format),
            bloom: None,
            bloom_views: Vec::new(),
            key: None,
        }
    }
    pub fn prepare(
        &mut self,
        device: &RenderDevice,
        frame: &LensFrame,
        pyramid: &LuminancePyramid,
        full: [u32; 2],
    ) -> Result<(), String> {
        let settings = frame.settings;
        let key = (
            full,
            settings.downsample,
            settings.source_mip,
            settings.target_mip,
            settings.flags(),
        );
        if self.key == Some(key) {
            return Ok(());
        }
        let source = u32::try_from(settings.source_mip)
            .map_err(|_| "negative bloom source mip is not yet supported")?;
        let mips: Vec<_> = (0..pyramid.level_count())
            .map(|i| pyramid.extent(i).unwrap().to_array())
            .collect();
        let schedule = crate::rendering::bloom::BloomSchedule::new(
            &mips,
            source,
            settings.target_mip,
            settings.flags(),
            [1.0, 1.0],
        )?;
        self.bloom_views = schedule
            .as_ref()
            .map(|s| {
                s.draws
                    .iter()
                    .map(|d| {
                        BloomViewParameters::full_target(d.target_extent, [1.0, 1.0], [0.0, 0.0])
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.bloom = schedule.map(|s| BloomChain::new(device, s)).transpose()?;
        self.key = Some(key);
        Ok(())
    }
    pub fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        frame: &LensFrame,
        images: LensImages<'_>,
        pyramid: &mut LuminancePyramid,
        programs: &LuminancePipelines,
        full: [u32; 2],
        camera_exposure: f32,
        parameters: &ToneMappingParameters,
        hdr: &TextureView,
        cube: &TextureView,
        exposure: &Buffer,
        destination: &TextureView,
    ) -> Result<(), String> {
        let bloom_enabled = self.bloom.is_some();
        let mut bloom = images.black;
        if let Some(chain) = &mut self.bloom {
            pyramid.ensure_levels(
                device,
                queue,
                encoder,
                programs,
                hdr,
                camera_exposure,
                pyramid.level_count(),
            );
            chain.record(
                device,
                queue,
                encoder,
                pyramid.view(chain.schedule().source_mip).unwrap(),
                &self.bloom_views,
                frame.veil_size,
            )?;
            bloom = chain.output();
        }
        let lens = LensCompositeParameters {
            viewport: [1.0, 1.0, 0.0, 0.0],
            pixel: [
                1.0 / full[0] as f32,
                1.0 / full[1] as f32,
                frame.camera_axis,
                139.26 * camera_exposure,
            ],
            controls: [
                frame.veil_strength,
                frame.dirt_strength,
                frame.dirt.weight,
                frame.white_level,
            ],
            streak: frame.streak,
            tint: [1.0, 1.0, 1.0, 0.0],
            flags: [
                u32::from(bloom_enabled),
                0,
                u32::from(frame.use_default_dirt),
                0,
            ],
            // Primary host HDR is uncompressed; the source adapter encodes it
            // when necessary. The native bloom image is expanded separately.
            options: [
                if frame.settings.hdr_buffer_format != 1 {
                    3
                } else {
                    0
                },
                0,
                0,
                0,
            ],
        };
        self.display.record_with_lens(
            device,
            queue,
            encoder,
            parameters,
            hdr,
            cube,
            exposure,
            destination,
            &LensCompositeInputs {
                parameters: &lens,
                bloom,
                dirt_from: images.dirt[0],
                dirt_to: images.dirt[1],
                flares: images.black,
            },
        )
    }
    #[cfg(test)]
    pub(super) fn bloom_output(&self) -> Option<&TextureView> {
        self.bloom.as_ref().map(|v| v.output())
    }
    pub(super) fn bloom_texture(&self) -> Option<&Texture> {
        self.bloom.as_ref().map(|v| v.output_texture())
    }
}
