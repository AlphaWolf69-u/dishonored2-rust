//! One host output stream owns the sample clock and the shared bus histories.
//! ECS entities retain individual pause, stop and completion controls.
pub(super) mod buses;
use super::graph;
use buses::{Buses, FRAMES};
use bevy::{audio::{ChannelCount, Decodable, SampleRate, Source}, prelude::*};
use std::{sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Duration};
use crate::gameplay::sound::{activity::Activity, ducking::Ducking};

pub(super) type ActionTarget = Option<Arc<[u32]>>;

pub(super) type StateGroups = Arc<Mutex<crate::gameplay::sound::states::States>>;

#[derive(Resource, Default)]
pub(super) struct Engine {
    shared: Option<Arc<Mutex<Shared>>>,
    activity: Option<Arc<Activity>>,
    playing_ids: crate::gameplay::sound::playing::PlayingIds,
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
struct FallbackVoice { playing: graph::Playing, control: Arc<Control>, event: u32, emitter: crate::gameplay::sound::Emitter }

/// Retains action control for the existing multichannel host decoder. Its
/// channel conversion and bus rendering still require the shared-output port.
#[derive(Component)]
pub(super) struct FallbackHandle(Arc<Control>);
impl FallbackHandle {
    pub fn pause(&self, paused: bool) { self.0.paused.store(paused, Ordering::Relaxed); }
}
impl Drop for FallbackHandle {
    fn drop(&mut self) { self.0.stopped.store(true, Ordering::Relaxed); }
}

enum Action { State(crate::assets::wwise::StateAction), Stop(crate::assets::wwise::StopAction) }
impl Action {
    fn game_object(&self) -> bool { match self { Self::State(action) => action.game_object, Self::Stop(action) => action.game_object } }
    fn delay(&self) -> crate::assets::wwise::ActionDelay { match self { Self::State(action) => action.delay, Self::Stop(action) => action.delay } }
}
struct PendingAction {
    tick: u64,
    serial: u64,
    action: Action,
    target_path: ActionTarget,
    event: u32,
    emitter: crate::gameplay::sound::Emitter,
    world_paused: bool,
    clock_frozen: bool,
    clock_paused: bool,
    paused_at: Option<u64>,
    _ticket: Option<crate::gameplay::sound::playing::Ticket>,
}
impl PendingAction {
    fn matches(&self, target: Option<u32>, emitter: Option<crate::gameplay::sound::Emitter>) -> bool {
        emitter.is_none_or(|emitter| self.action.game_object() && self.emitter == emitter)
            && target.is_none_or(|target| self.target_path.as_ref().is_some_and(|path| path.contains(&target)))
    }
}
struct PendingPlay { tick: u64, serial: u64, control: Arc<Control>, paused_at: Option<u64> }
struct PendingObjectRetirement { tick: u64, serial: u64, emitter: crate::gameplay::sound::Emitter }
enum Due { Action(PendingAction), Play(PendingPlay), Retire(PendingObjectRetirement) }
impl Due {
    fn order(&self) -> (u64, u64) {
        match self { Self::Action(action) => (action.tick, action.serial), Self::Play(action) => (action.tick, action.serial), Self::Retire(action) => (action.tick, action.serial) }
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
    pending_actions: Vec<PendingAction>,
    due: Vec<Due>,
    fallbacks: Vec<FallbackVoice>,
    pending_plays: Vec<PendingPlay>,
    action_serial: u64,
    failed: bool,
    error: Option<String>,
    retiring_objects: std::collections::HashSet<crate::gameplay::sound::Emitter>,
    object_voices: std::collections::HashMap<crate::gameplay::sound::Emitter, usize>,
    idle_retirements: Vec<crate::gameplay::sound::Emitter>,
    trace_retirement: bool,
}

impl Engine {
    pub(super) fn has_output(&self) -> bool { self.shared.is_some() }

    pub(super) fn observe_output(&self) -> super::observation::OutputSnapshot {
        let Some(shared) = &self.shared else { return Default::default() };
        let shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        super::observation::OutputSnapshot {
            present: true, blocks: shared.tick, voices: shared.voices.len(),
            fallback_voices: shared.fallbacks.len(), pending_actions: shared.pending_actions.len(),
            pending_plays: shared.pending_plays.len(), due: shared.due.len(),
            retiring_objects: shared.retiring_objects.len(), object_voices: shared.object_voices.values().sum(),
            error: shared.error.clone(),
        }
    }

    /// Read-only acceptance at the canonical level-retired boundary. Menu
    /// contexts belong to the session; exempt only work actually owned by Menu.
    pub(super) fn validate_level_retirement(&self) -> Result<(), String> {
        let Some(shared) = &self.shared else { return Ok(()) };
        let shared = shared.lock().map_err(|_| "audio output owner is poisoned")?;
        if shared.failed { return Err(format!("audio output failed: {:?}", shared.error)); }
        let menu = crate::gameplay::sound::Emitter::Menu;
        let menu_control = |control: &Arc<Control>| shared.voices.iter()
            .any(|voice| Arc::ptr_eq(&voice.control, control) && voice.audio.emitter() == menu);
        let voices = shared.voices.iter().filter(|voice| voice.audio.emitter() != menu).count();
        let fallbacks = shared.fallbacks.iter().filter(|voice| voice.emitter != menu).count();
        let actions = shared.pending_actions.iter().filter(|action| action.emitter != menu).count();
        let plays = shared.pending_plays.iter().filter(|play| !menu_control(&play.control)).count();
        let due = shared.due.iter().filter(|due| match due {
            Due::Action(action) => action.emitter != menu,
            Due::Retire(retire) => retire.emitter != menu,
            Due::Play(play) => !menu_control(&play.control),
        }).count();
        let retiring = shared.retiring_objects.iter().filter(|&&object| object != menu).count();
        let voice_objects = shared.object_voices.keys().filter(|&&object| object != menu).count();
        let idle = shared.idle_retirements.iter().filter(|&&object| object != menu).count();
        if voices + fallbacks + actions + plays + due + retiring + voice_objects + idle != 0 {
            return Err(format!("audio level output retained non-Menu owners: voices={voices} fallbacks={fallbacks} actions={actions} plays={plays} due={due} retiring={retiring} voice_objects={voice_objects} idle={idle}"));
        }
        Ok(())
    }

    pub fn retain_nodes(&self, actors: &std::collections::HashSet<u32>) {
        let Some(shared) = &self.shared else { return };
        let shared = shared.lock().unwrap_or_else(|e| e.into_inner());
        let mut switches = shared.switches.lock().unwrap_or_else(|e| e.into_inner());
        switches.definitions.retain(|node, _| actors.contains(node));
        switches.controllers.retain_objects(|emitter| emitter == crate::gameplay::sound::Emitter::Menu);
    }

    /// Host equivalent of releasing the outgoing world's audio objects.
    /// Keep UI voices and the output/bus clock; old world requests must not
    /// execute after another map reuses its component handles.
    pub fn release_level(&self) {
        let Some(shared) = &self.shared else { return };
        let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
        let menu = crate::gameplay::sound::Emitter::Menu;
        shared.retain_voices(|voice| {
            let keep = voice.audio.emitter() == menu;
            if !keep { voice.control.stopped.store(true, Ordering::Relaxed); }
            keep
        });
        shared.retain_fallbacks(|voice| {
            let keep = voice.emitter == menu;
            if !keep { voice.control.stopped.store(true, Ordering::Relaxed); }
            keep
        });
        shared.pending_actions.retain(|pending| pending.emitter == menu);
        shared.pending_plays.retain(|pending| !pending.control.stopped.load(Ordering::Relaxed));
        shared.due.retain(|due| match due {
            Due::Action(pending) => pending.emitter == menu,
            Due::Play(pending) => !pending.control.stopped.load(Ordering::Relaxed),
            Due::Retire(pending) => pending.emitter == menu,
        });
    }

    /// 140708620/1400c7ab0 release the component's actual sound object,
    /// including scheduled work. Keep every other object and the output clock.
    /// 1400c7a20 ->1400c8150: return after preceding object posts, then
    /// retain the object until the output owner acknowledges its final voice.
    pub(super) fn release_object(&self, emitter: crate::gameplay::sound::Emitter) -> Result<(), String> {
        let Some(shared) = &self.shared else { return Ok(()); };
        let mut shared = shared.lock().map_err(|_| "audio output owner is poisoned")?;
        if shared.failed { return Err("audio output stopped before object retirement".into()); }
        if shared.retiring_objects.contains(&emitter) || shared.due.iter().any(|entry|
            matches!(entry, Due::Retire(retire) if retire.emitter == emitter)) { return Ok(()); }
        let tick = shared.tick;
        let serial = shared.next_action();
        shared.due.push(Due::Retire(PendingObjectRetirement { tick, serial, emitter }));
        Ok(())
    }

    pub(super) fn object_retirement_pending(&self, emitter: crate::gameplay::sound::Emitter) -> Result<bool, String> {
        let Some(shared) = &self.shared else { return Ok(false); };
        let shared = shared.lock().map_err(|_| "audio output owner is poisoned")?;
        if shared.failed { return Err("audio output stopped before object retirement completed".into()); }
        Ok(shared.retiring_objects.contains(&emitter) || shared.due.iter().any(|entry|
            matches!(entry, Due::Retire(retire) if retire.emitter == emitter)))
    }

    pub(super) fn level_emitters(&self) -> std::collections::HashSet<crate::gameplay::sound::Emitter> {
        let Some(shared) = &self.shared else { return Default::default(); };
        let shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        shared.voices.iter().map(|voice| voice.audio.emitter())
            .chain(shared.fallbacks.iter().map(|voice| voice.emitter))
            .chain(shared.pending_actions.iter().map(|action| action.emitter))
            .chain(shared.retiring_objects.iter().copied())
            .chain(shared.due.iter().filter_map(|due| match due {
                Due::Action(action) => Some(action.emitter), Due::Retire(action) => Some(action.emitter), Due::Play(_) => None,
            })).collect()
    }


    pub fn new(mixer: &crate::gameplay::sound::mix::Mixer, states: StateGroups, base: &std::path::Path) -> Result<(Self, OutputAudio), String> {
        let activity = Arc::new(Activity::new(mixer)?);
        let ducking = Ducking::new(mixer)?;
        ducking.publish(&activity);
        let buses = Buses::new(mixer, &base, activity.clone())?;
        let shared = Arc::new(Mutex::new(Shared { buses, activity: activity.clone(), ducking, tick: 0,
            voices: Vec::new(), states, switches: Arc::new(Mutex::new(graph::SwitchControls::default())),
            pending_actions: Vec::new(), due: Vec::new(), fallbacks: Vec::new(), pending_plays: Vec::new(), action_serial: 0, failed: false, error: None, retiring_objects: Default::default(), object_voices: Default::default(), idle_retirements: Vec::new(), trace_retirement: std::env::var("DH2_DEBUG_AUDIO_RESOURCES").as_deref() == Ok("1") }));
        let output = OutputAudio { shared: shared.clone(), channels: ChannelCount::new(2).ok_or("invalid host stereo format")?,
            rate: SampleRate::new(48000).ok_or("invalid host processing rate")? };
        Ok((Self { shared: Some(shared), activity: Some(activity), playing_ids: Default::default() }, output))
    }

    pub fn activity(&self) -> Option<Arc<Activity>> { self.activity.clone() }
    /// Host event-instance keys keep every action from one Post in the same
    /// playing-ID scope. Zero remains the wildcard/invalid value.
    pub fn playing_id(&self) -> u32 {
        self.playing_ids.allocate()
    }
    pub(super) fn playing_ids(&self) -> crate::gameplay::sound::playing::PlayingIds { self.playing_ids.clone() }

    pub fn post_state(&self, action: crate::assets::wwise::StateAction, target_path: ActionTarget, event: u32,
        emitter: crate::gameplay::sound::Emitter, clock_frozen: bool) -> Result<(), String> {
        self.post_state_with_ticket(action, target_path, event, emitter, None, clock_frozen)
    }

    pub fn post_stop(&self, action: crate::assets::wwise::StopAction, target_path: ActionTarget, event: u32,
        emitter: crate::gameplay::sound::Emitter, clock_frozen: bool) -> Result<(), String> {
        self.post_stop_with_ticket(action, target_path, event, emitter, None, clock_frozen)
    }

    pub fn post_state_with_ticket(&self, action: crate::assets::wwise::StateAction, target_path: ActionTarget, event: u32,
        emitter: crate::gameplay::sound::Emitter, ticket: Option<crate::gameplay::sound::playing::Ticket>, clock_frozen: bool) -> Result<(), String> {
        self.post_action(Action::State(action), target_path, event, emitter, ticket, clock_frozen)
    }

    pub fn post_stop_with_ticket(&self, action: crate::assets::wwise::StopAction, target_path: ActionTarget, event: u32,
        emitter: crate::gameplay::sound::Emitter, ticket: Option<crate::gameplay::sound::playing::Ticket>, clock_frozen: bool) -> Result<(), String> {
        self.post_action(Action::Stop(action), target_path, event, emitter, ticket, clock_frozen)
    }

    fn post_action(&self, action: Action, target_path: ActionTarget, event: u32,
        emitter: crate::gameplay::sound::Emitter, ticket: Option<crate::gameplay::sound::playing::Ticket>, clock_frozen: bool) -> Result<(), String> {
        let shared = self.shared.as_ref().ok_or("shared audio action clock is unavailable")?;
        let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        if shared.failed { return Err("shared audio action clock stopped after a processing error".into()); }
        let delay = action.delay().sample_frames(|| rand::random_range(0..=32767));
        // 1410e5600 passes a quotient tick and remainder sample offset. Stop
        // (141127000) and Set State (14113c610) execute at the quotient tick.
        let tick = shared.tick.wrapping_add(u64::from(delay) / FRAMES as u64);
        let serial = shared.next_action();
        let pending = PendingAction { tick, serial, action, target_path, event, emitter, world_paused: false,
            clock_frozen, clock_paused: false, paused_at: None, _ticket: ticket };
        if delay < FRAMES as u32 { shared.execute(pending)?; }
        else { shared.pending_actions.push(pending); }
        Ok(())
    }

    pub fn register_fallback(&self, playing: graph::Playing, event: u32,
        emitter: crate::gameplay::sound::Emitter) -> Result<FallbackHandle, String> {
        let shared = self.shared.as_ref().ok_or("shared audio action clock is unavailable")?;
        let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
        if shared.failed { return Err("shared audio action clock stopped after a processing error".into()); }
        let control = Arc::new(Control { paused: AtomicBool::new(false), stopped: AtomicBool::new(false), finished: AtomicBool::new(false) });
        *shared.object_voices.entry(emitter).or_default() += 1;
        shared.fallbacks.push(FallbackVoice { playing, control: control.clone(), event, emitter });
        Ok(FallbackHandle(control))
    }

    pub fn stop_event(&self, event: u32, emitter: crate::gameplay::sound::Emitter) {
        if let Some(shared) = &self.shared {
            let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
            shared.pending_actions.retain(|pending| pending.event != event || pending.emitter != emitter);
            shared.due.retain(|due| !matches!(due, Due::Action(pending) if pending.event == event && pending.emitter == emitter));
            for voice in &shared.fallbacks {
                if voice.event == event && voice.emitter == emitter {
                    if voice.control.paused.load(Ordering::Relaxed) { voice.control.finished.store(true, Ordering::Relaxed); }
                    else { voice.playing.stop_all(0, 4); }
                }
            }
            for voice in &mut shared.voices {
                if voice.event != event || voice.audio.emitter() != emitter { continue; }
                voice.audio.stop_all(0, 4, voice.control.paused.load(Ordering::Relaxed));
                if voice.audio.cancelled() { voice.control.finished.store(true, Ordering::Relaxed); }
            }
        }
    }

    pub fn update_action_pause(&self, world_change: Option<bool>, frozen: impl Fn(crate::gameplay::sound::Emitter) -> bool) {
        if let Some(shared) = &self.shared {
            let mut shared = shared.lock().unwrap_or_else(|error| error.into_inner());
            let tick = shared.tick;
            for pending in &mut shared.pending_actions {
                //709050 snapshots existing IDs only on entry to stopped time.
                //A later accepted action starts outside that older snapshot.
                if let Some(paused) = world_change { pending.world_paused = paused; }
                let clock_frozen = frozen(pending.emitter);
                if clock_frozen != pending.clock_frozen {
                    pending.clock_frozen = clock_frozen;
                    pending.clock_paused = clock_frozen;
                }
                if pending.world_paused || pending.clock_paused {
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
        *shared.object_voices.entry(audio.emitter()).or_default() += 1;
        if shared.trace_retirement {
            eprintln!("AUDIO_VOICE_ADMITTED event={event:#010x} playing_id={playing_id} object={:?} delay_quanta={quanta} output_tick={}", audio.emitter(), shared.tick);
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
    fn retain_voices(&mut self, mut keep: impl FnMut(&Voice) -> bool) {
        let counts = &mut self.object_voices;
        let retiring = &self.retiring_objects;
        let idle = &mut self.idle_retirements;
        self.voices.retain(|voice| {
            if keep(voice) { return true; }
            Self::voice_removed(counts, retiring, idle, voice.audio.emitter());
            false
        });
    }

    fn voice_removed(counts: &mut std::collections::HashMap<crate::gameplay::sound::Emitter, usize>,
        retiring: &std::collections::HashSet<crate::gameplay::sound::Emitter>,
        idle: &mut Vec<crate::gameplay::sound::Emitter>, emitter: crate::gameplay::sound::Emitter) {
        let Some(count) = counts.get_mut(&emitter) else { debug_assert!(false, "sound voice has no object owner"); return };
        *count -= 1;
        if *count == 0 {
            counts.remove(&emitter);
            if retiring.contains(&emitter) { idle.push(emitter); }
        }
    }

    fn retain_fallbacks(&mut self, mut keep: impl FnMut(&FallbackVoice) -> bool) {
        let counts = &mut self.object_voices;
        let retiring = &self.retiring_objects;
        let idle = &mut self.idle_retirements;
        self.fallbacks.retain(|voice| {
            if keep(voice) { return true; }
            Self::voice_removed(counts, retiring, idle, voice.emitter);
            false
        });
    }

    fn begin_retirement(&mut self, emitter: crate::gameplay::sound::Emitter) {
        if self.trace_retirement { eprintln!("AUDIO_OBJECT_RETURN emitter={emitter:?} tick={} voices={}", self.tick, self.object_voices.get(&emitter).copied().unwrap_or(0)); }
        self.pending_actions.retain(|pending| pending.emitter != emitter);
        self.due.retain(|due| !matches!(due, Due::Action(action) if action.emitter == emitter));
        self.stop_actor(None, Some(emitter), 0, 4);
        // Delayed contexts have not entered the renderer; they cannot drain
        // a final mixed buffer. The native pending-action owner cancels them.
        for voice in &self.voices {
            if voice.audio.emitter() == emitter && self.pending_plays.iter().any(|pending| Arc::ptr_eq(&pending.control, &voice.control)) {
                voice.control.finished.store(true, Ordering::Relaxed);
            }
        }
        if self.retiring_objects.insert(emitter) && !self.object_voices.contains_key(&emitter) {
            self.idle_retirements.push(emitter);
        }
    }

    fn finish_idle_retirements(&mut self) {
        while let Some(emitter) = self.idle_retirements.pop() {
            if self.object_voices.contains_key(&emitter) { continue; }
            self.switches.lock().unwrap_or_else(|error| error.into_inner())
                .controllers.retain_objects(|object| object != emitter);
            self.retiring_objects.remove(&emitter);
            if self.trace_retirement { eprintln!("AUDIO_OBJECT_IDLE emitter={emitter:?} tick={}", self.tick); }
        }
    }

    fn next_action(&mut self) -> u64 { let serial = self.action_serial; self.action_serial = serial.wrapping_add(1); serial }
    fn execute(&mut self, pending: PendingAction) -> Result<(), String> {
        match pending.action {
            Action::State(action) => self.set_state(action.group, action.value)?,
            Action::Stop(action) => {
                // 141127000 does not execute Target Stop or draw its fade
                // modifiers when 141115df0 cannot find that target actor.
                if !action.all && pending.target_path.is_none() { return Ok(()); }
                let target = (!action.all).then_some(action.target);
                let emitter = action.game_object.then_some(pending.emitter);
                let milliseconds = action.transition.sample_ms(|| rand::random_range(0..=32767));
                self.stop_actor(target, emitter, milliseconds, action.transition.curve);
                // Stop live contexts first, then cancel both active and paused
                // pending actions, including later entries due in this block.
                self.cancel_actions(target, emitter);
            },
        }
        Ok(())
    }

    fn cancel_actions(&mut self, target: Option<u32>, emitter: Option<crate::gameplay::sound::Emitter>) {
        self.pending_actions.retain(|pending| !pending.matches(target, emitter));
        self.due.retain(|due| !matches!(due, Due::Action(pending) if pending.matches(target, emitter)));
    }

    fn synchronize_controls(&mut self) {
        self.retain_fallbacks(|voice| !voice.control.stopped.load(Ordering::Relaxed) && !voice.control.finished.load(Ordering::Relaxed));
        self.retain_voices(|voice| {
            let stopped = voice.control.stopped.load(Ordering::Relaxed) || voice.control.finished.load(Ordering::Relaxed);
            if stopped { voice.control.finished.store(true, Ordering::Relaxed); }
            !stopped
        });
        for voice in &mut self.voices {
            voice.audio.pause_controllers(voice.control.paused.load(Ordering::Relaxed));
            voice.audio.flush_stops(voice.control.paused.load(Ordering::Relaxed));
            if voice.audio.cancelled() { voice.control.finished.store(true, Ordering::Relaxed); }
        }
    }

    fn stop_actor(&mut self, target: Option<u32>, emitter: Option<crate::gameplay::sound::Emitter>, milliseconds: i32, curve: u8) {
        for voice in &self.fallbacks {
            if voice.control.stopped.load(Ordering::Relaxed) || emitter.is_some_and(|emitter| emitter != voice.emitter) { continue; }
            let paused = voice.control.paused.load(Ordering::Relaxed);
            if paused && target.is_none() { voice.control.finished.store(true, Ordering::Relaxed); continue; }
            let milliseconds = if paused { 0 } else { milliseconds };
            if let Some(target) = target { voice.playing.stop(target, milliseconds, curve); }
            else { voice.playing.stop_all(milliseconds, curve); }
        }
        for voice in &mut self.voices {
            if emitter.is_some_and(|emitter| emitter != voice.audio.emitter()) { continue; }
            // 1411068e0 stops a paused PBI immediately instead of waiting for
            // a fade on its stopped playback clock.
            let paused = voice.control.paused.load(Ordering::Relaxed);
            let milliseconds = if paused { 0 } else { milliseconds };
            if let Some(target) = target { voice.audio.stop_actor(target, milliseconds, curve, paused); }
            else { voice.audio.stop_all(milliseconds, curve, paused); }
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
        for pending in std::mem::take(&mut self.pending_actions) {
            if pending.paused_at.is_some() || pending.tick > tick { self.pending_actions.push(pending); }
            else { self.due.push(Due::Action(pending)); }
        }
        for mut pending in std::mem::take(&mut self.pending_plays) {
            if pending.control.stopped.load(Ordering::Relaxed) || pending.control.finished.load(Ordering::Relaxed) { continue; }
            if pending.control.paused.load(Ordering::Relaxed) {
                if pending.paused_at.is_none() { pending.paused_at = Some(tick); }
            } else if let Some(start) = pending.paused_at.take() { pending.tick = pending.tick.wrapping_add(tick.wrapping_sub(start)); }
            if pending.paused_at.is_some() || pending.tick > tick { self.pending_plays.push(pending); }
            else { self.due.push(Due::Play(pending)); }
        }
        self.due.sort_by_key(|action| std::cmp::Reverse(action.order()));
        while let Some(action) = self.due.pop() {
            match action {
                Due::Action(pending) => self.execute(pending)?,
                Due::Retire(pending) => self.begin_retirement(pending.emitter),
                Due::Play(pending) => {
                    if pending.control.stopped.load(Ordering::Relaxed) || pending.control.finished.load(Ordering::Relaxed) { continue; }
                    if let Some(voice) = self.voices.iter_mut().find(|voice| Arc::ptr_eq(&voice.control, &pending.control)) {
                        voice.audio.release_delayed();
                    }
                },
            }
        }
        self.synchronize_controls();
        self.pending_plays.retain(|pending| !pending.control.stopped.load(Ordering::Relaxed) && !pending.control.finished.load(Ordering::Relaxed));
        self.buses.begin();
        for voice in &mut self.voices {
            if voice.control.stopped.load(Ordering::Relaxed) { voice.control.finished.store(true, Ordering::Relaxed); }
            else {
                voice.audio.begin(voice.control.paused.load(Ordering::Relaxed));
                if voice.audio.cancelled() { voice.control.finished.store(true, Ordering::Relaxed); }
            }
        }
        self.retain_voices(|voice| !voice.control.finished.load(Ordering::Relaxed));
        self.ducking.synchronize(&self.activity, self.tick);
        self.ducking.advance(self.tick);
        self.ducking.publish(&self.activity);
        for frame in 0..FRAMES {
            for voice in &mut self.voices {
                if voice.control.finished.load(Ordering::Relaxed) || voice.control.paused.load(Ordering::Relaxed) { continue; }
                if !voice.audio.frame(&mut self.buses, frame)? { voice.control.finished.store(true, Ordering::Relaxed); }
            }
        }
        self.retain_voices(|voice| !voice.control.finished.load(Ordering::Relaxed));
        self.finish_idle_retirements();
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
                        for voice in &shared.fallbacks { voice.control.finished.store(true, Ordering::Relaxed); }
                        shared.fallbacks.clear();
                        shared.object_voices.clear();
                        shared.idle_retirements.clear();
                        shared.pending_actions.clear();
                        shared.due.clear();
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

pub(super) fn retire(mut commands: Commands, voices: Query<(Entity, &VoiceHandle)>, fallbacks: Query<(Entity, &FallbackHandle)>) {
    for (entity, voice) in &voices { if voice.finished() { commands.entity(entity).try_despawn(); } }
    for (entity, voice) in &fallbacks { if voice.0.finished.load(Ordering::Relaxed) { commands.entity(entity).try_despawn(); } }
}
