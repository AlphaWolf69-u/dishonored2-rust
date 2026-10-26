//! Linear-amplitude voice transitions (1411068e0 / 141130c50 / 141130ab0).
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy)]
pub(super) struct Envelope {
    start: u64,
    duration: u64,
    from: f32,
    to: f32,
    curve: u32,
}
impl Envelope {
    pub fn new(start: u64, duration: u64, from: f32, curve: u8) -> Self {
        // Voice transitions set bUseReciprocalCurve. Downward transitions
        // reverse complementary curves, except the two symmetric S curves.
        let curve = if curve == 3 || curve == 5 { curve } else { 8 - curve };
        Self { start, duration, from, to: 0.0, curve: u32::from(curve) }
    }
    /// 1411064a0 uses the same reciprocal transition descriptor, with
    /// increasing endpoints 0 -> 1; the authored curve is not reversed.
    pub fn fade_in(start: u64, duration: u64, curve: u8) -> Self {
        Self { start, duration, from: 0.0, to: 1.0, curve: u32::from(curve) }
    }
    pub fn value(self, frame: u64) -> Option<f32> {
        let age = frame.saturating_sub(self.start);
        if age >= self.duration { return if self.to == 0.0 { None } else { Some(self.to) }; }
        Some(crate::gameplay::sound::mix::interpolate(age as f32 / self.duration as f32, self.from, self.to, self.curve))
    }
}

#[derive(Clone, Default)]
pub(super) struct Control(Arc<Mutex<Option<(i32, u8)>>>);
impl Control {
    pub fn stop(&self, milliseconds: i32, curve: u8) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some((milliseconds, curve));
    }
    pub fn take(&self) -> Option<(i32, u8)> {
        self.0.lock().unwrap_or_else(|error| error.into_inner()).take()
    }
}

pub(super) fn frames(milliseconds: i32, rate: u32) -> u64 {
    if milliseconds == 0 { return 0; }
    // 142427c20/c24/c30: 21-ms transition ticks, 48 kHz, 1024 output
    // samples per quantum. 141130c50 rounds duration up to whole ticks.
    let quanta = u64::from((milliseconds.wrapping_add(20) / 21) as u32);
    (quanta * 1024 * u64::from(rate) + 47_999) / 48_000
}
