//! Constant-pitch sample-rate conversion (141138270 / 141146820).
//! A source frame position uses sixteen fractional bits, as in the decoder.

pub fn pitch_step(cents: f32, source_rate: u32, output_rate: u32) -> Result<u32, String> {
    if !cents.is_finite() || source_rate == 0 || output_rate == 0 {
        return Err("invalid sound pitch or sample rate".into());
    }
    let ratio = 2.0f32.powf(cents * f32::from_bits(0x3a5a740e));
    let step = ratio * (source_rate as f32 / output_rate as f32) * 65536.0 + 0.5;
    if !step.is_finite() || step > u32::MAX as f32 {
        return Err("sound pitch exceeds the fixed-point resampler range".into());
    }
    let step = step as u32;
    Ok(if step == 0 { if cents > 0.0 { u32::MAX } else { 1 } } else { step })
}

#[derive(Clone, Debug)]
pub struct Resampler {
    phase: u64,
    step: u32,
    target: u32,
    progress: u32,
}
impl Resampler {
    pub fn new(step: u32) -> Self {
        // 141137dc0 initializes fractional position to one source frame.
        Self { phase: 65536, step, target: step, progress: 1024 }
    }

    /// 141138270 captures an interrupted ramp's current step; 141147170
    /// advances the new fixed-point ramp across 1024 output frames.
    pub fn set_step(&mut self, target: u32, smooth: bool) {
        if target == self.target { return; }
        if self.progress < 1024 {
            let delta = self.target.wrapping_sub(self.step) as i32;
            let offset = delta.wrapping_mul(self.progress as i32) / 1024;
            self.step = self.step.wrapping_add(offset as u32);
            if self.step == 0 { self.step = 1; }
        }
        self.target = target;
        self.progress = if smooth { 0 } else { self.step = target; 1024 };
    }

    /// Sample one channel, then call advance once after the complete frame.
    /// Unity rate uses the direct-copy dispatch, retaining the last source
    /// frame without requesting an interpolation neighbour beyond the buffer.
    pub fn sample(&self, samples: &[f32], channels: usize, channel: usize) -> Option<f32> {
        self.sample_looped(samples, channels, channel, None)
    }

    pub fn sample_looped(&self, samples: &[f32], channels: usize, channel: usize,
        looping: Option<super::looping::SourceLoop>) -> Option<f32>
    {
        if channels == 0 || channel >= channels { return None; }
        let frame = usize::try_from(self.phase >> 16).ok()?;
        let source_frame = |frame: usize| match looping { Some(looping) => looping.frame(frame as u64), None => Some(frame) };
        let first = *samples.get(source_frame(frame.checked_sub(1)?)?.checked_mul(channels)?.checked_add(channel)?)?;
        if self.step == 65536 && self.step == self.target { return Some(first); }
        let second = *samples.get(source_frame(frame)?.checked_mul(channels)?.checked_add(channel)?)?;
        let fraction = (self.phase & 65535) as f32;
        Some((second - first) * fraction * (1.0 / 65536.0) + first)
    }

    pub fn advance(&mut self) {
        let step = if self.progress < 1024 {
            self.progress += 1;
            let delta = self.target.wrapping_sub(self.step);
            let step = self.step.wrapping_mul(1024).wrapping_add(delta.wrapping_mul(self.progress)) >> 10;
            if self.progress == 1024 { self.step = self.target; }
            step
        } else { self.step };
        self.phase += u64::from(step);
    }
}
