//! One host output stream owns the sample clock and the shared bus histories.
//! ECS entities retain individual pause, stop and completion controls.
pub(super) mod buses;
use super::graph;
use buses::{Buses, FRAMES};
use bevy::{audio::{ChannelCount, Decodable, SampleRate, Source}, prelude::*};
use std::{sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU32, Ordering}}, time::Duration};
use crate::gameplay::sound::{activity::Activity, ducking::Ducking};

pub(super) type StateGroups = Arc<Mutex<crate::gameplay::sound::states::States>>;

#[derive(Resource, Default)]
pub(super) struct Engine {
    shared: Option<Arc<Mutex<Shared>>>,
    activity: Option<Arc<Activity>>,
    playing_ids: AtomicU32,
}

struct Control { paused: AtomicBool, stopped: AtomicBool, finished: AtomicBool }

/// Despawning a sound entity also cancels the voice held by the output stream.
#[derive(Component)]
pub(super) struct VoiceHandle(Arc<Control>);
impl VoiceHandle {
    pub fn pause(&self, paused: bool) { self.0.paused.store(paused, Ordering::Relaxed); }
    pub fn finished(&self) -> bool { self.0.finished.load(Ordering::Relaxed) }
}
impl Drop for VoiceHandle {
    fn drop(&mut self) { self.0.stopped.store(true, Ordering::Relaxed); }
}

struct Voice { audio: graph::Routed, control: Arc<Control>, event: u32 }
struct PendingState {
    tick: u64,
    serial: u64,
    action: crate::assets::wwise::StateAction,
    event: u32,
    emitter: crate::gameplay::sound::Emitter,
    world_paused: bool,
    paused_at: Option<u64>,
}
struct PendingPlay { tick: u64, serial: u64, control: Arc<Control>, paused_at: Option<u64> }
enum Due { State(PendingState), Play(PendingPlay) }
impl Due {
    fn order(&self) -> (u64, u64) {
        match self { Self::State(action) => (action.tick, action.serial), Self::Play(action) => (action.tick, action.serial) }
    }
}
struct Shared {
    buses: Buses,
    activity: Arc<Activity>,
    ducking: Ducking,
    tick: u64,
    voices: Vec<Voice>,
    states: StateGroups,
    switches: graph::SwitchRegistry,
    pending_states: Vec<PendingState>,
    pending_plays: Vec<PendingPlay>,
    action_serial: u64,
    failed: bool,
    error: Option<String>,
}

impl Engine {
    pub fn new(mixer: &crate::gameplay::sound::mix::Mixer, states: StateGroups) -> Result<(Self, OutputAudio), String> {
        let base = crate::assets::installation::game_base().map_err(|error| error.to_string())?;
        let activity = Arc::new(Activity::new(mixer)?);
        let ducking = Ducking::new(mixer)?;
        ducking.publish(&activity);
        let buses = Buses::new(mixer, &base, activity.clone())?;
        let shared = Arc::new(Mutex::new(Shared { buses, activity: activity.clone(), ducking, tick: 0,
            voices: Vec::new(), states, switches: Arc::new(Mutex::new(graph::SwitchControls::default())),
            pending_states: Vec::new(), pending_plays: Vec::new(), action_serial: 0, failed: false, error: None }));
        let output = OutputAudio { shared: shared.clone(), channels: ChannelCount::new(2).ok_or("invalid host stereo format")?,
            rate: SampleRate::new(48000).ok_or("invalid host processing rate")? };
        Ok((Self { shared: Some(shared), activity: Some(activity), playing_ids: AtomicU32::new(0) }, output))
    }

    pub fn activity(&self) -> Option<Arc<Activity>> { self.activity.clone() }
    /// Host event-instance keys keep every action from one Post in the same
    /// playing-ID scope. Zero remains the wildcard/invalid value.
    pub fn playing_id(&self) -> u32 {
        loop {
            let id = self.playing_ids.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
            if id != 0 { return id; }
        }
    }

    pub fn post_state(&self, action: crate::assets::wwise::StateAction, event: u32,
        emitter: crate::gameplay::sound::Emitter) -> Result<(), String> {
        let shared = self.shared.as_ref().ok_or("shared audio action clock is unavailable")?;
        let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        if shared.failed { return Err("shared audio action clock stopped after a processing error".into()); }
        let delay = action.delay.sample_frames(|| rand::random_range(0..=32767));
        // 1410e5600 splits the delay into a tick and sample offset. Set State
        // (14113c610) does not read that offset, so it changes at the tick.
        let quanta = u64::from(delay) / FRAMES as u64;
        if quanta == 0 {
            shared.set_state(action.group, action.value)?;
        } else {
            let tick = shared.tick.wrapping_add(quanta);
            let serial = shared.next_action();
            shared.pending_states.push(PendingState { tick, serial, action, event, emitter, world_paused: false, paused_at: None });
        }
        Ok(())
    }

    pub fn stop_event(&self, event: u32, emitter: crate::gameplay::sound::Emitter) {
        if let Some(shared) = &self.shared {
            let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
            shared.pending_states.retain(|pending| pending.event != event || pending.emitter != emitter);
            shared.voices.retain(|voice| {
                let stop = voice.event == event && voice.audio.emitter() == emitter;
                if stop { voice.control.finished.store(true, Ordering::Relaxed); }
                !stop
            });
        }
    }

    pub fn stop_actor(&self, target: Option<u32>, emitter: Option<crate::gameplay::sound::Emitter>, milliseconds: i32, curve: u8) {
        if let Some(shared) = &self.shared {
            let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
            shared.stop_actor(target, emitter, milliseconds, curve);
        }
    }

    /// Stop All also cancels pending non-Play actions (1410e9950). A global
    /// Set State has no game-object execution scope and survives an emitter's
    /// Stop All, while stopping its posted event still cancels that action.
    pub fn cancel_state_actions(&self, emitter: Option<crate::gameplay::sound::Emitter>) {
        if let Some(shared) = &self.shared {
            shared.lock().unwrap_or_else(|error| error.into_inner()).pending_states
                .retain(|pending| emitter.is_some_and(|emitter| !pending.action.game_object || pending.emitter != emitter));
        }
    }

    pub fn update_state_pause(&self, world_change: Option<bool>, frozen: impl Fn(crate::gameplay::sound::Emitter) -> bool) {
        if let Some(shared) = &self.shared {
            let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
            let tick = shared.tick;
            for pending in &mut shared.pending_states {
                // Pause snapshots existing playing IDs. Later UI posts keep
                // running; an owner's stopped clock remains an independent pause.
                if let Some(paused) = world_change { pending.world_paused = paused; }
                if pending.world_paused || frozen(pending.emitter) {
                    if pending.paused_at.is_none() { pending.paused_at = Some(tick); }
                } else if let Some(start) = pending.paused_at.take() {
                    // Resume shifts the due tick by the paused interval
                    // (1410ea0e0); it does not restart the authored delay.
                    pending.tick = pending.tick.wrapping_add(tick.wrapping_sub(start));
                }
            }
        }
    }

    pub fn submit(&self, mut audio: graph::ScheduledAudio, playing_id: u32, event: u32) -> Result<VoiceHandle, String> {
        let shared = self.shared.as_ref().ok_or("shared audio output is unavailable")?;
        let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        if shared.failed { return Err("shared audio output stopped after a processing error".into()); }
        let control = Arc::new(Control { paused: AtomicBool::new(false), stopped: AtomicBool::new(false), finished: AtomicBool::new(false) });
        audio.bind_switches(shared.switches.clone(), shared.states.clone(), playing_id);
        let audio = graph::Routed::new(audio)?;
        let quanta = audio.delay_quanta();
        if quanta != 0 {
            let tick = shared.tick.wrapping_add(quanta);
            let serial = shared.next_action();
            shared.pending_plays.push(PendingPlay { tick, serial, control: control.clone(), paused_at: None });
        }
        shared.voices.push(Voice { audio, control: control.clone(), event });
        Ok(VoiceHandle(control))
    }

    pub fn update(&self, mixer: &crate::gameplay::sound::mix::Mixer) -> Result<(), String> {
        let Some(shared) = &self.shared else { return Ok(()) };
        let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = shared.error.take() { return Err(error); }
        shared.buses.update(mixer)
    }
}

impl Shared {
    fn next_action(&mut self) -> u64 { let serial = self.action_serial; self.action_serial = serial.wrapping_add(1); serial }
    fn synchronize_controls(&mut self) {
        self.voices.retain(|voice| {
            let stopped = voice.control.stopped.load(Ordering::Relaxed) || voice.control.finished.load(Ordering::Relaxed);
            if stopped { voice.control.finished.store(true, Ordering::Relaxed); }
            !stopped
        });
        for voice in &mut self.voices {
            voice.audio.pause_controllers(voice.control.paused.load(Ordering::Relaxed));
            voice.audio.flush_stops();
        }
    }

    fn stop_actor(&mut self, target: Option<u32>, emitter: Option<crate::gameplay::sound::Emitter>, milliseconds: i32, curve: u8) {
        for voice in &mut self.voices {
            if emitter.is_some_and(|emitter| emitter != voice.audio.emitter()) { continue; }
            // 1411068e0 stops a paused PBI immediately instead of waiting for
            // a fade on its stopped playback clock.
            let milliseconds = if voice.control.paused.load(Ordering::Relaxed) { 0 } else { milliseconds };
            if let Some(target) = target { voice.audio.stop_actor(target, milliseconds, curve); }
            else { voice.audio.stop_all(milliseconds, curve); }
            if voice.audio.cancelled() { voice.control.finished.store(true, Ordering::Relaxed); }
        }
    }

    fn switch_commands(&mut self, commands: Vec<crate::gameplay::sound::switches::Command<graph::SwitchContext>>) -> Result<(), String> {
        for command in commands {
                    use crate::gameplay::sound::switches::Command;
                    match &command {
                        Command::CancelPending { emitter, child } => {
                            for voice in &mut self.voices {
                                if voice.audio.emitter() == *emitter { voice.audio.cancel_pending(*child); }
                            }
                        },
                        Command::Stop { emitter, child, milliseconds, curve } => {
                            self.stop_actor(Some(*child), Some(*emitter), *milliseconds, *curve);
                        },
                        Command::Release { .. } => return Err("switch decoder-loop release is not hosted".into()),
                        Command::Play { controller, child, milliseconds, curve } => {
                            if let Some(voice) = self.voices.iter_mut().find(|voice| voice.audio.owns(&controller.context)) {
                                voice.audio.play_switch(&controller.context, *child, *milliseconds, *curve);
                            }
                        },
                        Command::ResetHistory { .. } | Command::StoreHistory { .. } => {
                            self.switches.lock().unwrap_or_else(|error| error.into_inner()).controllers.apply_history(&command);
                        },
                    }
        }
        Ok(())
    }

    fn set_state(&mut self, group: u32, value: u32) -> Result<(), String> {
        if self.states.lock().unwrap_or_else(|error| error.into_inner()).transition_ms(group, value).is_none() { return Ok(()); }
        self.synchronize_controls();
        let definitions = self.switches.lock().unwrap_or_else(|error| error.into_inner()).definitions.iter()
            .filter(|(_, settings)| settings.continuous && settings.group_kind == 1 && settings.group == group)
            .map(|(&node, settings)| (node, settings.clone())).collect::<Vec<_>>();
        for (node, settings) in definitions {
            let recipients = self.switches.lock().unwrap_or_else(|error| error.into_inner()).controllers.recipients(node);
            for emitter in recipients {
                let change = self.switches.lock().unwrap_or_else(|error| error.into_inner()).controllers.change(node, &settings, emitter, value);
                let Some(change) = change else { continue; };
                self.switch_commands(change.commands)?;
                let starts = self.switches.lock().unwrap_or_else(|error| error.into_inner()).controllers
                    .starts(node, &settings, emitter, value, change.previous);
                self.switch_commands(starts)?;
            }
        }
        // 1410d87b0 stores the current ID after notifying subscribers. Child
        // switches created by callbacks therefore still observe the old ID.
        self.states.lock().unwrap_or_else(|error| error.into_inner()).set(group, value);
        Ok(())
    }

    fn block(&mut self) -> Result<[f32; FRAMES * 2], String> {
        let tick = self.tick;
        // Due actions execute before voice preparation/rendering (1410e6800).
        let mut due = Vec::new();
        for pending in std::mem::take(&mut self.pending_states) {
            if pending.paused_at.is_some() || pending.tick > tick { self.pending_states.push(pending); }
            else { due.push(Due::State(pending)); }
        }
        for mut pending in std::mem::take(&mut self.pending_plays) {
            if pending.control.stopped.load(Ordering::Relaxed) || pending.control.finished.load(Ordering::Relaxed) { continue; }
            if pending.control.paused.load(Ordering::Relaxed) {
                if pending.paused_at.is_none() { pending.paused_at = Some(tick); }
            } else if let Some(start) = pending.paused_at.take() { pending.tick = pending.tick.wrapping_add(tick.wrapping_sub(start)); }
            if pending.paused_at.is_some() || pending.tick > tick { self.pending_plays.push(pending); }
            else { due.push(Due::Play(pending)); }
        }
        due.sort_by_key(Due::order);
        for action in due {
            match action {
                Due::State(pending) => self.set_state(pending.action.group, pending.action.value)?,
                Due::Play(pending) => {
                    if pending.control.stopped.load(Ordering::Relaxed) || pending.control.finished.load(Ordering::Relaxed) { continue; }
                    if let Some(voice) = self.voices.iter_mut().find(|voice| Arc::ptr_eq(&voice.control, &pending.control)) {
                        voice.audio.release_delayed();
                    }
                },
            }
        }
        self.synchronize_controls();
        self.buses.begin();
        for voice in &mut self.voices {
            if voice.control.stopped.load(Ordering::Relaxed) { voice.control.finished.store(true, Ordering::Relaxed); }
            else { voice.audio.begin(voice.control.paused.load(Ordering::Relaxed)); }
        }
        self.voices.retain(|voice| !voice.control.finished.load(Ordering::Relaxed));
        self.ducking.synchronize(&self.activity, self.tick);
        self.ducking.advance(self.tick);
        self.ducking.publish(&self.activity);
        for frame in 0..FRAMES {
            for voice in &mut self.voices {
                if voice.control.finished.load(Ordering::Relaxed) || voice.control.paused.load(Ordering::Relaxed) { continue; }
                if !voice.audio.frame(&mut self.buses, frame)? { voice.control.finished.store(true, Ordering::Relaxed); }
            }
        }
        self.voices.retain(|voice| !voice.control.finished.load(Ordering::Relaxed));
        self.ducking.synchronize(&self.activity, self.tick);
        self.ducking.publish(&self.activity);
        let output = *self.buses.finish()?;
        self.tick = self.tick.wrapping_add(1);
        Ok(output)
    }
}

#[derive(Asset, TypePath, Clone)]
pub(super) struct OutputAudio {
    shared: Arc<Mutex<Shared>>,
    channels: ChannelCount,
    rate: SampleRate,
}

pub(super) struct Decoder {
    shared: Arc<Mutex<Shared>>,
    channels: ChannelCount,
    rate: SampleRate,
    block: [f32; FRAMES * 2],
    position: usize,
}
impl Iterator for Decoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.position == self.block.len() {
            let mut shared = self.shared.lock().unwrap_or_else(|error| error.into_inner());
            if shared.failed { self.block.fill(0.0); }
            else {
                match shared.block() {
                    Ok(block) => self.block = block,
                    Err(error) => {
                        for voice in &shared.voices { voice.control.finished.store(true, Ordering::Relaxed); }
                        shared.voices.clear();
                        shared.pending_states.clear();
                        shared.pending_plays.clear();
                        shared.error = Some(error);
                        shared.failed = true;
                        self.block.fill(0.0);
                    },
                }
            }
            self.position = 0;
        }
        let value = self.block[self.position];
        self.position += 1;
        Some(value)
    }
}
impl Source for Decoder {
    fn current_span_len(&self) -> Option<usize> { None }
    fn channels(&self) -> ChannelCount { self.channels }
    fn sample_rate(&self) -> SampleRate { self.rate }
    fn total_duration(&self) -> Option<Duration> { None }
}
impl Decodable for OutputAudio {
    type Decoder = Decoder;
    fn decoder(&self) -> Decoder {
        Decoder { shared: self.shared.clone(), channels: self.channels, rate: self.rate,
            block: [0.0; FRAMES * 2], position: FRAMES * 2 }
    }
}

pub(super) fn retire(mut commands: Commands, voices: Query<(Entity, &VoiceHandle)>) {
    for (entity, voice) in &voices { if voice.finished() { commands.entity(entity).try_despawn(); } }
}
