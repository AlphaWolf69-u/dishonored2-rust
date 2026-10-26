//! Stereo processing for 4/8/12/16 delays (141180c10/141181020/
//! 1411815f0/141181de0). Other speaker formats have separate DSP paths.
use super::Parameters;
use crate::gameplay::sound::{delay::Delay, tail::{BufferStatus, Tail}};
use std::sync::Arc;

pub struct MatrixReverb {
    parameters: Parameters,
    defaults: Arc<[f32; 16]>,
    rate: u32,
    send: bool,
    lengths: Vec<u32>,
    lines: Vec<Delay>,
    pre_delay: Option<Delay>,
    coefficients: [[f32; 2]; 16],
    memory: [f32; 16],
    decay_time: f32,
    hf_ratio: f32,
    input_filter: [f32; 2],
    previous_input: f32,
    dc: [f32; 2],
    dc_pole: f32,
    levels: [f32; 2],
    tail: Tail,
}

impl MatrixReverb {
    pub fn new(mut parameters: Parameters, defaults: Arc<[f32; 16]>, rate: u32,
        speaker_mask: u32, send: bool) -> Result<Self, String>
    {
        if rate == 0 { return Err("matrix reverb requires a nonzero sample rate".into()); }
        if speaker_mask != 3 { return Err("matrix reverb requires its separate non-stereo processing path".into()); }
        parameters.structural_dirty = true;
        let levels = [parameters.dry_level, parameters.wet_level];
        let mut result = Self { parameters, defaults, rate, send, lengths: Vec::new(), lines: Vec::new(),
            pre_delay: None, coefficients: [[0.0; 2]; 16], memory: [0.0; 16], decay_time: 0.0, hf_ratio: 0.0,
            input_filter: [0.0; 2], previous_input: 0.0, dc: [0.0; 2],
            dc_pole: 1.0 - f32::from_bits(0x427b53d2) / rate as f32, levels, tail: Tail::default() };
        result.rebuild()?;
        Ok(result)
    }

    pub fn parameters(&self) -> &Parameters { &self.parameters }
    pub fn set_parameter(&mut self, id: u16, bytes: &[u8]) -> Result<(), String> { self.parameters.set(id, bytes) }

    fn rebuild(&mut self) -> Result<(), String> {
        if self.parameters.delay_mode == 0 {
            for (slot, time) in self.parameters.custom_delays.iter_mut().zip(self.defaults.iter())
                .take(self.parameters.delay_count as usize) { *slot = Some(*time); }
        }
        let lengths = self.parameters.delay_lengths(self.rate, &self.defaults)?;
        let lines = lengths.iter().map(|&length| Delay::exact(length)).collect::<Result<Vec<_>, _>>()?;
        let seconds = self.parameters.pre_delay;
        if !seconds.is_finite() || seconds < 0.0 { return Err("invalid matrix reverb pre-delay".into()); }
        let frames = crate::float::truncate_i64(self.rate as f32 * seconds) as u32;
        let pre_delay = if frames == 0 { None } else { Some(Delay::exact(frames)?) };
        self.lengths = lengths;
        self.lines = lines;
        self.pre_delay = pre_delay;
        self.reset();
        // 14117ed20 retains damping/input coefficients. Execute recalculates
        // them only when the cached decay time or HF ratio differs.
        self.parameters.structural_dirty = false;
        Ok(())
    }

    /// 14117eab0 clears histories while retaining every ring cursor.
    pub fn reset(&mut self) {
        for line in &mut self.lines { line.reset(); }
        if let Some(delay) = &mut self.pre_delay { delay.reset(); }
        self.memory.fill(0.0);
        self.previous_input = 0.0;
        self.dc = [0.0; 2];
    }

    pub fn process_buffer(&mut self, channels: &mut [&mut [f32]], status: BufferStatus) -> Result<BufferStatus, String> {
        if channels.len() != 2 || channels[0].len() != channels[1].len() {
            return Err("matrix reverb stereo buffer has inconsistent channels".into());
        }
        if self.parameters.structural_dirty { self.rebuild()?; }
        if self.decay_time != self.parameters.decay_time || self.hf_ratio != self.parameters.hf_ratio {
            for (slot, coefficients) in self.coefficients.iter_mut().zip(self.parameters.damping(&self.lengths, self.rate)?) {
                *slot = coefficients;
            }
            let ratio = 1.0 / f64::from(self.parameters.hf_ratio);
            let pole = (1.0 - ratio) / (ratio + 1.0);
            let gain = 1.0 / (1.0 - pole);
            self.input_filter = [gain as f32, (-gain * pole) as f32];
            self.decay_time = self.parameters.decay_time;
            self.hf_ratio = self.parameters.hf_ratio;
        }
        if self.send { self.parameters.dry_level = 0.0; self.levels[0] = 0.0; }
        let duration = crate::float::truncate_i64(self.rate as f32 * self.parameters.decay_time) as u32;
        let status = self.tail.prepare(channels, status, duration)?;
        let count = usize::from(status.valid_frames);
        if count == 0 { return Ok(status); }
        let inverse = 1.0 / channels[0].len() as f32;
        let steps = [(self.parameters.dry_level - self.levels[0]) * inverse,
            (self.parameters.wet_level - self.levels[1]) * inverse];
        let length = self.lines.len();
        let sum_order: &[usize] = match length {
            4 => &[3, 1, 2, 0],
            8 => &[7, 3, 5, 1, 6, 2, 4, 0],
            12 => &[11, 7, 3, 9, 5, 1, 10, 6, 2, 8, 4, 0],
            16 => &[15, 7, 3, 11, 13, 5, 1, 9, 14, 6, 2, 10, 12, 4, 0, 8],
            _ => return Err("matrix reverb has no configured stereo DSP path".into()),
        };
        let output_order: &[usize] = match length {
            4 => &[1, 3, 0, 2],
            8 => &[5, 1, 7, 3, 4, 0, 6, 2],
            12 => &[9, 1, 5, 11, 3, 7, 8, 0, 4, 10, 2, 6],
            _ => &[13, 1, 5, 9, 15, 3, 7, 11, 12, 0, 4, 8, 14, 2, 6, 10],
        };
        for frame in 0..count {
            for ((memory, line), coefficients) in self.memory.iter_mut().zip(&self.lines).zip(self.coefficients) {
                *memory = line.current() * coefficients[0] + *memory * coefficients[1];
            }
            let mut total = self.memory[sum_order[0]];
            for &index in &sum_order[1..] { total += self.memory[index]; }
            let common = total * (-2.0 / length as f32);
            for (level, step) in self.levels.iter_mut().zip(steps) { *level += step; }
            let input = channels[0][frame] + channels[1][frame];
            for (channel, signs) in channels.iter_mut().zip([[1.0, -1.0, 1.0, -1.0], [1.0, 1.0, -1.0, -1.0]]) {
                let first = output_order[0];
                let mut wet = self.memory[first] * signs[first & 3];
                for &index in &output_order[1..] { wet += self.memory[index] * signs[index & 3]; }
                channel[frame] = channel[frame] * self.levels[0] + wet * self.levels[1];
            }
            self.dc[1] = (self.dc_pole * self.dc[1] + input) - self.dc[0];
            self.dc[0] = input;
            let delayed = match &mut self.pre_delay { Some(delay) => delay.sample(self.dc[1]), None => self.dc[1] };
            let filtered = self.input_filter[0] * delayed + self.input_filter[1] * self.previous_input;
            self.previous_input = delayed;
            for (index, line) in self.lines.iter_mut().enumerate() {
                let next = self.memory[(index + 1) % length];
                let sample = if length == 4 || index < 3 || index == length - 1 { filtered + common + next }
                    else { filtered + next + common };
                line.sample(sample);
            }
        }
        self.levels = [self.parameters.dry_level, self.parameters.wet_level];
        Ok(status)
    }
}
