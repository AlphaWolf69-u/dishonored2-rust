//! FX screen fade producer (start 14022a570, phase update 14022a220 ->
//! 14022a430 fade-in / 14022a390 duration / 14022a4d0 fade-out) and the
//! view's composition of the registered fades (14022a6b0), which sets the
//! view's fade object colour (140167030).
use super::{
    envelope::{EnvelopePhase, ScalarEnvelopeSample, VectorEnvelopeSample},
    timing::EffectTimings,
};
use crate::assets::fx::screen_fade::ScreenFadeAsset;
use std::sync::Arc;

/// The composed fade colour of the last FX update; None while no fade is
/// registered (the fade object then keeps its last colour).
#[derive(bevy::prelude::Resource, Clone, Copy, Default)]
pub struct FxScreenFadeFrame(pub Option<[f32; 4]>);

pub struct ScreenFade {
    asset: Arc<ScreenFadeAsset>,
    timing: EffectTimings,
    colour: VectorEnvelopeSample,
    alpha: ScalarEnvelopeSample,
    /// +0x6c..+0x78: the current RGBA.
    pub rgba: [f32; 4],
    pub active: bool,
}

impl ScreenFade {
    pub fn new(asset: Arc<ScreenFadeAsset>, offsets: [f32; 4], seed: &mut u32) -> Result<Self, String> {
        let timing = EffectTimings::new(&asset.timing, offsets, seed);
        let colour = VectorEnvelopeSample::new(&asset.colour, seed)?;
        let alpha = ScalarEnvelopeSample::new(&asset.alpha, seed)?;
        let rgba = [colour.current[0], colour.current[1], colour.current[2], alpha.current];
        Ok(Self { asset, timing, colour, alpha, rgba, active: true })
    }

    pub fn can_loop(&self) -> bool {
        self.active && self.asset.looping
    }

    pub fn stop(&mut self, now: f32) {
        self.timing.begin_fade_out(self.asset.keep_duration_on_stop, now);
    }

    /// 14022a220.
    pub fn update(&mut self, age: f32, seed: &mut u32) -> Result<(), String> {
        if !self.active || age < self.timing.delay_end {
            return Ok(());
        }
        let (phase, mut t) = if self.timing.duration_end <= age {
            if !self.asset.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                (EnvelopePhase::FadeOut, (age - self.timing.duration_end) * self.timing.fade_out_rate)
            } else {
                self.timing.restart(&self.asset.timing, self.timing.duration_end, seed);
                if self.timing.fade_in_end <= age {
                    if self.timing.duration_end <= age {
                        return Ok(());
                    }
                    (EnvelopePhase::Duration, (age - self.timing.fade_in_end) * self.timing.duration_rate)
                } else {
                    (EnvelopePhase::FadeIn, (age - self.timing.delay_end) * self.timing.fade_in_rate)
                }
            }
        } else if self.timing.fade_in_end <= age {
            (EnvelopePhase::Duration, (age - self.timing.fade_in_end) * self.timing.duration_rate)
        } else {
            (EnvelopePhase::FadeIn, (age - self.timing.delay_end) * self.timing.fade_in_rate)
        };
        if matches!(phase, EnvelopePhase::FadeOut) && 1. < t {
            self.active = false;
            t = 1.;
        }
        let colour = self.colour.sample(&self.asset.colour, phase, t)?;
        let alpha = self.alpha.sample(&self.asset.alpha, phase, t)?;
        self.rgba = [colour[0], colour[1], colour[2], alpha];
        Ok(())
    }
}

/// 14022a6b0: the first registered fade's clamped colour, then each later
/// one composited over it (rgb = rgb (1 - a) + c, a = 1 - (1 - a)(1 - a')).
pub fn compose(fades: impl IntoIterator<Item = [f32; 4]>) -> Option<[f32; 4]> {
    let clamp = |rgba: [f32; 4]| rgba.map(|value| value.clamp(0., 1.));
    let mut fades = fades.into_iter();
    let mut result = clamp(fades.next()?);
    for fade in fades {
        let [r, g, b, a] = clamp(fade);
        let keep = 1. - a;
        result = [result[0] * keep + r, result[1] * keep + g, result[2] * keep + b, 1. - (1. - result[3]) * keep];
    }
    Some(result)
}
