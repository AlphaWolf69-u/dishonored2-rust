//! Rumble229ba0/229ec0 and phase kernels228440/229870/229980/229a90.
use super::{
    envelope::{EnvelopePhase, ScalarEnvelopeSample},
    timing::EffectTimings,
};
use crate::{
    assets::fx::rumble::Asset,
    float::{add, divide, multiply, subtract},
};
use std::sync::Arc;

pub(crate) struct Runtime {
    pub definition: Arc<Asset>,
    timing: EffectTimings,
    channels: [ScalarEnvelopeSample; 5],
    attenuation_seed: u32,
    radius: f32,
    impulse: f32,
    pub active: bool,
}
impl Runtime {
    pub fn new(definition: Arc<Asset>, offsets: [f32; 4], seed: &mut u32) -> Result<Self, String> {
        //229ba0 advances/stores a private attenuation seed BEFORE timings.
        let attenuation_seed = crate::random::lcg_step(seed);
        let timing = EffectTimings::new(&definition.timing, offsets, seed);
        let channels = [
            ScalarEnvelopeSample::new(&definition.channels[0], seed)?,
            ScalarEnvelopeSample::new(&definition.channels[1], seed)?,
            ScalarEnvelopeSample::new(&definition.channels[2], seed)?,
            ScalarEnvelopeSample::new(&definition.channels[3], seed)?,
            ScalarEnvelopeSample::new(&definition.channels[4], seed)?,
        ];
        let radius = super::timing::sample_range(definition.distance_radius, seed);
        let impulse = super::timing::sample_range(definition.impulse, seed);
        Ok(Self {
            definition,
            timing,
            channels,
            attenuation_seed,
            radius,
            impulse,
            active: true,
        })
    }
    pub fn can_loop(&self) -> bool {
        self.active && self.definition.looping
    }
    pub fn stop(&mut self, age: f32) {
        self.timing
            .begin_fade_out(self.definition.keep_duration_on_stop, age);
    }
    pub fn update(
        &mut self,
        age: f32,
        delta: f32,
        listener: Option<[f32; 3]>,
        position: [f32; 3],
        impulse: f32,
        seed: &mut u32,
    ) -> Result<Option<[f32; 4]>, String> {
        if !self.active || !(delta > 0.) || !(self.timing.delay_end <= age) {
            return Ok(None);
        }
        let (phase, t) = if self.timing.duration_end <= age {
            if !self.definition.looping || self.timing.fading_out {
                self.timing.fading_out = true;
                let t = multiply(
                    subtract(age, self.timing.duration_end),
                    self.timing.fade_out_rate,
                );
                //229a90 ends only for1<t, then samples exactly1 once.
                if t > 1. {
                    self.active = false;
                }
                (EnvelopePhase::FadeOut, if t > 1. { 1. } else { t })
            } else {
                self.timing
                    .restart(&self.definition.timing, self.timing.duration_end, seed);
                if !(age < self.timing.duration_end) {
                    return Ok(None);
                }
                (
                    EnvelopePhase::Duration,
                    multiply(
                        subtract(age, self.timing.fade_in_end),
                        self.timing.duration_rate,
                    ),
                )
            }
        } else if self.timing.fade_in_end <= age {
            if !(age < self.timing.duration_end) {
                return Ok(None);
            }
            (
                EnvelopePhase::Duration,
                multiply(
                    subtract(age, self.timing.fade_in_end),
                    self.timing.duration_rate,
                ),
            )
        } else {
            (
                EnvelopePhase::FadeIn,
                multiply(
                    subtract(age, self.timing.delay_end),
                    self.timing.fade_in_rate,
                ),
            )
        };
        let mut values = [0.; 5];
        for ((sample, definition), value) in self
            .channels
            .iter_mut()
            .zip(&self.definition.channels)
            .zip(&mut values)
        {
            *value = sample.sample(definition, phase, t)?;
        }
        //229710 still advances envelope/tails without a view context.
        let Some(listener) = listener else {
            return Ok(None);
        };
        let difference = std::array::from_fn::<_, 3, _>(|i| subtract(position[i], listener[i]));
        let distance = add(
            add(
                multiply(difference[0], difference[0]),
                multiply(difference[1], difference[1]),
            ),
            multiply(difference[2], difference[2]),
        )
        .sqrt();
        //229710 copies+7c into a stack seed. The same spatial/impulse
        //modulation is retained on subsequent frames, not resampled over time.
        let mut attenuation_seed = self.attenuation_seed;
        let distance_fade = modulated(
            &self.definition.distance_fade,
            divide(distance, if self.radius > 1. { self.radius } else { 1. }),
            &mut attenuation_seed,
        )?;
        let impulse_fade = modulated(
            &self.definition.impulse_fade,
            divide(impulse, if self.impulse > 1. { self.impulse } else { 1. }),
            &mut attenuation_seed,
        )?;
        let magnitude = multiply(multiply(values[0], distance_fade), impulse_fade);
        Ok(Some(std::array::from_fn(|i| {
            multiply(values[i + 1], magnitude)
        })))
    }
}
///228550/228e30 use their own two random draws and Catmull key-value
///sampling (not the standard envelope's packed Bezier tangent sampler).
fn modulated(
    parameter: &crate::gameplay::particles::parameters::ScalarParameter,
    t: f32,
    seed: &mut u32,
) -> Result<f32, String> {
    let scale_mix = crate::random::lcg_unit(seed);
    let curve_mix = crate::random::lcg_unit(seed);
    let scale = add(
        multiply(scale_mix, subtract(parameter.range[0], parameter.range[1])),
        parameter.range[1],
    );
    let sample = |curve: &super::curve::EffectFloatCurve| attenuation_curve(curve, t);
    let maximum = sample(&parameter.curve.maximum)?;
    //2289fd bypasses min interpolation when its key count is zero, while
    //the two RNG draws above still occur for a constant maximum curve.
    if parameter.curve.minimum.times.is_empty() {
        return Ok(multiply(maximum, scale));
    }
    let minimum = sample(&parameter.curve.minimum)?;
    Ok(multiply(
        add(multiply(subtract(maximum, minimum), curve_mix), minimum),
        scale,
    ))
}

///2286c0..2289fd /228a82..228dd7 retain the same key-value Catmull
///basis as1f3f10, but multiply coefficients before key values. This preserves
///the actual rumble kernel's operation order without changing particle curves.
fn attenuation_curve(curve: &super::curve::EffectFloatCurve, time: f32) -> Result<f32, String> {
    let count = curve.times.len();
    if count == 0 {
        return Ok(0.);
    }
    let tripled = curve.values.len() == count * 3;
    let key = |index: usize| {
        curve
            .values
            .get(if tripled { index * 3 + 1 } else { index })
            .copied()
            .ok_or_else(|| "rumble attenuation curve key absent".to_owned())
    };
    if count == 1 || curve.times[0] >= time {
        return key(0);
    }
    if time >= curve.times[count - 1] {
        return key(count - 1);
    }
    let mut index = 0;
    while index + 1 < count && time > curve.times[index + 1] {
        index += 1;
    }
    let [start, end] = super::curve::curve_key_times(&curve.times, index as i32);
    let mut t = subtract(time, start);
    let interval = subtract(end, start);
    if interval > f32::from_bits(0x3400_0000) {
        t = divide(t, interval);
    }
    let half = multiply(t, 0.5);
    let triple = multiply(t, 3.);
    let c0 = multiply(subtract(multiply(subtract(2., t), t), 1.), half);
    let c1 = add(multiply(multiply(subtract(triple, 5.), t), half), 1.);
    let c2 = multiply(add(multiply(subtract(4., triple), t), 1.), half);
    let c3 = multiply(multiply(subtract(t, 1.), t), half);
    let point = |at: i64| -> Result<f32, String> {
        if at < 0 {
            let first = key(0)?;
            Ok(add(multiply(subtract(key(1)?, first), at as f32), first))
        } else if at >= count as i64 {
            let last = key(count - 1)?;
            Ok(add(
                multiply(
                    subtract(last, key(count - 2)?),
                    (at - (count as i64 - 1)) as f32,
                ),
                last,
            ))
        } else {
            key(at as usize)
        }
    };
    let i = index as i64;
    Ok(add(
        add(
            add(multiply(point(i)?, c1), multiply(point(i - 1)?, c0)),
            multiply(point(i + 1)?, c2),
        ),
        multiply(point(i + 2)?, c3),
    ))
}
