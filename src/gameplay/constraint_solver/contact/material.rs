//! Packed contact material pair coefficients (140d14840).
//! Inputs are borrowed from the world's canonical MaterialLibrary; this module
//! neither resolves declarations nor retains a second material cache.
use super::{inverse_length_estimate, max, min};
use crate::gameplay::materials::MaterialRecord;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PairMaterial {
    /// Actual cache+34, +36, +38 upper-half floats.
    pub dynamic_friction: u16,
    pub static_friction_delta: u16,
    pub restitution: u16,
}

fn byte(record: &MaterialRecord, offset: usize) -> u8 {
    (record.words[offset / 4] >> ((offset % 4) * 8)) as u8
}
fn half(record: &MaterialRecord, offset: usize) -> f32 {
    let word = (record.words[offset / 4] >> ((offset % 4) * 8)) as u16;
    f32::from_bits(u32::from(word) << 16)
}
fn combine(a: f32, b: f32, policy: u8) -> f32 {
    match policy {
        0 => {
            let product = b * a;
            if product <= 0.0 {
                0.0
            } else {
                inverse_length_estimate(product) * product
            }
        }
        1 => min(a, b),
        2 => max(a, b),
        _ => (b + a) * 0.5,
    }
}
fn pack_signed(value: f32) -> u16 {
    ((value.to_bits() as i32) >> 16).clamp(i16::MIN as i32, i16::MAX as i32) as u16
}
fn pack_unsigned(value: f32) -> u16 {
    ((value.to_bits() >> 16) as i32).clamp(i16::MIN as i32, i16::MAX as i32) as u16
}

impl PairMaterial {
    pub(crate) fn from_records(a: &MaterialRecord, b: &MaterialRecord) -> Self {
        // MaterialRecord::words starts at native material object+28.
        let friction_policy = byte(a, 16).max(byte(b, 16));
        let restitution_policy = byte(a, 17).max(byte(b, 17));
        let friction = combine(half(a, 10), half(b, 10), friction_policy);
        let static_friction = max(half(a, 12), half(b, 12));
        let restitution = combine(half(a, 14), half(b, 14), restitution_policy);
        Self {
            dynamic_friction: pack_signed(friction),
            static_friction_delta: pack_unsigned(max(static_friction - friction, 0.0)),
            restitution: pack_unsigned(restitution),
        }
    }
    pub(crate) fn base_friction(self) -> f32 {
        f32::from_bits(u32::from(self.dynamic_friction) << 16)
    }
    pub(crate) fn friction_delta(self) -> f32 {
        f32::from_bits(u32::from(self.static_friction_delta) << 16)
    }
    pub(crate) fn restitution(self) -> f32 {
        f32::from_bits(u32::from(self.restitution) << 16)
    }
}
