//! FOV family21b360/21b1b0 and registration-ordered view mixer21b4e0.
use super::{parameters::EffectParameterSample, timing::EffectTimings};
use std::sync::Arc;

pub(crate) struct Runtime {
    pub definition: Arc<crate::assets::fx::fov::Asset>,
    timing: EffectTimings,
    parameter: EffectParameterSample,
    pub value: f32,
    pub blend: f32,
    pub active: bool,
}
impl Runtime {
    pub fn new(definition: Arc<crate::assets::fx::fov::Asset>, offsets: [f32; 4], seed: &mut u32) -> Result<Self, String> {
        let timing = EffectTimings::new(&definition.common.timing, offsets, seed);
        let parameter = EffectParameterSample::new(definition.value.range, seed);
        let value = definition.value.duration.sample(0., parameter.curve_mix)? * parameter.scale;
        Ok(Self { definition, timing, parameter, value, blend: 0., active: true })
    }
    pub fn can_loop(&self) -> bool { self.active && self.definition.common.looping }
    pub fn stop(&mut self, now: f32) {
        self.timing.begin_fade_out(self.definition.common.keep_duration_on_stop, now);
    }
    pub fn update(&mut self, now: f32, seed: &mut u32) -> Result<(), String> {
        if !self.active || !(self.timing.delay_end <= now) { return Ok(()); }
        if self.timing.duration_end <= now {
            if !self.definition.common.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                let time = (now-self.timing.duration_end)*self.timing.fade_out_rate;
                self.blend = self.definition.value.fade_out.sample(time, self.parameter.curve_mix)?;
                if time > 1. { self.active = false; }
                return Ok(());
            }
            self.timing.restart(&self.definition.common.timing, self.timing.duration_end, seed);
        }
        if now < self.timing.fade_in_end {
            self.blend = self.definition.value.fade_in.sample(
                (now-self.timing.delay_end)*self.timing.fade_in_rate, self.parameter.curve_mix)?;
        } else if now < self.timing.duration_end {
            self.value = self.definition.value.duration.sample(
                (now-self.timing.fade_in_end)*self.timing.duration_rate, self.parameter.curve_mix)? * self.parameter.scale;
            self.blend = 1.;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Default)]
pub struct Sample { pub value: f32, pub blend: f32, pub percentage: bool }
impl Sample {
    pub fn apply(self, base: f32, current: f32) -> f32 {
        let value = if self.percentage { self.value * base } else { self.value };
        let value = super::normalize_small(value);
        let current = super::normalize_small(current);
        (1.-self.blend)*current + value*self.blend
    }
}
