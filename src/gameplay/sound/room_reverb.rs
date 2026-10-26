//! Parameters of registered effect 0x00760003 (1400c9bb0).
//! Bank reader/defaults: 141168570/141168310; live controls: 141168850.

pub const PLUGIN: u32 = 0x00760003;
mod early;
pub use early::EarlyReflections;

#[derive(Clone, Copy, Debug)]
pub struct FilterBand {
    pub gain_db: f32,
    pub frequency: f32,
    pub q: f32,
    pub insertion: u32,
    pub curve: u32,
}

#[derive(Clone, Debug)]
pub struct Parameters {
    pub decay_time: f32,
    pub hf_damping: f32,
    pub diffusion: f32,
    pub stereo_width: f32,
    pub filters: [FilterBand; 3],
    /// Front/rear/center/LFE output levels.
    pub channel_levels: [f32; 4],
    /// Center/LFE levels used by the input downmix (14116a600).
    pub input_levels: [f32; 2],
    pub reverb_level: f32,
    pub dry_level: f32,
    pub early_level: f32,
    pub early_reflections: bool,
    pub early_pattern: u32,
    pub early_room_size: f32,
    pub early_rear_delay: f32,
    pub pre_delay: f32,
    pub density: f32,
    pub room_shape: f32,
    pub quality: u32,
    pub tone_controls: bool,
    pub tuning: [f32; 11],
    pub rtpc_dirty: bool,
    pub room_dirty: bool,
}

fn gain(db: f32) -> f32 {
    // The plug-in calls the CRT powf (1411b41e0), independently of the
    // sound engine's approximate dB conversion (1410cb000).
    10.0f32.powf(db * f32::from_bits(0x3d4ccccd))
}

/// 141171d80's energy-normalized input mix. Wwise stores LFE last,
/// independently of its position in the standard speaker-mask bits.
pub struct InputMix {
    weights: Vec<f32>,
}
impl InputMix {
    pub fn new(mask: u32, mut center: f32, mut lfe: f32) -> Self {
        if 1.0 < center { center = 1.0; }
        if 1.0 < lfe { lfe = 1.0; }
        let mut ordinary = 0.0;
        for bit in [1, 2, 0x200, 0x400, 0x10, 0x20] {
            if mask & bit != 0 { ordinary += 1.0; }
        }
        let center_energy = if mask & 4 != 0 { center * center } else { 0.0 };
        let lfe_energy = if mask & 8 != 0 { lfe * lfe } else { 0.0 };
        let normalization = (1.0 / (center_energy + ordinary + lfe_energy)).sqrt();
        let mut weights = Vec::new();
        for bit in 0..32 {
            let channel = 1u32 << bit;
            if mask & channel == 0 || channel == 8 { continue; }
            weights.push(if channel == 4 { normalization * center } else { normalization });
        }
        if mask & 8 != 0 { weights.push(normalization * lfe); }
        Self { weights }
    }

    pub fn frame(&self, channels: &[f32]) -> Result<f32, String> {
        if channels.len() != self.weights.len() { return Err("reverb input does not match its speaker mask".into()); }
        let mut result = 0.0;
        for (sample, gain) in channels.iter().zip(&self.weights) { result = sample * gain + result; }
        Ok(result)
    }
}

impl Default for Parameters {
    fn default() -> Self {
        // Constructor defaults from 141168310, including its pre-scaled
        // exponent constants; newer SDK defaults need not match this build.
        Self { decay_time: f32::from_bits(0x3f99999a), hf_damping: 2.25, diffusion: 100.0,
            stereo_width: 180.0,
            filters: std::array::from_fn(|index| FilterBand { gain_db: 0.0,
                frequency: [100.0, 1000.0, 10000.0][index], q: 1.0, insertion: 3, curve: index as u32 }),
            channel_levels: [1.0, 1.0, 1.0, 10.0f32.powf(f32::from_bits(0xc099999a))],
            input_levels: [1.0, 10.0f32.powf(f32::from_bits(0xc099999a))],
            reverb_level: 10.0f32.powf(f32::from_bits(0xbf933333)), dry_level: 1.0, early_level: 0.1,
            early_reflections: true, early_pattern: 23, early_room_size: 0.0, early_rear_delay: 40.0,
            pre_delay: 25.0, density: 80.0, room_shape: 100.0, quality: 8, tone_controls: false,
            tuning: [8.0, 50.0, 2.0, f32::from_bits(0x3dcccccd), f32::from_bits(0x3f4ccccd),
                66.0, 15.0, 5.0, 40.0, 0.5, 50.0], rtpc_dirty: true, room_dirty: true }
    }
}

impl Parameters {
    pub fn early_reflection_unit(&self, patterns: &crate::assets::sound_effects::ReflectionPatterns,
        sample_rate: u32, speaker_mask: u32) -> Result<Option<EarlyReflections>, String>
    {
        // 141168ff0 disables ER for an LFE-only format even if authored on.
        if !self.early_reflections || speaker_mask == 8 { return Ok(None); }
        EarlyReflections::new(patterns.get(self.early_pattern)?, self.early_room_size, sample_rate).map(Some)
    }

    pub fn input_mix(&self, mask: u32) -> InputMix { InputMix::new(mask, self.input_levels[0], self.input_levels[1]) }

    /// 14116f660 bypasses a zero-frame pre-delay; a nonzero request uses
    /// the delay constructor's four-sample minimum, even below four frames.
    pub fn pre_delay_line(&self, sample_rate: u32) -> Result<Option<super::delay::Delay>, String> {
        let frames = crate::float::truncate_i64(self.pre_delay * 0.001 * sample_rate as f32) as u32;
        if frames == 0 { Ok(None) } else { super::delay::Delay::new(frames).map(Some) }
    }

    /// Four input diffusion stages, allocated by 14116ef80. The milliseconds
    /// distribution and odd-length search are 14116dbf0/14116de60.
    pub fn input_diffusers(&self, sample_rate: u32) -> Result<[super::delay::AllPass; 4], String> {
        let mut seed = 0x002e_e007u32;
        let mut base = self.tuning[6];
        let mut times = [0.0; 4];
        for time in &mut times {
            seed = seed.wrapping_mul(0x0bb3_8435).wrapping_add(0x3619_636b);
            *time = self.tuning[7] * 0.01 * base * seed as f32 * f32::from_bits(0x2f800000) + base;
            base = base * self.tuning[5] * 0.01;
        }
        if times.iter().any(|time| !time.is_finite()) { return Err("non-finite reverb diffusion delay distribution".into()); }
        times.sort_by(|a, b| if a < b { std::cmp::Ordering::Less } else if b < a { std::cmp::Ordering::Greater } else { std::cmp::Ordering::Equal });
        let lengths = odd_lengths(&times, sample_rate);
        let maximum = f32::from_bits(0x3f1e3737);
        let mut stages = Vec::new();
        for (index, length) in lengths.into_iter().enumerate() {
            let coefficient = self.diffusion * 0.01 * maximum * 4.0 - (3 - index) as f32 * maximum;
            let coefficient = if coefficient <= 0.0 { 0.0 } else if maximum <= coefficient { maximum } else { coefficient };
            stages.push(super::delay::AllPass::new(length, coefficient)?);
        }
        stages.try_into().map_err(|_| "reverb input diffusion stage count differs from four".into())
    }

    pub fn read(bytes: &[u8]) -> Result<Self, String> {
        if bytes.is_empty() { return Ok(Self::default()); }
        if bytes.len() != 186 { return Err(format!("room reverb parameters require 186 bytes, got {}", bytes.len())); }
        let word = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let float = |offset| f32::from_bits(word(offset));
        Ok(Self { decay_time: float(0), hf_damping: float(4), diffusion: float(8), stereo_width: float(12),
            filters: std::array::from_fn(|index| FilterBand { gain_db: float(16 + index * 12),
                frequency: float(20 + index * 12), q: float(24 + index * 12),
                insertion: word(110 + index * 8), curve: word(114 + index * 8) }),
            channel_levels: std::array::from_fn(|index| gain(float(52 + index * 4))),
            reverb_level: gain(float(76) - 3.0), dry_level: gain(float(68)), early_level: gain(float(72)),
            input_levels: [gain(float(134)), gain(float(138))],
            early_reflections: bytes[80] != 0, early_pattern: word(81), pre_delay: float(85),
            early_room_size: float(89), early_rear_delay: float(93), density: float(97), room_shape: float(101),
            quality: word(105), tone_controls: bytes[109] != 0,
            tuning: std::array::from_fn(|index| float(142 + index * 4)), rtpc_dirty: true, room_dirty: true })
    }

    /// Control payloads preserve their union representation. Pattern, quality
    /// and enum controls carry integers; boolean controls need only one byte.
    /// Unrecognized IDs succeed without changing state, as in 141168850.
    pub fn set(&mut self, id: u16, bytes: &[u8]) -> Result<(), String> {
        if matches!(id, 4 | 20) {
            let value = *bytes.first().ok_or("missing room reverb boolean payload")? != 0;
            if id == 4 { self.early_reflections = value; } else { self.tone_controls = value; }
            self.room_dirty = true;
            return Ok(());
        }
        if !matches!(id, 0..=3 | 10..=16 | 21..=35 | 40..=41 | 50..=53 | 60..=62 | 100..=110) { return Ok(()); }
        let word = u32::from_le_bytes(bytes.get(..4).ok_or("missing room reverb control word")?.try_into().unwrap());
        let value = f32::from_bits(word);
        match id {
            0 => self.early_pattern = word,
            1 => self.pre_delay = value,
            2 => self.early_room_size = value,
            3 => self.early_rear_delay = value,
            10 => self.decay_time = value,
            11 => self.hf_damping = value,
            12 => self.density = value,
            13 => self.room_shape = value,
            14 => self.quality = word,
            15 => self.diffusion = value,
            16 => self.stereo_width = value,
            21..=35 => {
                let index = usize::from((id - 21) / 5);
                match (id - 21) % 5 {
                    0 => self.filters[index].insertion = word,
                    1 => self.filters[index].curve = word,
                    2 => self.filters[index].gain_db = value,
                    3 => self.filters[index].frequency = value,
                    _ => self.filters[index].q = value,
                }
            }
            40..=41 => self.input_levels[usize::from(id - 40)] = gain(value),
            50..=53 => self.channel_levels[usize::from(id - 50)] = gain(value),
            60 => self.dry_level = gain(value),
            61 => self.early_level = gain(value),
            62 => self.reverb_level = gain(value - 3.0),
            100..=110 => { self.tuning[usize::from(id - 100)] = value; return Ok(()); }
            _ => unreachable!(),
        }
        if matches!(id, 0..=3 | 12..=14 | 21..=22 | 26..=27 | 31..=32 | 40..=41) { self.room_dirty = true; }
        else { self.rtpc_dirty = true; }
        Ok(())
    }
}

/// The divisor bound is computed once from the initial odd candidate and
/// stays fixed during retries. Only the immediately previous length is
/// excluded. In particular the original accepts one; a generic next-prime
/// routine would change these low-length cases.
fn odd_lengths(times: &[f32], sample_rate: u32) -> Vec<u32> {
    let mut output = Vec::with_capacity(times.len());
    for &time in times {
        let mut candidate = crate::float::truncate_i64(time * 0.001 * sample_rate as f32) as u32;
        if candidate & 1 == 0 { candidate = candidate.wrapping_add(1); }
        let limit = (f64::from(candidate).sqrt() as i32) + 1;
        loop {
            let mut divisor = 3u32;
            let mut divisible = false;
            while (divisor as i32) < limit {
                if candidate % divisor == 0 { divisible = true; break; }
                divisor += 2;
            }
            if !divisible && output.last().copied() != Some(candidate) { break; }
            candidate = candidate.wrapping_add(2);
        }
        output.push(candidate);
    }
    output
}
