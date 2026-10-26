//! Parameters of registered effect 0x00760003 (1400c9bb0).
//! Bank reader/defaults: 141168570/141168310; live controls: 141168850.

pub const PLUGIN: u32 = 0x00760003;

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
    /// Front/rear/center/LFE output, followed by center/LFE input levels.
    pub channel_levels: [f32; 6],
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

impl Default for Parameters {
    fn default() -> Self {
        // Constructor defaults from 141168310, including its pre-scaled
        // exponent constants; newer SDK defaults need not match this build.
        Self { decay_time: f32::from_bits(0x3f99999a), hf_damping: 2.25, diffusion: 100.0,
            stereo_width: 180.0,
            filters: std::array::from_fn(|index| FilterBand { gain_db: 0.0,
                frequency: [100.0, 1000.0, 10000.0][index], q: 1.0, insertion: 3, curve: index as u32 }),
            channel_levels: [1.0, 1.0, 1.0, 10.0f32.powf(f32::from_bits(0xc099999a)), 1.0, 0.1],
            reverb_level: 10.0f32.powf(f32::from_bits(0xbf933333)), dry_level: 1.0,
            early_level: 10.0f32.powf(f32::from_bits(0xc099999a)),
            early_reflections: true, early_pattern: 23, early_room_size: 25.0, early_rear_delay: 0.0,
            pre_delay: 40.0, density: 80.0, room_shape: 100.0, quality: 8, tone_controls: false,
            tuning: [8.0, 50.0, 2.0, f32::from_bits(0x3dcccccd), f32::from_bits(0x3f4ccccd),
                66.0, 15.0, 5.0, 40.0, 0.5, 50.0], rtpc_dirty: true, room_dirty: true }
    }
}

impl Parameters {
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
            reverb_level: gain(float(76) - 3.0), dry_level: gain(float(134)), early_level: gain(float(138)),
            early_reflections: bytes[80] != 0, early_pattern: word(81), early_room_size: float(85),
            early_rear_delay: float(89), pre_delay: float(93), density: float(97), room_shape: float(101),
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
            1 => self.early_room_size = value,
            2 => self.early_rear_delay = value,
            3 => self.pre_delay = value,
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
            40 => self.dry_level = gain(value),
            41 => self.early_level = gain(value),
            50..=53 => self.channel_levels[usize::from(id - 50)] = gain(value),
            60..=61 => self.channel_levels[usize::from(id - 60) + 4] = gain(value),
            62 => self.reverb_level = gain(value - 3.0),
            100..=110 => { self.tuning[usize::from(id - 100)] = value; return Ok(()); }
            _ => unreachable!(),
        }
        if matches!(id, 0..=3 | 12..=14 | 21..=22 | 26..=27 | 31..=32 | 40..=41) { self.room_dirty = true; }
        else { self.rtpc_dirty = true; }
        Ok(())
    }
}
