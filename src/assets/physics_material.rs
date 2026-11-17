//! Material packing constants read directly from the audited installed image.
//! The compression table stays with the installation, not in rewrite assets.
use super::executable::{word, Image};
use std::{path::Path, sync::{Arc, OnceLock}};

pub(crate) struct MaterialConstants {
    /// 140cb07d0's authored face min-half-angle decode table (1423f0f30).
    /// Shared with the level's contact/material owner, never per body.
    pub face_half_angle_cosines: [f32; 256],
    pub distances: [u16; 256],
    pub maximum_distance: f32,
    pub trigger_distance: f32,
    pub contact_tolerance: u16,
    pub material_limit: u16,
}
impl MaterialConstants {
    pub(crate) fn acquire(catalog: &crate::archive::Catalog) -> Result<Arc<Self>, String> {
        static MODE: OnceLock<(bool, bool)> = OnceLock::new();
        let (separate, trace) = *MODE.get_or_init(|| (
            std::env::var_os("DH2_DEBUG_SEPARATE_MATERIAL_CONSTANTS").is_some_and(|value| value == "1"),
            std::env::var_os("DH2_DEBUG_MATERIAL_CONSTANTS").is_some_and(|value| value == "1"),
        ));
        let start = trace.then(crate::loading::thread_cycles).flatten();
        let mut loaded = false;
        let mut load = || {
            let constants = Self::load(catalog.base()).map(Arc::new)?;
            loaded = true;
            Ok::<_, String>(constants)
        };
        let constants = if separate {
            load()?
        } else {
            let mut owner = catalog.material_constants.lock()
                .map_err(|_| "physics material constants owner is poisoned")?;
            if owner.is_none() { *owner = Some(load()?); }
            owner.as_ref().unwrap().clone()
        };
        if trace {
            let cycles = start.zip(crate::loading::thread_cycles())
                .map_or(0, |(start, end)| end.saturating_sub(start));
            let mut signature = blake3::Hasher::new();
            for value in constants.distances { signature.update(&value.to_le_bytes()); }
            for value in constants.face_half_angle_cosines { signature.update(&value.to_bits().to_le_bytes()); }
            for value in [constants.maximum_distance, constants.trigger_distance] {
                signature.update(&value.to_bits().to_le_bytes());
            }
            for value in [constants.contact_tolerance, constants.material_limit] {
                signature.update(&value.to_le_bytes());
            }
            let source_bytes = if loaded {
                catalog.base().parent().and_then(|path| std::fs::metadata(path.join("Dishonored2.exe")).ok())
                    .map_or(0, |metadata| metadata.len())
            } else { 0 };
            eprintln!("MATERIAL_CONSTANTS_ACQUIRE catalog={} identity={:p} loaded={loaded} source_bytes={source_bytes} cycles={cycles} signature={}",
                catalog.identity(), Arc::as_ptr(&constants), signature.finalize());
        }
        Ok(constants)
    }

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
        let face_cosines = image.read(0x23f0f30, 1024)?;
        let float = |rva| -> Result<f32, String> {
            Ok(f32::from_bits(word(image.read(rva, 4)?, 0, 4)? as u32))
        };
        Ok(Self {
            face_half_angle_cosines: std::array::from_fn(|i| f32::from_bits(u32::from_le_bytes(face_cosines[i*4..i*4+4].try_into().unwrap()))),
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

impl Drop for MaterialConstants {
    fn drop(&mut self) {
        if std::env::var_os("DH2_DEBUG_MATERIAL_CONSTANTS").is_some_and(|value| value == "1") {
            eprintln!("MATERIAL_CONSTANTS_RETIRE identity={:p}", self);
        }
    }
}
