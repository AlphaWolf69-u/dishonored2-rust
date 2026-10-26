//! Continuation-action timing (14112ed80 / 14112f640 / 14112f9a0).
use crate::assets::wwise::TransitionMode;

pub const QUANTUM: u32 = 1024; // 142427c30

fn truncate_i32(value: f64) -> i32 {
    // CVTTSD2SI/CVTTSS2SI return the indefinite integer on overflow or NaN.
    let value = value.trunc();
    if value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX) { value as i32 }
    else { i32::MIN }
}

/// 14112ed80 rounds the signed double result away from zero at a half sample.
/// Delayed-action storage interprets that 32-bit result as an unsigned count.
pub fn samples_from_ms(milliseconds: f64, rate: u32) -> u32 {
    let samples = f64::from(rate) * milliseconds * 0.001;
    truncate_i32(samples + if samples > 0.0 { 0.5 } else { -0.5 }) as u32
}

#[derive(Clone, Copy, Debug)]
pub struct Crossfade {
    pub delay_samples: u32,
    pub milliseconds: i32,
    pub curve: u8,
}

/// 14112f640 only schedules an early crossfade for sources at least 50 ms
/// long. Its overlap is bounded by half the pitch-adjusted source duration.
pub fn crossfade(mode: TransitionMode, source_ms: f32, transition_ms: f32, rate: u32) -> Option<Crossfade> {
    let curve = match mode {
        TransitionMode::CrossfadeAmplitude => 4,
        TransitionMode::CrossfadePower => 1,
        _ => return None,
    };
    if source_ms < 50.0 { return None; }
    let half = source_ms * 0.5;
    let overlap = if half <= transition_ms { half } else { transition_ms };
    Some(Crossfade { delay_samples: samples_from_ms(f64::from(source_ms - overlap), rate),
        milliseconds: truncate_i32(f64::from(overlap)), curve })
}

/// Source-end notifications are processed after rendering the quantum and
/// before incrementing the action clock (1410e6800). A sub-quantum delay is
/// executed immediately with a PBI sample offset; that PBI first renders in
/// the next quantum. Longer actions run when their quotient tick is reached
/// (1410e5600). Sample-accurate continuations use the source handoff instead.
pub fn after_source_end(frame: u64, mode: TransitionMode, delay_samples: u32) -> u64 {
    if mode == TransitionMode::SampleAccurate { return frame + 1; }
    let quantum = u64::from(QUANTUM);
    let tick = frame / quantum * quantum;
    tick + u64::from(delay_samples) + if delay_samples < QUANTUM { quantum } else { 0 }
}

/// No PBI was created: 1410ddd30 selects from the enclosing continuation
/// immediately, then queues that actor through 1410dae80 / 1410e5600. Only
/// Delay and TriggerRate use their authored interval; all modes wait at least
/// ten quanta. This path does not carry the failed PBI's sample offset.
pub fn after_failed_start(frame: u64, mode: TransitionMode, milliseconds: f32, rate: u32) -> u64 {
    let minimum = QUANTUM * 10;
    let delay = if matches!(mode, TransitionMode::Delay | TransitionMode::TriggerRate) {
        (samples_from_ms(f64::from(milliseconds), rate) as i32).max(minimum as i32) as u32
    } else { minimum };
    frame / u64::from(QUANTUM) * u64::from(QUANTUM) + u64::from(delay)
}
