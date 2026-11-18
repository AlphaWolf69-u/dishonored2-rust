//! Authored player screen quads, after camera effects and before exposure.
pub(super) mod passes;
pub(super) mod dark_vision_wave;
mod fullscreen;
mod sources;
mod state;
pub(super) use super::particle_system::ParticleDrawContext;
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
pub(super) use state::ScreenEffectState;
pub(crate) use state::ScreenLifecycleSnapshot;
use std::sync::Arc;

fn trace_fullscreen_distortion() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("DH2_DEBUG_ACTION_SCENARIO").as_deref() == Ok("blink"))
}

#[derive(Resource, Clone, Default, ExtractResource)]
pub struct ScreenEffectMaterials(pub Option<Arc<ScreenEffectSources>>);

pub struct QuadDraw {
    material: usize,
    blend: f32,
    stage: ScreenQuadStage,
}

#[derive(Clone, Copy)]
pub(super) struct FullscreenInputs<'a> {
    pub frame: &'a crate::gameplay::effects::screen_distortion::Frame,
    pub dark_vision: Option<crate::gameplay::powers::dark_vision::DarkVisionRecord>,
    pub matrices: &'a crate::rendering::view_history::ViewMatrixFrame,
    pub shader_time: f32,
    pub local_time: f32,
}
impl FullscreenInputs<'_> {
    pub fn active(self) -> bool {
        self.frame.enabled()
    }
}
impl QuadDraw {
    pub(super) fn debug_values(&self) -> (usize, u32) {
        (self.material, self.blend.to_bits())
    }
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
    fullscreen_recorded: bool,
    last_fullscreen_receipt: Option<(u32, [u32; 7], bool)>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct ScreenConfig {
    extent: [u32; 2],
    downsample: u32,
    compressed: bool,
    format: TextureFormat,
}
impl ScreenEffects {
    pub(super) fn clear_frame(&mut self) {
        self.completed = false;
        self.fullscreen_recorded = false;
        self.last_fullscreen_receipt = None;
    }
    /// Command recording only: device execution and pixels need a capture.
    pub(super) fn fullscreen_recorded(&self) -> bool { self.fullscreen_recorded }
    pub(super) fn release_level(&mut self) {
        // 1400410f0 ends ownership of the world's effect materials. These
        // passes contain their uploaded images, not just viewport scratch;
        // matching dimensions cannot make them valid for a different world.
        let released = self.gpu.take().is_some();
        self.clear_frame();
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY screen-effect release: material_owner={released}");
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        encoder: &mut CommandEncoder,
        sources: &ScreenEffectSources,
        draws: &[QuadDraw],
        fullscreen: FullscreenInputs<'_>,
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
        self.fullscreen_recorded = false;
        if !fullscreen.active() { self.last_fullscreen_receipt = None; }
        let wave_record = dark_vision_wave::DarkVisionWave::active(fullscreen.dark_vision);
        if draws.is_empty() && !fullscreen.active() && wave_record.is_none() {
            return Ok(());
        }
        // 3222f0 tests DAT14397a8f8, registered at 14397a8d0 as
        // r_skipDistortionSurfaces. Its scope includes quads and UberMV.
        if crate::platform::console::shared().read().cvars
            .int("r_skipDistortionSurfaces").unwrap_or(0) != 0 {
            self.last_fullscreen_receipt = None;
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
        let sampler = passes.clamp_sampler().clone();
        let wave = wave_record.map(|record| dark_vision_wave::WaveFrame {
            record,
            view_projection: fullscreen.matrices.current.view_projection,
            local_time,
            target_extent: extent,
            full_height: extent[1],
            host_units: 139.26 * camera_exposure,
            depth,
            sampler: &sampler,
            adaptation,
        });
        passes.record_vectors(
            device, queue, encoder, sources, draws, Some(fullscreen), local_time, depth, inverse_z, wave.as_ref(),
        )?;
        // 140327270 adds the sonar wave colour to the scene before its
        // distortion resolves.
        if let Some(wave) = &wave {
            passes.dark_vision.as_mut().ok_or("dark vision wave pipelines absent")?
                .record_colour(device, queue, encoder, &sources.dark_vision, wave, colour, format)?;
        }
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
        self.fullscreen_recorded = fullscreen.active();
        if self.fullscreen_recorded && trace_fullscreen_distortion() {
            let frame = fullscreen.frame;
            let options = u32::from(frame.radial_blur) | (u32::from(frame.lens) << 1)
                | (u32::from(frame.wobble) << 2) | (u32::from(frame.sand_storm) << 3)
                | (u32::from(frame.camera_blur) << 4);
            let wobble = [frame.wobble_low_bound, frame.wobble_high_bound, frame.wobble_speed, frame.wobble_strength];
            let camera = [frame.camera_blur_strength, frame.camera_blur_start_depth, frame.camera_blur_end_depth];
            let moved = fullscreen.matrices.current.view_projection != fullscreen.matrices.previous.view_projection;
            let signature = (options, [wobble[0], wobble[1], wobble[2], wobble[3], camera[0], camera[1], camera[2]].map(f32::to_bits), moved);
            if self.last_fullscreen_receipt != Some(signature) {
                eprintln!("ACTION_SMOKE_FSDISTO_RENDER frame={} extent={extent:?} downsample={downsample} options={options:#x} wobble={wobble:?} camera_blur={camera:?} camera_moved={moved} parameters_uploaded=true pipeline_bound=true vector_draws=1 resolves=2 host_hdr_finished=true gpu_execution_verified=false pixels_verified=false", fullscreen.matrices.frame);
                self.last_fullscreen_receipt = Some(signature);
            }
        }
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
