//! Authored silence source 0x00650002 (14118a130 / 14118a270 / 14118a340).
#[derive(Clone, Copy, Debug)]
pub struct Parameters { pub seconds: f32, pub minimum_offset: f32, pub maximum_offset: f32 }
impl Parameters {
    /// 141189ff0 / 14118a060: empty blocks use defaults; otherwise three floats.
    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() { return Ok(Self { seconds: 1.0, minimum_offset: 0.0, maximum_offset: 0.0 }); }
        if bytes.len() != 12 { return Err("silence source requires three float parameters".into()); }
        let value = |offset| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        Ok(Self { seconds: value(0), minimum_offset: value(4), maximum_offset: value(8) })
    }
    /// 14118a090 rejects unknown parameter identifiers.
    pub fn set(&mut self, parameter: u16, value: f32) -> Result<(), String> {
        match parameter {
            0 => self.seconds = value, 1 => self.minimum_offset = value, 2 => self.maximum_offset = value,
            _ => return Err("unknown silence source parameter".into()),
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Voice {
    pub parameters: Parameters,
    offset: f32,
    initial_seconds: f32,
    sample_rate: u32,
    loops: u16,
    elapsed_bytes: u32,
}
fn clamp_seconds(seconds: f32) -> f32 {
    // COMISS/JNC selects the minimum on unordered input too.
    if seconds >= f32::from_bits(0x3a83126f) { seconds } else { f32::from_bits(0x3a83126f) }
}
fn truncate_i64(value: f32) -> i64 {
    // CVTTSS2SI's indefinite integer for unordered/out-of-range input.
    if value >= i64::MIN as f32 && value < 9223372036854775808.0f32 { value as i64 } else { i64::MIN }
}
impl Voice {
    pub fn new(parameters: Parameters, sample_rate: u32, loops: u16, mut random15: impl FnMut() -> u16) -> Self {
        // Init draws even when both offset endpoints are equal (14118a17f).
        let offset = f32::from(random15()) * f32::from_bits(0x38000100)
            * (parameters.maximum_offset - parameters.minimum_offset) + parameters.minimum_offset;
        Self { parameters, offset, initial_seconds: clamp_seconds(parameters.seconds + offset),
            sample_rate, loops, elapsed_bytes: 0 }
    }
    pub fn duration_ms(&self) -> f32 { self.initial_seconds * 1000.0 * f32::from(self.loops as i16) }
    fn total_bytes(&self) -> u32 {
        (truncate_i64(self.sample_rate as f32 * clamp_seconds(self.parameters.seconds + self.offset)) as u32)
            .wrapping_mul(i32::from(self.loops as i16) as u32).wrapping_mul(4)
    }
    /// Mono float output can be sampled procedurally, without allocating PCM.
    pub fn sample(&self, frame: u64) -> Option<f32> {
        (self.loops == 0 || frame < u64::from(self.total_bytes() / 4)).then_some(0.0)
    }
    /// 14118a270: exact buffer exhaustion reports data-ready; the following
    /// request reports no-more-data. Infinite sources don't advance elapsed.
    pub fn time_skip(&mut self, requested: u16) -> (u16, bool) {
        if self.loops == 0 { return (requested, false); }
        let requested_bytes = u32::from(requested) * 4;
        let total = self.total_bytes();
        if self.elapsed_bytes >= total { return (0, true); }
        let bytes = requested_bytes.min(total - self.elapsed_bytes);
        self.elapsed_bytes += bytes;
        ((bytes / 4) as u16, bytes < requested_bytes)
    }
    /// 14118a340 clears exactly the valid generated samples.
    pub fn execute(&mut self, output: &mut [f32], requested: u16) -> Result<(u16, bool), String> {
        if output.len() < usize::from(requested) { return Err("silence output buffer is too short".into()); }
        let (frames, finished) = self.time_skip(requested);
        output[..usize::from(frames)].fill(0.0);
        Ok((frames, finished))
    }
    pub fn reset(&mut self) { self.elapsed_bytes = 0; }
    pub fn stop_looping(&mut self) { self.loops = 1; }
    pub fn seek(&mut self, frame: u32) -> Result<(), String> {
        if self.loops != 0 {
            let bytes = frame.wrapping_mul(4);
            if bytes >= self.total_bytes() { return Err("silence seek exceeds source duration".into()); }
            self.elapsed_bytes = bytes;
        }
        Ok(())
    }
}
