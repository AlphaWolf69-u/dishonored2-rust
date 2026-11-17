//! FSDisto parameter producer: 21cf70/21d1d0/21d1f0 and view resolve8911f0.
//! The existing FX owner supplies family chains, parent clock and shared RNG.
use super::{
    environment::EnvironmentEffectDefinition,
    parameters::{EffectParameterSample, EffectVectorParameterSample},
    timing::EffectTimings,
    vector_curve::EffectVectorCurvePair,
};
use std::sync::Arc;

pub struct VectorEnvelope {
    pub range: [[f32; 4]; 2],
    pub duration: EffectVectorCurvePair<4>,
    pub fade_in: EffectVectorCurvePair<4>,
    pub fade_out: EffectVectorCurvePair<4>,
    pub uniform: bool,
}
impl Default for VectorEnvelope {
    fn default() -> Self {
        Self { range: [[0.; 4]; 2], duration: EffectVectorCurvePair::constant([1.; 4]),
            fade_in: EffectVectorCurvePair::constant([1.; 4]),
            fade_out: EffectVectorCurvePair::constant([1.; 4]), uniform: false }
    }
}

pub(crate) struct ParameterSample {
    sampled: EffectVectorParameterSample<4>,
    duration_base: [f32; 4],
    last_main: [f32; 4],
    current: [f32; 4],
}
impl ParameterSample {
    ///21c0b0/21c170 then21bfe0. Null renderParm references still sample.
    fn new(definition: &VectorEnvelope, seed: &mut u32) -> Result<Self, String> {
        let sampled = EffectVectorParameterSample::new(definition.range, definition.uniform, seed)?;
        let duration = definition.duration.sample(0., sampled.curve_mix)?;
        let duration_base = std::array::from_fn(|i| duration[i] * sampled.scale[i]);
        let fade = definition.fade_in.sample(0., sampled.curve_mix)?;
        let last_main = std::array::from_fn(|i| duration_base[i] * fade[i]);
        Ok(Self { sampled, duration_base, last_main, current: last_main })
    }
    fn update(&mut self, definition: &VectorEnvelope, phase: u8, time: f32) -> Result<(), String> {
        let (curve, base) = match phase {
            0 => (&definition.duration, self.sampled.scale),
            1 => (&definition.fade_in, self.duration_base),
            _ => (&definition.fade_out, self.last_main),
        };
        let curve = curve.sample(time, self.sampled.curve_mix)?;
        self.current = std::array::from_fn(|i| {
            if phase == 2 || (phase == 0 && i != 0) { curve[i] * base[i] }
            else { base[i] * curve[i] }
        });
        if phase != 2 { self.last_main = self.current; }
        Ok(())
    }
}

pub(crate) struct Runtime {
    pub definition: Arc<crate::assets::fx::screen_distortion::Asset>,
    timing: EffectTimings,
    blend_parameter: EffectParameterSample,
    initial_main: f32,
    last_main: f32,
    parameters: Vec<ParameterSample>,
    pub blend: f32,
    pub active: bool,
    pub visible: bool,
}
impl Runtime {
    pub fn new_reusing(definition: Arc<crate::assets::fx::screen_distortion::Asset>, offsets: [f32; 4], seed: &mut u32,
        mut parameters: Vec<ParameterSample>) -> Result<Self, String> {
        let common = &definition.blend;
        let timing = EffectTimings::new(&common.timing, offsets, seed);
        let blend_parameter = EffectParameterSample::new(common.scale, seed);
        let initial_main = common.duration.sample(0., blend_parameter.curve_mix)? * blend_parameter.scale;
        let last_main = common.fade_in.sample(0., blend_parameter.curve_mix)? * initial_main;
        parameters.clear();
        for parameter in &definition.parameters { parameters.push(ParameterSample::new(&parameter.value, seed)?); }
        Ok(Self { definition, timing, blend_parameter, initial_main, last_main,
            parameters, blend: last_main, active: true, visible: true })
    }
    pub fn can_loop(&self) -> bool { self.active && self.definition.blend.looping }
    pub fn stop(&mut self, now: f32) { self.timing.begin_fade_out(self.definition.blend.keep_duration_on_stop, now); }
    pub fn values(&self) -> impl Iterator<Item = [f32; 4]> + '_ { self.parameters.iter().map(|sample| sample.current) }
    pub fn take_samples(&mut self) -> Vec<ParameterSample> { std::mem::take(&mut self.parameters) }
    ///21c480: one duration restart per tick; strict fadeout terminal test.
    pub fn update(&mut self, now: f32, visible: bool, seed: &mut u32) -> Result<(), String> {
        self.visible = false;
        if !self.active || !(self.timing.delay_end <= now) { return Ok(()); }
        let common: &EnvironmentEffectDefinition = &self.definition.blend;
        let (phase, time, curve, base) = if self.timing.duration_end <= now {
            if !common.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                let time = (now - self.timing.duration_end) * self.timing.fade_out_rate;
                self.active = time < 1.;
                if !self.active { return Ok(()); }
                (2, time, &common.fade_out, self.last_main)
            } else {
                self.timing.restart(&common.timing, self.timing.duration_end, seed);
                if self.timing.fade_in_end <= now {
                    if !(now < self.timing.duration_end) { return Ok(()); }
                    (0, (now-self.timing.fade_in_end)*self.timing.duration_rate, &common.duration, self.blend_parameter.scale)
                } else {
                    (1, (now-self.timing.delay_end)*self.timing.fade_in_rate, &common.fade_in, self.initial_main)
                }
            }
        } else if self.timing.fade_in_end <= now {
            if !(now < self.timing.duration_end) { return Ok(()); }
            (0, (now-self.timing.fade_in_end)*self.timing.duration_rate, &common.duration, self.blend_parameter.scale)
        } else {
            (1, (now-self.timing.delay_end)*self.timing.fade_in_rate, &common.fade_in, self.initial_main)
        };
        for (sample, definition) in self.parameters.iter_mut().zip(&self.definition.parameters) {
            sample.update(&definition.value, phase, time)?;
        }
        self.blend = curve.sample(time, self.blend_parameter.curve_mix)? * base;
        if phase != 2 { self.last_main = self.blend; }
        self.visible = visible;
        Ok(())
    }
}

/// Final numeric view state. The renderer owns its buffers and shader; no
/// declaration, FX slot, image or lifetime lease crosses this publication.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub radial_blur: bool,
    pub lens: bool,
    pub wobble: bool,
    pub sand_storm: bool,
    pub camera_blur: bool,
    pub wobble_low_bound: f32,
    pub wobble_high_bound: f32,
    pub wobble_speed: f32,
    pub wobble_strength: f32,
    /// Authored and blended by8911f0, although the installed uber shader uses
    /// its literal0.15 instead of reading this declaration.
    pub wobble_dead_zone: f32,
    pub radial_blur_dead_zone: f32,
    pub radial_blur_strength: f32,
    pub radial_disto_strength: f32,
    pub lens_strength: f32,
    pub sand_storm_dead_zone: f32,
    pub sand_storm_start_depth: f32,
    pub sand_storm_end_depth: f32,
    pub sand_storm_start_value: [f32; 4],
    pub sand_storm_end_value: [f32; 4],
    pub sand_storm_world_wind: [f32; 3],
    pub camera_blur_strength: f32,
    pub camera_blur_start_depth: f32,
    pub camera_blur_end_depth: f32,
}

impl Frame {
    pub fn enabled(self) -> bool {
        self.radial_blur || self.lens || self.wobble || self.sand_storm || self.camera_blur
    }
    pub(crate) fn from_values(v: [[f32; 4]; 23]) -> Self {
        Self { radial_blur: v[0][0] != 0., lens: v[1][0] != 0., wobble: v[2][0] != 0.,
            sand_storm: v[3][0] != 0., camera_blur: v[4][0] != 0.,
            wobble_low_bound: v[5][0], wobble_high_bound: v[6][0], wobble_speed: v[7][0],
            wobble_strength: v[8][0], wobble_dead_zone: v[9][0], radial_disto_strength: v[10][0],
            radial_blur_strength: v[11][0], radial_blur_dead_zone: v[12][0], lens_strength: v[13][0],
            sand_storm_dead_zone: v[14][0], sand_storm_start_depth: v[15][0], sand_storm_end_depth: v[16][0],
            sand_storm_start_value: v[17], sand_storm_end_value: v[18],
            sand_storm_world_wind: v[19][..3].try_into().unwrap(), camera_blur_strength: v[20][0],
            camera_blur_start_depth: v[21][0], camera_blur_end_depth: v[22][0] }
    }
}
