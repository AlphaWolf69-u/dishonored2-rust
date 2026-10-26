//! FX sound event scheduling (230160/2301f0/2302a0/230310).
//! The output owner supplies a sound object and tracks its playing/queued IDs;
//! this state owns the authored delayed start/stop event and local position.
use bevy_math::{Mat3, Vec3};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoundEvent {
    /// arkFXSoundDesc::soundEvent_t: delay endpoints followed by the decl.
    pub delay: [f32; 2],
    pub event: Option<u32>,
}

pub struct SoundEffect {
    pub local_position: Vec3,
    pending: Option<u32>,
    post_time: f32,
    keep_alive: bool,
    stop: SoundEvent,
}

pub struct SoundUpdate {
    /// Post on the existing object, or spawn one when none is allocated.
    pub event: Option<u32>,
    /// Every update moves the object's position, even without a new event.
    pub position: Vec3,
}

/// 22ffb0 / 1da100: three independent samples, including fixed ranges.
pub fn sample_position(range: [Vec3; 2], seed: &mut u32) -> Vec3 {
    Vec3::from_array(std::array::from_fn(|axis| super::timing::sample_range([range[0][axis], range[1][axis]], seed)))
}

impl SoundEffect {
    /// Initialization consumes a delay sample even when the start event is
    /// absent. A stop event retains the slot after the start sound finishes.
    pub fn new(local_position: Vec3, start: SoundEvent, stop: SoundEvent, start_age: f32, seed: &mut u32) -> Self {
        Self {
            local_position,
            pending: start.event,
            post_time: super::add(start_age, super::timing::sample_range(start.delay, seed)),
            keep_alive: stop.event.is_some(),
            stop,
        }
    }

    /// 230120: the host passes whether the handle has any playing or queued
    /// IDs. Allocation of an empty sound object alone does not retain a slot.
    pub fn is_alive(&self, playing_or_queued: bool) -> bool {
        self.pending.is_some() || self.keep_alive || playing_or_queued
    }

    /// 2302a0: a valid stop replaces any unposted start and clears queued
    /// handle events before scheduling the stop. It does not stop voices
    /// directly: the authored stop event supplies the Wwise actions.
    pub fn request_stop(&mut self, age: f32, seed: &mut u32, mut clear_queued: impl FnMut()) {
        if self.stop.event.is_some() {
            clear_queued();
            self.pending = self.stop.event;
            self.post_time = super::add(age, super::timing::sample_range(self.stop.delay, seed));
            self.keep_alive = false;
        }
    }

    /// 230310: an event is attempted once when its delay expires, regardless
    /// of whether the output object successfully posts it.
    pub fn update(&mut self, age: f32, origin: Vec3, axis: Mat3) -> SoundUpdate {
        let local = self.local_position;
        let position = axis.y_axis * local.y + axis.x_axis * local.x + axis.z_axis * local.z + origin;
        let event = if self.post_time <= age { self.pending.take() } else { None };
        SoundUpdate { event, position }
    }
}
