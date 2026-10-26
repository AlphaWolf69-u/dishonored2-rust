//! Authored player screen quads, after camera effects and before exposure.
mod passes;
mod particles;
mod sources;
mod state;
pub(super) use state::ScreenEffectState;
pub(super) use particles::ParticleDrawContext;
use crate::{
    assets::fx::screen_quad::ScreenQuadStage,
    gameplay::effects::environment::EnvironmentBlendEffect,
};
use bevy::{
    prelude::*,
    render::{
        extract_resource::ExtractResource,
        render_resource::*,
        renderer::{RenderDevice, RenderQueue},
    },
};
pub use sources::ScreenEffectSources;
use std::sync::Arc;

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct ScreenEffectMaterials(pub Option<Arc<ScreenEffectSources>>);

pub struct QuadDraw {
    material: usize,
    blend: f32,
}
struct PlayingQuad {
    material: usize,
    stage: ScreenQuadStage,
    leaving: bool,
    blend: EnvironmentBlendEffect,
}
#[derive(Default)]
pub(super) struct ScreenEffects {
    gpu: Option<(
        ScreenConfig,
        passes::DistortionPasses,
        super::water_hdr::WaterHdr,
    )>,
    completed: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct ScreenConfig {
    extent: [u32; 2],
    downsample: u32,
    compressed: bool,
    format: TextureFormat,
}
impl ScreenEffects {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        sources: &ScreenEffectSources,
        draws: &[QuadDraw],
        local_time: f32,
        colour: &TextureView,
        depth: &TextureView,
        extent: [u32; 2],
        downsample: u32,
        compressed: bool,
        format: TextureFormat,
        camera_exposure: f32,
        white_level: f32,
        adaptation: &Buffer,
        inverse_z: [f32; 3],
    ) -> Result<(), String> {
        self.completed = false;
        if draws.is_empty() {
            return Ok(());
        }
        let config = ScreenConfig {
            extent,
            downsample,
            compressed,
            format,
        };
        if self.gpu.as_ref().is_none_or(|g| g.0 != config) {
            let hdr = super::water_hdr::WaterHdr::new(device, queue, extent, compressed, format)?;
            let native_format = if compressed {
                TextureFormat::Rg11b10Ufloat
            } else {
                TextureFormat::Rgba16Float
            };
            self.gpu = Some((
                config,
                passes::DistortionPasses::new(
                    device,
                    queue,
                    sources,
                    extent,
                    downsample,
                    native_format,
                )?,
                hdr,
            ));
        }
        let (_, passes, hdr) = self.gpu.as_mut().unwrap();
        passes.record_vectors(
            device,
            queue,
            encoder,
            sources,
            draws,
            local_time,
            depth,
            inverse_z,
        )?;
        let hdr = hdr.prepare(
            device,
            queue,
            encoder,
            colour,
            extent,
            camera_exposure,
            white_level,
            adaptation,
        )?;
        passes.record_resolve(device, encoder, hdr.source(), hdr.colour());
        hdr.finish_source(encoder);
        self.completed = true;
        Ok(())
    }
    pub(super) fn capture_planes(&self) -> Vec<(&'static str, &Texture)> {
        if !self.completed {
            return Vec::new();
        }
        self.gpu.as_ref().map_or_else(Vec::new, |(_, p, _)| {
            vec![("screen_distortion_vectors", &p.vectors)]
        })
    }
}
