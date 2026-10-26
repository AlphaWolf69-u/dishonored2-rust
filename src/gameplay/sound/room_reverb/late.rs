//! Late reverberation allocation and block flow (14116f350/14116ab60).
use super::{FeedbackNetwork, Parameters};
use crate::gameplay::sound::delay::{AllPass, Delay};

struct Unit {
    input_delay: Delay,
    network: FeedbackNetwork,
}

#[derive(Clone, Copy)]
struct DcBlocker {
    input: f32,
    output: f32,
    pole: f32,
}
impl DcBlocker {
    fn process(&mut self, samples: &mut [f32], gain: f32) {
        // 1411705d0 applies normalization before differencing the input.
        for sample in samples {
            let scaled = gain * *sample;
            self.output = (scaled - self.input) + self.output * self.pole;
            self.input = scaled;
            *sample = self.output;
        }
    }
}

pub struct LateReverb {
    rate: u32,
    pre_delay: Option<Delay>,
    diffusers: [AllPass; 4],
    units: Vec<Unit>,
    dc: [DcBlocker; 2],
    normalization: f32,
}
impl LateReverb {
    pub fn new(parameters: &Parameters, rate: u32) -> Result<Self, String> {
        let (units, normalization) = allocate_units(parameters, rate)?;
        let pole = 1.0 - (parameters.tuning[8] * f32::from_bits(0x40c90fdb)) / rate as f32;
        Ok(Self { rate, pre_delay: parameters.pre_delay_line(rate)?, diffusers: parameters.input_diffusers(rate)?, units,
            dc: [DcBlocker { input: 0.0, output: 0.0, pole }; 2],
            normalization })
    }

    pub fn configure_room(&mut self, previous: &Parameters, parameters: &Parameters) -> Result<(), String> {
        if previous.quality != parameters.quality || previous.density != parameters.density || previous.room_shape != parameters.room_shape {
            let (units, normalization) = allocate_units(parameters, self.rate)?;
            self.units = units;
            self.normalization = normalization;
        }
        if previous.pre_delay != parameters.pre_delay { self.pre_delay = parameters.pre_delay_line(self.rate)?; }
        Ok(())
    }

    pub fn configure_rtpc(&mut self, previous: &Parameters, parameters: &Parameters) {
        if previous.decay_time != parameters.decay_time || previous.hf_damping != parameters.hf_damping {
            for unit in &mut self.units { unit.network.configure(parameters.decay_time, parameters.hf_damping, self.rate); }
        }
        if previous.diffusion != parameters.diffusion {
            for (diffuser, coefficient) in self.diffusers.iter_mut().zip(parameters.diffusion_coefficients(true)) {
                diffuser.set_coefficient(coefficient);
            }
        }
    }

    pub fn reset(&mut self) {
        if let Some(delay) = &mut self.pre_delay { delay.reset(); }
        for diffuser in &mut self.diffusers { diffuser.reset(); }
        for unit in &mut self.units { unit.input_delay.reset(); unit.network.reset(); }
        for dc in &mut self.dc { dc.input = 0.0; dc.output = 0.0; }
    }

    /// Input downmix/input-tone filtering and output mixing belong to the
    /// effect owner. The callback applies tone bands at insertion point 2,
    /// after diffusion and before the feedback units (14116a8a0).
    pub fn process(&mut self, input: &[f32], left: &mut [f32], right: &mut [f32],
        mut after_diffusion: impl FnMut(&mut [f32]) -> Result<(), String>) -> Result<(), String>
    {
        if input.len() != left.len() || input.len() != right.len() || input.len() % 4 != 0 {
            return Err("late reverb buffers require equal frame counts divisible by four".into());
        }
        let mut work = super::Block([0.0; 256]);
        for start in (0..input.len()).step_by(256) {
            let count = (input.len() - start).min(256);
            let work = &mut work.0[..count];
            work.copy_from_slice(&input[start..start + count]);
            if let Some(delay) = &mut self.pre_delay { delay.process_in_place(work); }
            for diffuser in &mut self.diffusers { diffuser.process_in_place(work); }
            after_diffusion(work)?;
            let left = &mut left[start..start + count];
            let right = &mut right[start..start + count];
            left.fill(0.0);
            right.fill(0.0);
            for (index, unit) in self.units.iter_mut().enumerate() {
                unit.input_delay.process_in_place(work);
                unit.network.process_add(work, if index & 1 == 0 { &mut *left } else { &mut *right })?;
            }
            self.dc[0].process(left, self.normalization);
            self.dc[1].process(right, self.normalization);
        }
        Ok(())
    }
}

fn allocate_units(parameters: &Parameters, rate: u32) -> Result<(Vec<Unit>, f32), String> {
    // 14116f350's fixed local arrays hold four lengths per unit.
    if parameters.quality > 16 { return Err("room reverb quality exceeds sixteen units".into()); }
    let count = parameters.quality as usize;
    let times = distribution(parameters, count * 4)?;
    let lengths = super::odd_lengths(&times, rate);
    let mut seed = 0x01bc_0f99u32;
    let mut units = Vec::new();
    for index in 0..count {
        let base = (1.0 / count as f32) * parameters.tuning[9];
        seed = seed.wrapping_mul(0x0bb3_8435).wrapping_add(0x3619_636b);
        let milliseconds = parameters.tuning[10] * 0.01 * base * seed as f32 * f32::from_bits(0x2f800000) + base;
        let frames = crate::float::truncate_i64(milliseconds * 0.001 * rate as f32) as u32;
        let requested = std::array::from_fn(|quarter| lengths[index + quarter * count]);
        units.push(Unit { input_delay: Delay::new(frames)?,
            network: FeedbackNetwork::new(requested, parameters.decay_time, parameters.hf_damping, rate)? });
    }
    let normalization = if count < 2 { 1.0 } else { 1.0 / (count as f32).sqrt() };
    Ok((units, normalization * f32::from_bits(0x3fb504f3)))
}

fn distribution(parameters: &Parameters, count: usize) -> Result<Vec<f32>, String> {
    // 14116d700: density selects the mean; shape selects its lower bound,
    // then a deterministic LCG perturbs the linearly spaced values.
    let tuning = &parameters.tuning;
    let mean = (tuning[1] - tuning[0]) * 0.01 * parameters.density + tuning[0];
    let minimum = ((tuning[4] - tuning[3]) * 0.01 * parameters.room_shape + tuning[3]) * mean;
    let maximum = (mean - minimum) + mean;
    let mut seed = 0x006b_4cc7u32;
    let mut times = Vec::with_capacity(count);
    for index in 0..count {
        let base = index as f32 * (maximum - minimum) * (1.0 / (count as f32 - 1.0)) + minimum;
        seed = seed.wrapping_mul(0x0bb3_8435).wrapping_add(0x3619_636b);
        let value = tuning[2] * 0.01 * base * seed as f32 * f32::from_bits(0x2f800000) + base;
        if !value.is_finite() { return Err("non-finite feedback reverb delay distribution".into()); }
        times.push(value);
    }
    times.sort_by(|a, b| if a < b { std::cmp::Ordering::Less } else if b < a { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
    Ok(times)
}
