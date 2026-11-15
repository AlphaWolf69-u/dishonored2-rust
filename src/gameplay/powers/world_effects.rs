//! Bend Time and Dark Vision component protocols, traced from 14061c670,
//! 14061cd60, 14061d1c0 and 14043d3c0..14043d560. Effect lifetime belongs
//! to the real world clock/timers and dark-vision component, not the arms.
use crate::{assets::entities::Value, gameplay::options::decl::atom};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeLease(pub i32);
#[derive(Clone, Copy, Debug)]
pub struct TimeRequest {
    pub world: f32,
    pub unbent: f32,
    pub ease: f32,
}
#[derive(Clone, Copy)]
struct Request { lease: TimeLease, world: f32, unbent: f32 }

/// 14080eb40/14080ede0: the smallest retained scale wins independently
/// for the world and unbent clocks. Removing one user's request must not
/// restore clocks still held by Blink, another power or a scripted effect.
#[derive(Default)]
pub struct TimeRequests { next: i32, requests: Vec<Request> }
impl TimeRequests {
    pub fn scales(&self) -> (f32, f32) {
        let Some(first) = self.requests.first() else { return (1.0, 1.0) };
        self.requests.iter().skip(1).fold((first.world, first.unbent), |(world, unbent), request|
            (world.min(request.world), unbent.min(request.unbent)))
    }
    /// Return the owner's handle and the effective clock targets. The
    /// platform clock owner applies easing (140059890, channels 3 and 4).
    pub fn acquire(&mut self, request: TimeRequest) -> (TimeLease, TimeRequest) {
        let lease = TimeLease(self.next);
        self.next = self.next.wrapping_add(1);
        self.requests.push(Request { lease, world: request.world, unbent: request.unbent });
        let (world, unbent) = self.scales();
        (lease, TimeRequest { world, unbent, ease: request.ease })
    }
    pub fn release(&mut self, lease: TimeLease, ease: f32) -> Option<TimeRequest> {
        let index = self.requests.iter().position(|request| request.lease == lease)?;
        let before = self.scales();
        self.requests.remove(index);
        let (world, unbent) = self.scales();
        (before != (world, unbent)).then_some(TimeRequest { world, unbent, ease })
    }
}

pub struct BendTimeSettings {
    pub unbent_time_scale: f32,
    pub ease_in: f32,
    pub ease_out: f32,
    pub lower_stop_min: f32,
    pub lower_stop_max: f32,
    pub start_sound: Option<String>,
    pub stop_sound: Option<String>,
}
impl BendTimeSettings {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let number = |name, default| match atom(fields, name) {
            None => Ok(default), Some(value) => value.trim_end_matches('f').parse::<f32>().ok()
                .filter(|value| value.is_finite()).ok_or_else(|| format!("invalid Bend Time {name}: {value}")),
        };
        let path = |name| atom(fields, name).filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("NULL")).map(str::to_owned);
        // Defaults are constructor 14061c410, not UI descriptions.
        Ok(Self { unbent_time_scale: number("m_unbentTimeScale", 100.0)?,
            ease_in: number("m_easeInDuration", 0.5)?, ease_out: number("m_easeOutDuration", 0.5)?,
            lower_stop_min: number("m_lowerStopTimeScaleMin", 0.0)?,
            lower_stop_max: number("m_lowerStopTimeScaleMax", 90.0)?,
            start_sound: path("m_startLoopSoundEvent"), stop_sound: path("m_stopLoopSoundEvent") })
    }
}
/// Values from the player's attribute owner; upgrade items modify these
/// same attributes. Missing time-factor/duration indices read zero; a
/// missing/nonpositive relativity index uses the declaration's percentage.
#[derive(Clone, Copy)]
pub struct BendTimeAttributes { pub time_factor: f32, pub duration: f32, pub relativity: f32 }
impl BendTimeAttributes {
    /// Tags are the executable's arkAttributeUnique names corresponding to
    /// the indices retained at +c4/+c8/+cc by 14061c980.
    pub fn read(attributes: &crate::gameplay::player::states::Attributes) -> Self {
        Self { time_factor: attributes.get("ARK_ATTRIBUTE_BENDTIME_TIMEFACTOR"),
            duration: attributes.get("ARK_ATTRIBUTE_BENDTIME_DURATION"),
            relativity: attributes.get("ARK_ATTRIBUTE_BENDTIME_RELATIVITY") }
    }
}

/// Real engine owner operations required by the power. Hosts must reject
/// admission until all are present, including the timer/message receiver.
pub trait BendTimeWorld {
    fn available(&self) -> bool;
    fn acquire_time(&mut self, request: TimeRequest) -> TimeLease;
    fn release_time(&mut self, lease: TimeLease, ease: f32);
    /// cpntTimersManager slot 0, mode 2 (140778f40); expiry calls timer_expired.
    fn start_timer(&mut self, duration: f32);
    fn cancel_timer(&mut self);
    /// 14080d8c0 applies the native per-entity override. On activation the
    /// player receives the original sentinel; stopped NPCs ease out on exit.
    fn set_player_time_override(&mut self, value: f32);
    fn ease_out_stopped_npcs(&mut self, duration: f32);
    fn message(&mut self, id: u32);
    /// Event 0x73 with kind plus the actual player's position payload.
    fn usage_event(&mut self, kind: u32);
    fn sound(&mut self, event: &str);
    fn power_state(&mut self, active: bool);
}
pub struct BendTime {
    pub settings: BendTimeSettings,
    lease: Option<TimeLease>,
    stopped: bool,
    lower_stop: bool,
    must_cancel: bool,
}
impl BendTime {
    pub fn new(settings: BendTimeSettings) -> Self {
        Self { settings, lease: None, stopped: false, lower_stop: false, must_cancel: false }
    }
    pub fn active(&self) -> bool { self.lease.is_some() }
    /// 14061c480 clears cancellation on entry to a new use-power action.
    pub fn begin_cast(&mut self) { self.must_cancel = false; }
    pub fn stopped(&self) -> bool { self.stopped }
    pub fn must_cancel(&self) -> bool { self.must_cancel }
    pub fn mana_cost(&self, normal: f32) -> f32 { if self.active() { 0.0 } else { normal } }
    pub fn cooldown(&self, normal: f32) -> f32 { if self.active() { 0.0 } else { normal } }
    fn unbent(&self, attributes: BendTimeAttributes) -> f32 {
        if attributes.relativity > 0.0 { 1.0 } else { self.settings.unbent_time_scale * 0.01 }
    }
    fn replace(&mut self, world: &mut impl BendTimeWorld, scale: f32, attributes: BendTimeAttributes) {
        if let Some(lease) = self.lease.take() { world.release_time(lease, 0.0); }
        self.lease = Some(world.acquire_time(TimeRequest { world: scale,
            unbent: self.unbent(attributes), ease: self.settings.ease_in }));
    }
    /// 14061cd60, activation half. Uses an actual clock request and timer;
    /// no local duration counter can stand in for the timer manager's clock.
    fn activate(&mut self, world: &mut impl BendTimeWorld, attributes: BendTimeAttributes) {
        let factor = attributes.time_factor.max(0.0);
        self.stopped = factor == 0.0;
        self.replace(world, factor * 0.01, attributes);
        world.message(0xe2); world.message(0xe4);
        world.start_timer(attributes.duration.max(0.0));
        if let Some(sound) = &self.settings.start_sound { world.sound(sound); }
        world.usage_event(6);
        world.set_player_time_override(f32::from_bits(0x7149_f2ca));
        self.lower_stop = false;
        world.power_state(true);
    }
    /// 14061cd60, deactivation half. The live request and timer are both
    /// withdrawn even if the equipped arm scene has already been cancelled.
    pub fn stop(&mut self, world: &mut impl BendTimeWorld) -> bool {
        let Some(lease) = self.lease.take() else { return false };
        if self.stopped { world.ease_out_stopped_npcs(self.settings.ease_out); }
        world.cancel_timer();
        world.release_time(lease, self.settings.ease_out);
        self.stopped = false; self.lower_stop = false;
        if let Some(sound) = &self.settings.stop_sound { world.sound(sound); }
        world.message(0xe3); world.message(0xe5);
        world.usage_event(5);
        world.power_state(false);
        true
    }
    /// 14061c670. The held-button path temporarily lowers stopped time;
    /// otherwise a cast toggles the real effect. Payment is the caller's
    /// shared arkCpntPower callback, invoked in native order before toggle.
    pub fn cast(&mut self, world: &mut impl BendTimeWorld, attributes: BendTimeAttributes,
        button_held: bool, mut pay: impl FnMut()) -> bool {
        if !world.available() { return false; }
        if !self.active() || !self.stopped || !button_held {
            if !self.lower_stop {
                pay();
                if self.active() { self.stop(world); } else { self.activate(world, attributes); }
                return true;
            }
            self.lower_stop = false;
        } else {
            self.lower_stop = !self.lower_stop;
        }
        if !self.lower_stop { self.replace(world, 0.0, attributes); }
        if !button_held { pay(); }
        true
    }
    /// 14061d1c0 maps analog hold pressure through the actual controller
    /// deadzone and declaration percentages while the lowering phase is live.
    pub fn update_hold(&mut self, world: &mut impl BendTimeWorld, attributes: BendTimeAttributes,
        button_held: bool, pressure: f32, deadzone: f32, cast_active: bool, cast_committed: bool,
        pay: impl FnMut()) {
        if !cast_active || cast_committed || !self.lower_stop { return; }
        if !button_held { self.cast(world, attributes, false, pay); return; }
        let min = (self.settings.lower_stop_min * 0.01).clamp(0.0, 1.0);
        let max = (self.settings.lower_stop_max * 0.01).clamp(0.0, 1.0);
        // Preserve the native ordered comparisons (rather than clamp,
        // which panics when authored endpoints are reversed).
        let pressure = (pressure - deadzone) / (1.0 - deadzone);
        let pressure = if min <= pressure { if max < pressure { max } else { pressure } } else { min };
        self.replace(world, pressure, attributes);
    }
    /// The real timer's d1 message, 14061d350. The native debug disable is
    /// supplied explicitly; effect clocks may not be guessed from frame dt.
    pub fn timer_expired(&mut self, world: &mut impl BendTimeWorld, cast_active: bool, disabled: bool) {
        if self.active() && !disabled { self.stop(world); self.must_cancel |= cast_active; }
    }
    /// 14061c5f0: ending the arm cast restores full stop if a lowering
    /// preview was active; it does not end the already purchased effect.
    pub fn finish_cast(&mut self, world: &mut impl BendTimeWorld, attributes: BendTimeAttributes) {
        if self.lower_stop { self.replace(world, 0.0, attributes); }
        self.lower_stop = false;
    }
    /// The player's d6 modifier notification, 14061cbd0: while stopped,
    /// relativity changes immediately replace the shared clock lease.
    pub fn attributes_changed(&mut self, world: &mut impl BendTimeWorld, attributes: BendTimeAttributes) {
        if !self.active() { return; }
        if let Some(sound) = &self.settings.start_sound { world.sound(sound); }
        if self.stopped { self.replace(world, 0.0, attributes); }
    }
}

/// This is the effect component's actual lifecycle, not a power-local
/// on/off flag. In particular a pending Start/Stop must keep its ordering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DarkVisionState { Stopped, PendingStart, Started, PendingStop }
pub trait DarkVisionWorld {
    fn component_state(&self) -> Option<DarkVisionState>;
    /// 14043d530: component declaration gate plus mark suppression roster
    /// or its outstanding suppression count; the mark owner computes this.
    fn blocked(&self) -> bool;
    fn request_start(&mut self);
    fn request_stop(&mut self);
}
pub struct DarkVision;
impl DarkVision {
    /// 14043d490. Deactivating or a blocked start costs zero.
    pub fn mana_cost(world: &impl DarkVisionWorld, normal: f32) -> f32 {
        match world.component_state() {
            Some(DarkVisionState::Stopped | DarkVisionState::PendingStart) if !world.blocked() => normal,
            _ => 0.0,
        }
    }
    /// 14043d3c0. No component is a failed cast (sets the caller's cancel
    /// flag); payment otherwise precedes the start/stop lifecycle request.
    pub fn cast(world: &mut impl DarkVisionWorld, mut pay: impl FnMut()) -> bool {
        let Some(state) = world.component_state() else { return false };
        pay();
        match state {
            DarkVisionState::Stopped | DarkVisionState::PendingStart if !world.blocked() => world.request_start(),
            DarkVisionState::Started | DarkVisionState::PendingStop => world.request_stop(),
            _ => {},
        }
        true
    }
    /// 14043d300: cancelling the arm action requests stop only if the
    /// component actually entered the started/pending-stop lifecycle.
    pub fn cancel(world: &mut impl DarkVisionWorld) {
        if matches!(world.component_state(), Some(DarkVisionState::Started | DarkVisionState::PendingStop)) {
            world.request_stop();
        }
    }
}
