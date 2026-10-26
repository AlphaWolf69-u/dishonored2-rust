//! Mono-to-stereo speaker panning and spread (1410ff970 / 1410fedb0 /
//! 141100b30). The table stores power; output gains are its square roots.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule { Speakers, Headphones }

#[derive(Clone, Copy, Debug)]
pub struct SpeakerAngles {
    pub horizontal: [f32; 3],
    pub elevation: f32,
}

/// 1410ffde0, with an unpacked standard channel mask.
pub fn default_angles(mask: u32) -> SpeakerAngles {
    SpeakerAngles { horizontal: [if mask & !7 == 0 { 45.0 } else { 30.0 },
        if mask & 0x10 != 0 && mask & 0x200 != 0 { 100.0 } else { 110.0 }, 142.5], elevation: 45.0 }
}

/// 141101070: round speaker angles onto the 512-bin circle and return the
/// narrowest speaker interval used to choose the spread sampling density.
pub fn quantize_angles(degrees: &[f32]) -> Result<(Vec<u32>, u32), String> {
    if degrees.is_empty() { return Err("speaker angles are empty".into()); }
    let mut angles = Vec::with_capacity(degrees.len());
    for &angle in degrees {
        if !angle.is_finite() || angle < 0.0 { return Err("invalid speaker angle".into()); }
        let angle = (angle * 512.0 * f32::from_bits(0x3b360b61) + 0.5) as u32;
        if angle > 255 { return Err("speaker angle exceeds the supported hemisphere".into()); }
        angles.push(angle);
    }
    if angles[0] >= 128 { return Err("front speaker angle must be below ninety degrees".into()); }
    let mut minimum = angles[0];
    for pair in angles.windows(2) {
        if pair[1] <= pair[0] { return Err("speaker angles must increase".into()); }
        minimum = minimum.min(pair[1] - pair[0]);
    }
    if angles.len() > 1 { minimum = minimum.min((256 - angles[angles.len() - 1]) * 2); }
    Ok((angles, minimum))
}

pub struct Stereo {
    front: u32,
    density: f32,
    power: [[f32; 2]; 257],
    headphones: [f32; 129],
}

impl Default for Stereo {
    fn default() -> Self {
        // Stereo's 45-degree front angle becomes exactly 64 bins.
        Self::from_front_bin(64)
    }
}

impl Stereo {
    pub fn new(front_degrees: f32) -> Result<Self, String> {
        let (angles, minimum) = quantize_angles(&[front_degrees])?;
        if minimum == 0 { return Err("zero-width speaker interval".into()); }
        Ok(Self::from_front_bin(angles[0]))
    }

    fn from_front_bin(front: u32) -> Self {
        let mut power = [[0.0; 2]; 257];
        // 1410ff970 generates adjacent sine/cosine values by recurrence.
        // Preserve its constants and operation order rather than replacing
        // the authored speaker layout with a generic equal-power pan law.
        let full_circle = f32::from_bits(0x40c90fdb);
        let angle = front as f32 * full_circle * (1.0 / 512.0);
        let (speaker_sin, speaker_cos) = (angle.sin(), angle.cos());
        let step = f32::from_bits(0x3c490fdb);
        let phase = -f32::from_bits(0x3d16cbe4);
        let twice_cos = step.cos() * 2.0;
        let mut previous_sin = (phase + step).sin();
        let mut current_sin = (phase + f32::from_bits(0x3cc90fdb)).sin();
        let mut previous_cos = (phase + step).cos();
        let mut current_cos = (phase + f32::from_bits(0x3cc90fdb)).cos();
        for entry in power.iter_mut().take(front as usize + 1) {
            let next_cos = current_cos * twice_cos - previous_cos;
            let next_sin = current_sin * twice_cos - previous_sin;
            let a = next_cos * speaker_sin;
            let b = next_sin * speaker_cos;
            let second = (b + a) * (b + a);
            let first = (a - b) * (a - b);
            let inverse = 1.0 / (second + first);
            *entry = [inverse * first, inverse * second];
            previous_sin = current_sin;
            current_sin = next_sin;
            previous_cos = current_cos;
            current_cos = next_cos;
        }
        for entry in power.iter_mut().take((256 - front) as usize).skip(front as usize + 1) { *entry = [1.0, 0.0]; }
        for index in (256 - front) as usize..=256 {
            let source = power[256 - index];
            power[index] = [source[1], source[0]];
        }
        // 141100f60's table advances two angle bins per entry, through 128.
        let step = f64::from_bits(0x3f8921fb60000000);
        let headphones = std::array::from_fn(|index| { let sine = (index as f64 * step).sin(); (sine * sine) as f32 });
        Self { front, density: 1.0 / front as f32, power, headphones }
    }

    /// 1410fedb0's stereo branches, adding power for one virtual source.
    fn add(&self, angle: u32, rule: Rule, power: &mut [f32; 2]) {
        if rule == Rule::Headphones {
            let mut index = angle.wrapping_add(128) & 511;
            if index > 255 { index = 512 - index; }
            let left = self.headphones[(index >> 1) as usize];
            power[0] += left;
            power[1] += 1.0 - left;
        } else {
            let angle = angle & 511;
            let absolute = if angle > 256 { 512 - angle } else { angle };
            let entry = self.power[absolute as usize];
            if (self.front < absolute) == (angle < 257) {
                power[0] += entry[0];
                power[1] += entry[1];
            } else {
                power[0] += entry[1];
                power[1] += entry[0];
            }
        }
    }

    /// 141100b30 for a mono input. Spread and focus are authored percentages;
    /// azimuth is the listener-relative right-positive angle in radians.
    pub fn gains(&self, azimuth: f32, spread: f32, focus: f32, rule: Rule) -> [f32; 2] {
        let half_span = spread * f32::from_bits(0x4023d70a);
        let mut center = -azimuth * f32::from_bits(0x3e22f983) * 512.0;
        let count = (half_span * self.density + 1.0) as u32 + 1;
        let inverse = 1.0 / count as f32;
        let half_step = (1.0 - focus * 0.01) * (half_span / count as f32);
        if count & 1 == 0 { center -= half_step; }
        let step = half_step * 2.0;
        let mut angle = (count >> 1) as f32 * step + 512.0 + center;
        let mut power = [0.0; 2];
        for _ in 0..count {
            self.add((angle + 0.5) as i64 as u32, rule, &mut power);
            angle -= step;
        }
        [(inverse * power[0]).sqrt(), (inverse * power[1]).sqrt()]
    }
}
