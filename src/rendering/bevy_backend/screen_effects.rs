//! Authored player screen quads, after camera effects and before exposure.
mod passes;
mod sources;
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
    underwater: bool,
    age: f32,
    seed: u32,
    frame: Option<u32>,
    quads: Vec<PlayingQuad>,
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
    fn start(
        &mut self,
        definition: &sources::EffectDefinition,
        leaving: bool,
    ) -> Result<(), String> {
        for group in &definition.groups {
            let offsets = crate::gameplay::effects::groups::sample_offsets(
                &group.timing,
                self.age,
                &mut self.seed,
            );
            for quad in &group.quads {
                self.quads.push(PlayingQuad {
                    material: quad.material,
                    stage: quad.stage,
                    leaving,
                    blend: EnvironmentBlendEffect::new(
                        quad.blend.clone(),
                        offsets,
                        1,
                        &mut self.seed,
                    )?,
                });
            }
        }
        Ok(())
    }
    /// 821350's enter/leave transition and 21f820's retirement-before-update.
    fn advance(
        &mut self,
        sources: &ScreenEffectSources,
        frame: u32,
        delta: f32,
        underwater: bool,
    ) -> Result<Vec<QuadDraw>, String> {
        if self.frame != Some(frame) {
            self.age += delta;
            self.quads.retain(|q| q.blend.active);
            if underwater != self.underwater {
                // Stop the opposite FX instance before starting its replacement.
                for quad in &mut self.quads {
                    if quad.leaving == underwater {
                        quad.blend.stop(self.age);
                    }
                }
                self.start(
                    if underwater {
                        &sources.underwater
                    } else {
                        &sources.leave_water
                    },
                    !underwater,
                )?;
                self.underwater = underwater;
                info!("Player screen distortion transition: underwater={underwater}");
            }
            for quad in &mut self.quads {
                quad.blend.update(self.age, 1, &mut self.seed)?;
            }
            self.frame = Some(frame);
        }
        let mut draws = Vec::new();
        for stage in [
            ScreenQuadStage::MotionVectorBegin,
            ScreenQuadStage::MotionVectorEnd,
        ] {
            // 31da70's view publication has four slots for each stage.
            draws.extend(
                self.quads
                    .iter()
                    .filter(|q| q.stage == stage && q.blend.blend != 0.)
                    .take(4)
                    .map(|q| QuadDraw {
                        material: q.material,
                        blend: q.blend.blend,
                    }),
            );
        }
        Ok(draws)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        sources: &ScreenEffectSources,
        water: &super::water_scene::WaterFrame,
        underwater: bool,
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
        let draws = self.advance(
            sources,
            water.matrices.frame,
            water.delta_seconds,
            underwater,
        )?;
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
            &draws,
            water.local_time[0],
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
