//! The64-byte motion-property records used by the world's property library.
//! Preserve all words for the source's bitwise record deduplication.
use super::signal::Signal;
use super::{constraint_solver::motion::MotionProperties, deactivation::DeactivationProfile};
use crate::float::multiply;

/// ca5510's retained table. Added observers use the shared Signal algorithm;
/// records retain source order, free-list words and full bitwise identity.
pub struct MotionPropertiesLibrary<S, C> {
    records: Vec<MotionProfile>,
    free_head: i32,
    pub added: Signal<S, C>,
}

impl<S, C> Default for MotionPropertiesLibrary<S, C> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            free_head: -1,
            added: Signal::default(),
        }
    }
}

impl<S, C> MotionPropertiesLibrary<S, C> {
    pub fn get(&self, id: u16) -> Option<&MotionProfile> {
        self.records.get(usize::from(id))
    }

    ///18d020 resolves the current body's motion and its property-table ID.
    /// A missing owner/record is an error rather than an assumed unit scale.
    pub fn body_velocity_scale(
        &self,
        bodies: &super::body_storage::BodyStorage,
        motions: &super::motion_storage::MotionStorage,
        id: u32,
    ) -> Result<f32, String> {
        let body = bodies
            .get_by_index(id & 0xffffff)
            .ok_or("velocity-scale body is missing")?;
        let motion = motions
            .get(body.motion_id)
            .ok_or("velocity-scale motion is missing")?;
        Ok(self
            .get(motion.properties_id)
            .ok_or("motion-property record is missing")?
            .velocity_scale())
    }
}

impl<S: Copy + Eq, C: Copy + Eq> MotionPropertiesLibrary<S, C> {
    /// ca5b90 doubles the retained slot count, adding at least8. Each new free
    /// record starts with profile1 and checksFF; word0 links the ascending chain.
    fn grow(&mut self) -> Result<(), String> {
        let old = self.records.len();
        let count = old
            .checked_mul(2)
            .and_then(|n| old.checked_add(8).map(|m| n.max(m)))
            .ok_or("motion-property slot count overflow")?;
        if count > 0x10000 {
            return Err("motion-property table exceeds16-bit IDs".into());
        }
        self.records
            .try_reserve(count - old)
            .map_err(|e| format!("motion-property allocation failed: {e}"))?;
        let mut free = MotionProfile::from_words([0; 16]);
        free.configure(1, f32::from_bits(0x411c_f5c3));
        free.words[14] = (free.words[14] & 0xffff_ff00) | 0xff;
        for i in old..count {
            free.words[0] = if i + 1 == count {
                self.free_head as u32
            } else {
                (i + 1) as u32
            };
            self.records.push(free);
        }
        self.free_head = old as i32;
        Ok(())
    }

    /// ca5600 compares all16 words only for sharing0, including retained free
    /// records. A deduplicated ID returns before allocation or added observers.
    pub fn add(
        &mut self,
        profile: MotionProfile,
        mut dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16),
    ) -> Result<u16, String> {
        if profile.sharing() == 0 {
            if let Some(index) = self.records.iter().position(|record| *record == profile) {
                return Ok(index as u16);
            }
        }
        if self.free_head < 0 {
            self.grow()?;
        }
        let id = self.free_head as u16;
        let slot = self
            .records
            .get_mut(usize::from(id))
            .ok_or("motion-property free slot is missing")?;
        self.free_head = slot.words[0] as i32;
        *slot = profile;
        self.added.emit(&id, |signal, subscriber, callback, &id| {
            dispatch(signal, subscriber, callback, id);
        });
        Ok(id)
    }

    /// ca5550 resets the table's live count/free chain but retains its storage
    /// and subscribers, then installs unique profiles0..4 with current gravity.
    pub fn initialize(
        &mut self,
        gravity: f32,
        mut dispatch: impl FnMut(&mut Signal<S, C>, S, C, u16),
    ) -> Result<(), String> {
        self.records.clear();
        self.free_head = -1;
        for kind in 0..5 {
            let mut profile = MotionProfile::from_words([0; 16]);
            profile.configure(kind, gravity);
            profile.set_sharing(1);
            self.add(profile, &mut dispatch)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionProfile {
    words: [u32; 16],
}

impl MotionProfile {
    pub fn from_words(words: [u32; 16]) -> Self {
        Self { words }
    }
    pub fn words(&self) -> &[u32; 16] {
        &self.words
    }
    pub fn sharing(&self) -> u32 {
        self.words[0]
    }
    pub fn set_sharing(&mut self, sharing: u32) {
        self.words[0] = sharing;
    }

    /// c9f050 clears only bytes4..3f, retaining the caller's sharing word.
    /// Unknown kinds still receive deactivation settings but no preset/time1.
    pub fn configure(&mut self, kind: i32, gravity: f32) {
        self.words[1..].fill(0);
        match kind {
            0 => (),
            1 | 2 => {
                self.words[4] = 200f32.to_bits();
                self.words[5] = 100f32.to_bits();
                if kind == 1 {
                    self.words[2] = 1f32.to_bits();
                }
            }
            3 => {
                self.words[6] = 30f32.to_bits();
                self.words[7] = 30f32.to_bits();
            }
            4 => {
                self.words[2] = 1f32.to_bits();
                self.words[4] = 100f32.to_bits();
                self.words[5] = 10f32.to_bits();
                self.words[6] = 0x3c23_d70a;
                self.words[7] = 0x3f00_0000;
            }
            _ => {
                self.write_deactivation(DeactivationProfile::default());
                return;
            }
        }
        self.words[3] = 1f32.to_bits();
        self.set_preset(if kind == 4 { 3 } else { 0 }, gravity);
        self.write_deactivation(DeactivationProfile::default());
    }

    /// c9f160. Machine UCOMISS/JNZ substitutes10 for both zero and unordered
    /// gravity. Only the two reduction-control words change.
    pub fn set_preset(&mut self, preset: i32, gravity: f32) {
        let gravity = f32::from_bits(gravity.to_bits() & 0x7fff_ffff);
        let gravity = if gravity == 0. || gravity.is_nan() {
            10.
        } else {
            gravity
        };
        let (threshold, scale) = match preset {
            1 => (0x3dcc_cccd, 0x3cf5_c28f),
            2 => (0x3e2e_147b, 0x3d4c_cccd),
            3 => (0x3e4c_cccd, 0x3dcc_cccd),
            4 => (0x3ecc_cccd, 0x3e4c_cccd),
            _ => (0, 0),
        };
        self.words[8] = threshold;
        self.words[9] = multiply(f32::from_bits(scale), gravity).to_bits();
    }

    fn write_deactivation(&mut self, p: DeactivationProfile) {
        self.words[10] = p.linear_squared.to_bits();
        self.words[11] = p.angular_squared.to_bits();
        self.words[12] = p.inverse_scale.to_bits();
        self.words[13] =
            u32::from(p.coherence_high as u16) | (u32::from(p.coherence_low as u16) << 16);
        self.words[14] = u32::from(p.checks)
            | (u32::from(p.slow) << 8)
            | (u32::from(p.still) << 16)
            | (u32::from(p.spike_margin) << 24);
        self.words[15] = (self.words[15] & 0xffff_ff00) | u32::from(p.spike);
    }

    pub fn deactivation(&self) -> DeactivationProfile {
        DeactivationProfile {
            linear_squared: f32::from_bits(self.words[10]),
            angular_squared: f32::from_bits(self.words[11]),
            inverse_scale: f32::from_bits(self.words[12]),
            coherence_high: self.words[13] as i16,
            coherence_low: (self.words[13] >> 16) as i16,
            checks: self.words[14] as u8,
            slow: (self.words[14] >> 8) as u8,
            still: (self.words[14] >> 16) as u8,
            spike_margin: (self.words[14] >> 24) as u8,
            spike: self.words[15] as u8,
        }
    }

    pub fn set_coherence(&mut self, slow: f32, high: f32, low: f32, margin: u8, spike: f32) {
        let mut profile = self.deactivation();
        profile.set_coherence(slow, high, low, margin, spike);
        self.write_deactivation(profile);
    }

    /// The source18d020 reads exactly this+0c word through motion.properties_id.
    pub fn velocity_scale(&self) -> f32 {
        f32::from_bits(self.words[3])
    }

    pub fn solver_properties(&self) -> MotionProperties {
        MotionProperties {
            gravity_factor: f32::from_bits(self.words[2]),
            maximum_linear_velocity: f32::from_bits(self.words[4]),
            maximum_angular_velocity: f32::from_bits(self.words[5]),
            linear_damping: f32::from_bits(self.words[6]),
            angular_damping: f32::from_bits(self.words[7]),
            linear_threshold: f32::from_bits(self.words[8]),
            linear_reduction: f32::from_bits(self.words[9]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_match_executed_presets_and_complete_records() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/motion_profiles.txt").lines() {
            let w: Vec<_> = row
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let mut profile = MotionProfile::from_words(w[3..19].try_into().unwrap());
            if w[0] == 0 {
                profile.configure(w[1] as i32, f32::from_bits(w[2]));
            } else {
                profile.set_preset(w[1] as i32, f32::from_bits(w[2]));
            }
            assert_eq!(profile.words(), &w[19..], "case{count}");
            count += 1;
        }
        assert_eq!(count, 2048);
    }

    #[test]
    fn coherence_update_matches_executed_full_records() {
        let mut count = 0;
        for row in include_str!("../../tests/fixtures/motion_coherence.txt").lines() {
            let w: Vec<_> = row
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let mut profile = MotionProfile::from_words(w[5..21].try_into().unwrap());
            profile.set_coherence(
                f32::from_bits(w[0]),
                f32::from_bits(w[1]),
                f32::from_bits(w[2]),
                w[4] as u8,
                f32::from_bits(w[3]),
            );
            assert_eq!(profile.words(), &w[21..], "case{count}");
            count += 1;
        }
        assert_eq!(count, 1024);
    }

    #[test]
    fn library_matches_complete_executed_initialization_addition_and_growth() {
        let mut library = MotionPropertiesLibrary::<u32, u32>::default();
        let mut count = 0;
        for (case, row) in include_str!("../../tests/fixtures/motion_properties_library.txt")
            .lines()
            .enumerate()
        {
            if case % 32 == 0 {
                library = MotionPropertiesLibrary::default();
                library.added.subscribe(11, 12);
            }
            let w: Vec<_> = row
                .split_whitespace()
                .map(|s| u32::from_str_radix(s, 16).unwrap())
                .collect();
            let mut events = Vec::new();
            let dispatch = |_: &mut Signal<_, _>, subscriber, callback, id| {
                assert_eq!((subscriber, callback), (11, 12));
                events.push(u32::from(id));
            };
            let id = if w[0] == 0 {
                library.initialize(f32::from_bits(w[1]), dispatch).unwrap();
                0xffff
            } else {
                u32::from(
                    library
                        .add(
                            MotionProfile::from_words(w[2..18].try_into().unwrap()),
                            dispatch,
                        )
                        .unwrap(),
                )
            };
            let mut actual = vec![
                id,
                library.records.len() as u32,
                library.free_head as u32,
                events.len() as u32,
            ];
            actual.extend(events);
            actual.extend(library.records.iter().flat_map(|p| p.words));
            assert_eq!(actual, w[18..], "case{case}");
            count += 1;
        }
        assert_eq!(count, 512);
    }
}
