//! Stereo early-reflection history (14116f860/14116fbc0/14116fb90).
use crate::assets::sound_effects::{ReflectionPattern, ReflectionTap};
use crate::gameplay::sound::delay::Delay;

struct Tap {
    cursor: usize,
    gain: f32,
}

pub struct EarlyReflections {
    earliest: Option<Delay>,
    history: Vec<f32>,
    write: usize,
    left: Vec<Tap>,
    right: Vec<Tap>,
}
impl EarlyReflections {
    pub fn new(pattern: &ReflectionPattern, room_size: f32, sample_rate: u32) -> Result<Self, String> {
        if !(1..=64).contains(&pattern.left.len()) || !(1..=64).contains(&pattern.right.len()) {
            return Err("early reflections require 1..64 taps per side".into());
        }
        let scale = 2.0f32.powf(room_size * 0.01);
        let first = if pattern.right[0].milliseconds <= pattern.left[0].milliseconds {
            pattern.right[0].milliseconds
        } else { pattern.left[0].milliseconds } * scale;
        let left_last = pattern.left.last().unwrap().milliseconds;
        let right_last = pattern.right.last().unwrap().milliseconds;
        let last = if left_last <= right_last { right_last } else { left_last };
        let span = (last * scale - first) * 0.001 * sample_rate as f32;
        if !span.is_finite() { return Err("non-finite early reflection history length".into()); }
        let mut length = crate::float::truncate_i64(span) as u32;
        if length < 4 { length = 4; }
        else if length > 65535 { return Err("early reflection history exceeds the 16-bit cursor range".into()); }
        let length = length & !3; // This allocation rounds DOWN to four.
        let taps = |source: &[ReflectionTap]| -> Vec<Tap> {
            let mut output = Vec::new();
            let mut previous = u32::MAX;
            for tap in source {
                let mut offset = crate::float::truncate_i64((scale * tap.milliseconds - first) * 0.001 * sample_rate as f32) as u32 & !3;
                if length <= offset { offset = length - 4; }
                // 14116f860 keeps the first coefficient for coincident offsets.
                if offset != previous {
                    output.push(Tap { cursor: if offset == 0 { 0 } else { (length - offset) as usize }, gain: tap.gain });
                }
                previous = offset;
            }
            output
        };
        // 14116f0b0 allocates the common first-arrival delay separately.
        let first_frames = crate::float::truncate_i64(sample_rate as f32 * first * 0.001) as u32;
        let earliest = if first_frames == 0 { None } else { Some(Delay::new(first_frames)?) };
        let mut history = Vec::new();
        history.try_reserve_exact(length as usize).map_err(|error| format!("early reflection allocation: {error}"))?;
        history.resize(length as usize, 0.0);
        Ok(Self { earliest, history, write: 0, left: taps(&pattern.left), right: taps(&pattern.right) })
    }

    /// Reset clears samples without moving the write or tap cursors.
    pub fn reset(&mut self) {
        self.history.fill(0.0);
        if let Some(delay) = &mut self.earliest { delay.reset(); }
    }

    pub fn process(&mut self, input: &[f32], left: &mut [f32], right: &mut [f32]) -> Result<(), String> {
        if input.len() != left.len() || input.len() != right.len() || input.len() % 4 != 0 {
            return Err("early reflection buffers require equal frame counts divisible by four".into());
        }
        for start in (0..input.len()).step_by(4) {
            for lane in 0..4 {
                let value = input[start + lane];
                let value = if let Some(delay) = &mut self.earliest { delay.sample(value) } else { value };
                self.history[self.write + lane] = value;
            }
            for lane in 0..4 {
                let sum = |taps: &[Tap]| {
                    let mut value = 0.0;
                    for tap in taps { value = value + self.history[tap.cursor + lane] * tap.gain; }
                    value
                };
                left[start + lane] = sum(&self.left);
                right[start + lane] = sum(&self.right);
            }
            for tap in self.left.iter_mut().chain(&mut self.right) {
                tap.cursor += 4;
                if tap.cursor == self.history.len() { tap.cursor = 0; }
            }
            self.write += 4;
            if self.write == self.history.len() { self.write = 0; }
        }
        Ok(())
    }
}
