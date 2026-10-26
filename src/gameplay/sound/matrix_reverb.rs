//! Parameters of effect 0x00730003 (1400c9bb0). Reader/defaults/live edits:
//! 14117e580 / 14117e4b0 / 14117e660. Its DSP is distinct from Room Reverb.
pub const PLUGIN: u32 = 0x00730003;
mod processor;
pub use processor::MatrixReverb;

#[derive(Clone, Debug)]
pub struct Parameters {
    pub decay_time: f32,
    pub hf_ratio: f32,
    pub dry_level: f32,
    pub wet_level: f32,
    pub delay_count: u32,
    /// Seconds, multiplied directly by the processing rate in 14117ed20.
    pub pre_delay: f32,
    pub include_lfe: bool,
    pub delay_mode: u32,
    /// Mode 0 uses the installed executable's table at 141f4baa0. Mode 1
    /// supplies times in milliseconds. Unwritten native slots stay unknown.
    pub custom_delays: [Option<f32>; 16],
    pub structural_dirty: bool,
}

impl Default for Parameters {
    fn default() -> Self {
        Self { decay_time: 4.0, hf_ratio: 2.0,
            dry_level: 10.0f32.powf(f32::from_bits(0xbe19999a)),
            wet_level: 10.0f32.powf(-0.5), delay_count: 8, pre_delay: 0.0,
            include_lfe: true, delay_mode: 0, custom_delays: [None; 16], structural_dirty: true }
    }
}

impl Parameters {
    /// Delay design in 14117ed20. Lengths are sorted after conversion; equal
    /// times remain equal, unlike RoomVerb's adjacent-length exclusion.
    pub fn delay_lengths(&self, rate: u32, defaults: &[f32; 16]) -> Result<Vec<u32>, String> {
        if !matches!(self.delay_count, 4 | 8 | 12 | 16) {
            return Err("matrix reverb has no DSP path for this delay count".into());
        }
        if rate == 0 { return Err("matrix reverb requires a nonzero sample rate".into()); }
        let mut lengths = Vec::with_capacity(self.delay_count as usize);
        for (index, default) in defaults.iter().take(self.delay_count as usize).enumerate() {
            let time = match self.delay_mode {
                0 => *default,
                1 => self.custom_delays[index].ok_or("matrix reverb custom delay was never initialized")?,
                _ => return Err("matrix reverb delay mode leaves native delay slots uninitialized".into()),
            };
            if !time.is_finite() || time < 0.0 { return Err("invalid matrix reverb delay time".into()); }
            lengths.push(super::room_reverb::odd_length(time, rate, None));
        }
        // 1411891f0 subtracts its two u32 operands and returns a signed int.
        lengths.sort_by(|left, right| ((*left).wrapping_sub(*right) as i32).cmp(&0));
        Ok(lengths)
    }

    /// 141189200: longest-line damping limit, then per-line gain and pole.
    pub fn damping(&self, lengths: &[u32], rate: u32) -> Result<Vec<[f32; 2]>, String> {
        if lengths.len() != self.delay_count as usize || lengths.is_empty() || rate == 0 {
            return Err("matrix reverb damping requires its configured delay lines and rate".into());
        }
        let inverse_rate = 1.0 / f64::from(rate);
        let amplitude = |length: u32| f64::from_bits(0x3f50624dd2f1a9fc)
            .powf((f64::from(length) * inverse_rate) / f64::from(self.decay_time));
        let scale = 10.0f64.ln() * 0.25; // 141a59770
        let ratio = 1.0 / f64::from(self.hf_ratio);
        let mut difference = 1.0 - 1.0 / (ratio * ratio);
        let longest = amplitude(*lengths.last().unwrap()).log10();
        if 1.0 < longest * scale * difference { difference = 1.0 / (longest * scale); }
        Ok(lengths.iter().map(|&length| {
            let amplitude = amplitude(length);
            let pole = amplitude.log10() * scale * difference;
            [((1.0 - pole) * amplitude) as f32, pole as f32]
        }).collect())
    }

    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() { return Ok(Self::default()); }
        if bytes.len() < 29 { return Err("truncated matrix reverb parameters".into()); }
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        let delay_count = word(8);
        let delay_mode = word(25);
        if delay_count > 16 { return Err("matrix reverb exceeds its sixteen delay slots".into()); }
        let custom_count = if delay_mode == 1 { delay_count as usize } else { 0 };
        if bytes.len() != 29 + custom_count * 4 { return Err("matrix reverb parameter size does not match its delay mode".into()); }
        let mut custom_delays = [None; 16];
        for (index, time) in custom_delays.iter_mut().take(custom_count).enumerate() {
            *time = Some(f32::from_bits(word(29 + index * 4)));
        }
        Ok(Self { decay_time: f32::from_bits(word(0)), hf_ratio: f32::from_bits(word(4)),
            delay_count, dry_level: 10.0f32.powf(f32::from_bits(word(12)) * 0.05),
            wet_level: 10.0f32.powf(f32::from_bits(word(16)) * 0.05),
            pre_delay: f32::from_bits(word(20)), include_lfe: bytes[24] != 0,
            delay_mode, custom_delays, structural_dirty: true })
    }

    pub fn set(&mut self, id: u16, bytes: &[u8]) -> Result<(), String> {
        if id == 6 {
            self.include_lfe = *bytes.first().ok_or("missing matrix reverb LFE setting")? != 0;
        } else {
            let bits = u32::from_le_bytes(bytes.get(..4).ok_or("missing matrix reverb parameter word")?.try_into().unwrap());
            let value = f32::from_bits(bits);
            match id {
                0 => self.decay_time = value,
                1 => self.hf_ratio = value,
                2 => {
                    if bits > 16 { return Err("matrix reverb exceeds its sixteen delay slots".into()); }
                    self.delay_count = bits;
                },
                3 => self.dry_level = 10.0f32.powf(value * 0.05),
                4 => self.wet_level = 10.0f32.powf(value * 0.05),
                5 => self.pre_delay = value,
                7 => self.delay_mode = bits,
                8..=23 => self.custom_delays[usize::from(id - 8)] = Some(value),
                _ => return Err("matrix reverb parameter exceeds its native storage".into()),
            }
        }
        if !matches!(id, 0 | 1 | 3 | 4) { self.structural_dirty = true; }
        Ok(())
    }
}
