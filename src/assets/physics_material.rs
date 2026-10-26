//! Material packing constants read directly from the audited installed image.
//! The compression table stays with the installation, not in rewrite assets.
use super::executable::{word, Image};
use std::path::Path;

pub(crate) struct MaterialConstants {
    pub distances: [u16; 256],
    pub maximum_distance: f32,
    pub trigger_distance: f32,
    pub contact_tolerance: u16,
    pub material_limit: u16,
}
impl MaterialConstants {
    pub fn load(game_base: &Path) -> Result<Self, String> {
        let path = game_base
            .parent()
            .ok_or("asset directory has no installation parent")?
            .join("Dishonored2.exe");
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if blake3::hash(&bytes).to_hex().as_str() != super::class_metadata::IMAGE_HASH {
            return Err("material constants require the audited installed executable".into());
        }
        let image = Image::parse(&bytes)?;
        let table = image.read(0x1e9b130, 512)?;
        let float = |rva| -> Result<f32, String> {
            Ok(f32::from_bits(word(image.read(rva, 4)?, 0, 4)? as u32))
        };
        Ok(Self {
            distances: std::array::from_fn(|i| {
                u16::from_le_bytes(table[i * 2..i * 2 + 2].try_into().unwrap())
            }),
            maximum_distance: float(0x1ad2738)?,
            trigger_distance: float(0x1ad3814)?,
            contact_tolerance: (float(0x1ab3814)?.to_bits() >> 16) as u16,
            material_limit: (float(0x1abf154)?.to_bits() >> 16) as u16,
        })
    }

    /// ae3630's bounded search, including its unordered/NaN comparisons.
    pub fn pack_distance(&self, value: f32) -> u8 {
        let decode = |index: usize| {
            if self.distances[index] == 0 {
                0.
            } else {
                f32::from_bits((u32::from(self.distances[index]) + 0x3b800) << 12)
            }
        };
        if !(decode(1) <= value) {
            return 0;
        }
        let mut lower = 0;
        let mut upper = 256;
        let mut middle = 128;
        for _ in 0..7 {
            if decode(middle) < value {
                lower = middle;
            } else {
                upper = middle;
            }
            middle = (lower + upper) >> 1;
        }
        if decode(middle) < value && middle < 255 {
            middle += 1;
        }
        middle as u8
    }
}
