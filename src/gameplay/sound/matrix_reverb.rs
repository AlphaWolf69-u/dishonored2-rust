//! Parameters of effect 0x00730003 (1400c9bb0). Reader/defaults/live edits:
//! 14117e580 / 14117e4b0 / 14117e660. Its DSP is distinct from Room Reverb.
pub const PLUGIN: u32 = 0x00730003;

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
