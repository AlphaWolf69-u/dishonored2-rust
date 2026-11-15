//! Player-wide body-part request counts, independent of the active mode.
//! 1408029e0/a00 own hard requests at player+2e4; 1408029f0/a10
//! own code requests at +2e9. 1408035c0/600 and 140854890 admit an
//! authored part mask only when no hard request intersects it, and either
//! no code request intersects it or m_unlocksCodeLockedBodyParts is set.
//!
//! Counts are bytes in the executable. One owner adds/removes each request;
//! mode switches must not reset requests belonging to other components.

pub const BODY: usize = 0;
pub const LEFT_ARM: usize = 1;
pub const RIGHT_ARM: usize = 2;
pub const THIRD_ARM: usize = 3;
pub const LEGS: usize = 4;
pub const COUNT: usize = 5;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyParts {
    hard: [u8; COUNT],
    soft: [u8; COUNT],
}

impl BodyParts {
    /// 1408029e0. The component that acquires a request owns its matching
    /// release; a declaration's m_lockedBodyParts is a separate mode rule.
    pub fn hard_lock(&mut self, part: usize) {
        self.hard[part] = self.hard[part].wrapping_add(1);
    }
    /// 140802a00.
    pub fn hard_unlock(&mut self, part: usize) {
        self.hard[part] = self.hard[part].wrapping_sub(1);
    }
    /// 1408029f0. Idempotent state helpers retain their own held bit, as
    /// 140854050 does; independent components may request the same part.
    pub fn soft_lock(&mut self, part: usize) {
        self.soft[part] = self.soft[part].wrapping_add(1);
    }
    /// 140802a10, paired with the same actual request owner.
    pub fn soft_unlock(&mut self, part: usize) {
        self.soft[part] = self.soft[part].wrapping_sub(1);
    }
    pub fn hard_counts(&self) -> &[u8; COUNT] { &self.hard }
    pub fn soft_counts(&self) -> &[u8; COUNT] { &self.soft }

    /// 140854890: +68 overrides code-only occupancy, never hard occupancy.
    /// Self-admission (m_allowSelf, +69 bit0) does not affect this decision.
    pub fn can_activate(&self, mask: u8, unlocks_code_locked: bool) -> bool {
        (0..COUNT).all(|part| mask & (1 << part) == 0
            || self.hard[part] == 0 && (self.soft[part] == 0 || unlocks_code_locked))
    }
}
