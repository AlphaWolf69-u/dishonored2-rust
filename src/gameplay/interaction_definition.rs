//! The source `arkInteractionDef` value and its process-lifetime UID owner.
use std::sync::atomic::{AtomicU32, Ordering};

// 77c2d0 assigns DAT_14398e390 and then increments the same 32-bit counter.
// Construction can run on preparation workers, so the one host counter uses
// relaxed atomic ordering; copying a value does not allocate another UID.
static NEXT_UID: AtomicU32 = AtomicU32::new(0);
static NEXT_REQUEST_ID: AtomicU32 = AtomicU32::new(0);

/// Process-wide DAT_143994a30, shared by interaction requests and state
/// request records. Both 3b7aa0 and 681f00 advance this same counter.
pub(crate) fn next_state_request_id() -> u32 {
    let previous = NEXT_REQUEST_ID.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
        let next = value.wrapping_add(1) & 0x7fff_ffff;
        Some(if next == 0 { 1 } else { next })
    }).expect("state request counter update cannot fail");
    let next = previous.wrapping_add(1) & 0x7fff_ffff;
    if next == 0 { 1 } else { next }
}

pub(crate) const ALLOW_SWIMMING: u8 = 1 << 0;
pub(crate) const ALLOW_SHADOW_WALKING: u8 = 1 << 1;
pub(crate) const ALLOW_POSSESSION: u8 = 1 << 2;
pub(crate) const SEND_USE_NOTIFICATION: u8 = 1 << 3;
pub(crate) const FRONT_ONLY: u8 = 1 << 4;
pub(crate) const LOCKED: u8 = 1 << 5;
const SOURCE_FLAGS: u8 = 0x3f;

/// The defined fields copied by 681fb0. The high two bits of the native flag
/// byte are unspecified padding and are not exposed as a separate state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct InteractionDef {
    pub name: u32,
    pub usage_text: u32,
    pub maximum_distance: f32,
    pub maximum_height: f32,
    pub occlusion_shape: i32,
    pub player_mode: i32,
    pub player_state: i32,
    flags: u8,
    pub icon: i32,
    pub uid: u32,
}
impl InteractionDef {
    /// Native constructor 77c2d0; UID wrap is the source's 32-bit wrap.
    pub(crate) fn new() -> Self {
        Self { name: 0, usage_text: 0, maximum_distance: 3.0, maximum_height: 1000.0,
            occlusion_shape: 0, player_mode: 14, player_state: 91,
            flags: ALLOW_SWIMMING | ALLOW_POSSESSION | SEND_USE_NOTIFICATION,
            icon: -1, uid: NEXT_UID.fetch_add(1, Ordering::Relaxed) }
    }
    pub(crate) fn flags(&self) -> u8 { self.flags }
    pub(crate) fn set_flags(&mut self, flags: u8) { self.flags = flags & SOURCE_FLAGS; }
    pub(crate) fn has(&self, flag: u8) -> bool { self.flags & flag != 0 }
}

/// One of the two inline request records constructed by 681f00. Its
/// InteractionDef is the actual 77c2d0 value, including its own UID.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StateRequestRecord {
    pub serial: u32,
    pub state: i32,
    pub result: i32,
    pub words: [u32; 2],
    pub interaction: InteractionDef,
}

/// The fixed pair belongs to its state instance. Allocate each request ID
/// and its interaction UID in native constructor order, not at preparation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct StateRequestPair {
    pub records: [StateRequestRecord; 2],
}
impl StateRequestPair {
    pub(crate) fn new() -> Self {
        Self { records: std::array::from_fn(|_| StateRequestRecord {
            serial: next_state_request_id(), state: 0x5b, result: -1,
            words: [0; 2], interaction: InteractionDef::new(),
        }) }
    }
}
