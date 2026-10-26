//! Authored collision settings, constructors and version conversion.
//! Reflection 15507f0 and routines 088f70/088f90/0890c0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CollisionSettings {
    pub flags: u16,
    pub version: i32,
    pub blocker: u32,
    pub legacy_flags: u8,
}
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum CollisionBlock {
    Npc = 1,
    Player = 2,
    Weapon = 4,
    Vision = 8,
    Dynamic = 16,
    BlockedByStatic = 32,
    DebrisAndParticles = 64,
    Ragdoll = 128,
}
impl Default for CollisionSettings {
    fn default() -> Self {
        Self {
            flags: 0x7f,
            version: 0,
            blocker: 4,
            legacy_flags: 0x7f,
        }
    }
}
impl CollisionSettings {
    pub const CURRENT_VERSION: i32 = 0x0133_7834;
    /// NPC, player, weapon, vision, dynamic, static, debris, ragdoll order.
    pub fn from_blocking(blocking: [bool; 8]) -> Self {
        let flags = blocking
            .into_iter()
            .enumerate()
            .fold(0u8, |flags, (bit, value)| flags | (u8::from(value) << bit));
        Self {
            flags: u16::from(flags),
            version: Self::CURRENT_VERSION,
            blocker: 4,
            legacy_flags: flags,
        }
    }
    pub fn legacy_blocking(&self, block: CollisionBlock) -> bool {
        self.legacy_flags & block as u8 != 0
    }
    /// Reflected legacy Boolean setters edit the byte at +0c, not flags at +0.
    pub fn set_legacy_blocking(&mut self, block: CollisionBlock, enabled: bool) {
        self.legacy_flags =
            (self.legacy_flags & !(block as u8)) | if enabled { block as u8 } else { 0 };
    }
    pub fn upgrade(&mut self) {
        if self.version < Self::CURRENT_VERSION {
            if self.flags == 0x7f {
                let legacy = match self.blocker {
                    1 => 0,
                    2 | 4 => self.legacy_flags,
                    _ => 0x7f,
                };
                *self = Self::from_blocking(std::array::from_fn(|bit| legacy & (1 << bit) != 0));
            } else {
                self.version = Self::CURRENT_VERSION;
            }
        }
    }
}
#[cfg(test)]
#[path = "collision_settings/tests.rs"]
mod tests;
