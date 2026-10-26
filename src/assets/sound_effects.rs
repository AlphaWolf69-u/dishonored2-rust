//! Installed RoomVerb early-reflection tables, referenced by 14116f220.
//! Pattern samples remain in the user's executable rather than this repository.
use super::executable::{word, Image};
use std::{collections::HashMap, path::Path};

#[derive(Clone, Copy, Debug)]
pub struct ReflectionTap {
    pub milliseconds: f32,
    pub gain: f32,
}

#[derive(Clone, Debug)]
pub struct ReflectionPattern {
    pub left: Vec<ReflectionTap>,
    pub right: Vec<ReflectionTap>,
}

pub struct ReflectionPatterns(HashMap<u32, ReflectionPattern>);
impl ReflectionPatterns {
    pub fn load(game_base: &Path, indices: impl IntoIterator<Item = u32>) -> Result<Self, String> {
        let executable = game_base.parent().ok_or("game asset folder has no parent")?.join("Dishonored2.exe");
        let bytes = std::fs::read(&executable).map_err(|error| format!("{}: {error}", executable.display()))?;
        let image = Image::parse(&bytes)?;
        // Room parameter virtual28 must still name the audited bank reader.
        // This checks the layout without rejecting unrelated host capture patches.
        if image.base != 0x140000000 || word(image.read(0x1f44770, 8)?, 0, 8)? != image.base + 0x1168570 {
            return Err("unsupported room reverb executable layout".into());
        }
        let read_taps = |count: u32, address: u64| -> Result<Vec<ReflectionTap>, String> {
            if !(1..=64).contains(&count) { return Err("reflection pattern tap count is outside 1..64".into()); }
            let rva = address.checked_sub(image.base).and_then(|rva| u32::try_from(rva).ok())
                .ok_or("reflection tap address is outside the executable")?;
            if image.executable_rva(rva) { return Err("reflection taps reference executable code".into()); }
            let data = image.read(rva, count as usize * 8)?;
            let mut taps = Vec::new();
            for tap in data.chunks_exact(8) {
                let milliseconds = f32::from_bits(word(tap, 0, 4)? as u32);
                let gain = f32::from_bits(word(tap, 4, 4)? as u32);
                if !milliseconds.is_finite() || !gain.is_finite() {
                    return Err("non-finite installed reflection pattern".into());
                }
                if taps.last().is_some_and(|previous: &ReflectionTap| previous.milliseconds > milliseconds) {
                    return Err("installed reflection pattern times are not ordered".into());
                }
                taps.push(ReflectionTap { milliseconds, gain });
            }
            Ok(taps)
        };
        let mut patterns = HashMap::new();
        for index in indices {
            if patterns.contains_key(&index) { continue; }
            let rva = index.checked_mul(0x28).and_then(|offset| 0x1f4a3b8u32.checked_add(offset))
                .ok_or("reflection pattern index overflow")?;
            let row = image.read(rva, 0x28)?;
            let left = read_taps(word(row, 0, 4)? as u32, word(row, 8, 8)?)?;
            let right = read_taps(word(row, 0x10, 4)? as u32, word(row, 0x18, 8)?)?;
            patterns.insert(index, ReflectionPattern { left, right });
        }
        Ok(Self(patterns))
    }

    pub fn get(&self, index: u32) -> Result<&ReflectionPattern, String> {
        self.0.get(&index).ok_or_else(|| format!("reflection pattern {index} is not loaded"))
    }
}
