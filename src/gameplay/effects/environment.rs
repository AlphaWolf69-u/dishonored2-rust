//! Environment FX blend producer (21a240, 21a310 and 21a090). The FX scheduler
//! owns registration, the shared RNG and the parent effect's clock/offsets.
use super::{
    curve::{EffectFloatCurve, EffectFloatCurvePair},
    parameters::EffectParameterSample,
    timing::*,
};
use std::sync::Arc;

pub struct EnvironmentEffectDefinition {
    pub looping: bool,
    pub keep_duration_on_stop: bool,
    pub timing: EffectTimingRanges,
    pub scale: [f32; 2],
    pub duration: EffectFloatCurvePair,
    pub fade_in: EffectFloatCurvePair,
    pub fade_out: EffectFloatCurvePair,
}
impl Default for EnvironmentEffectDefinition {
    /// 21a000 -> 224300 and 219ef0(1, 0, 1). Sparse authored edits inherit
    /// these curves; an omitted duration curve is constant one, not empty.
    fn default() -> Self {
        let maximum = |times, values| EffectFloatCurvePair {
            maximum: EffectFloatCurve { times, values },
            minimum: EffectFloatCurve::default(),
        };
        Self {
            looping: false,
            keep_duration_on_stop: false,
            timing: EffectTimingRanges::default(),
            scale: [1.; 2],
            duration: maximum(vec![0.], vec![0., 1., 0.]),
            fade_in: maximum(vec![0., 1.], vec![0., 0., 0., 0., 1., 0.]),
            fade_out: maximum(vec![0., 1.], vec![0., 1., 0., 0., 0., 0.]),
        }
    }
}

pub struct EnvironmentBlendEffect {
    definition: Arc<EnvironmentEffectDefinition>,
    pub timing: EffectTimings,
    pub parameter: EffectParameterSample,
    pub initial_main: f32,
    pub last_main: f32,
    pub blend: f32,
    pub view_mask: u8,
    pub active: bool,
}
impl EnvironmentBlendEffect {
    pub fn new(
        definition: impl Into<Arc<EnvironmentEffectDefinition>>,
        offsets: [f32; 4],
        view_mask: u8,
        seed: &mut u32,
    ) -> Result<Self, String> {
        let definition = definition.into();
        let timing = EffectTimings::new(&definition.timing, offsets, seed);
        let parameter = EffectParameterSample::new(definition.scale, seed);
        let initial_main = definition.duration.sample(0., parameter.curve_mix)? * parameter.scale;
        let last_main = definition.fade_in.sample(0., parameter.curve_mix)? * initial_main;
        Ok(Self {
            definition,
            timing,
            parameter,
            initial_main,
            last_main,
            blend: 0.,
            view_mask,
            active: true,
        })
    }

    pub fn can_loop(&self) -> bool {
        self.active && self.definition.looping
    }

    pub fn stop(&mut self, now: f32) {
        self.timing
            .begin_fade_out(self.definition.keep_duration_on_stop, now);
    }

    pub fn update(&mut self, now: f32, view_mask: u8, seed: &mut u32) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        self.view_mask = view_mask;
        if self.timing.delay_end > now {
            return Ok(());
        }
        if now >= self.timing.duration_end {
            if !self.definition.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                let t = (now - self.timing.duration_end) * self.timing.fade_out_rate;
                self.active = 1. >= t;
                self.blend = self
                    .definition
                    .fade_out
                    .sample(t, self.parameter.curve_mix)?
                    * self.last_main;
                return Ok(());
            }
            // One restart per tick, even if the clock skips several cycles.
            self.timing
                .restart(&self.definition.timing, self.timing.duration_end, seed);
        }
        // JBE includes unordered inputs; preserve the source's NaN branches.
        let blend = if !(self.timing.fade_in_end > now) {
            if !(self.timing.duration_end > now) {
                return Ok(());
            }
            let t = (now - self.timing.fade_in_end) * self.timing.duration_rate;
            self.definition
                .duration
                .sample(t, self.parameter.curve_mix)?
                * self.parameter.scale
        } else {
            let t = (now - self.timing.delay_end) * self.timing.fade_in_rate;
            self.definition
                .fade_in
                .sample(t, self.parameter.curve_mix)?
                * self.initial_main
        };
        self.last_main = blend;
        self.blend = blend;
        Ok(())
    }
}
