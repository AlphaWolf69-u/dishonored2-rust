//! Per-voice random property sampling (1410dbb30). Random values are retained
//! for a voice's lifetime while its non-randomized RTPC values can change.
use crate::assets::wwise::MediaProperties;

#[derive(Clone, Copy, Debug, Default)]
pub struct RandomProperties {
    pub volume_db: f32,
    pub makeup_db: f32,
    pub pitch_cents: f32,
    pub low_pass: f32,
    pub high_pass: f32,
}

impl RandomProperties {
    pub fn sample(properties: &MediaProperties, mut random15: impl FnMut() -> u16) -> Self {
        let mut values = [0.0; 5];
        for node in &properties.nodes {
            // The getter queries these property IDs in this order, rather
            // than using the serialized order of the modifier bundle. Other
            // serialized ranges, including bus properties 5/0x17, are not
            // sampled by this source getter (1410dbb30).
            for (slot, property) in [0, 2, 3, 4, 0x21].into_iter().enumerate() {
                let Some(range) = node.ranges.iter().find(|range| range.property == property) else { continue };
                let difference = range.max - range.min;
                let offset = if difference == 0.0 { 0.0 } else {
                    (f64::from(random15()) * f64::from_bits(0x3f00002000400080) * f64::from(difference)) as f32
                };
                values[slot] = offset + range.min + values[slot];
            }
        }
        Self { volume_db: values[0], pitch_cents: values[1], low_pass: values[2],
            high_pass: values[3], makeup_db: values[4] }
    }
}
