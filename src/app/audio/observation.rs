//! Host observation: the sole consumer of the production output without a
//! hardware device. Samples are rendered, decoded and discarded at real wall
//! time/sample-rate pace; no readiness, EOF, End or retirement is synthesized.
use super::*;
use crate::gameplay::sound::playing::{Notification, NotificationKind};
use bevy::audio::{Decodable, Source};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[derive(Resource, Default)]
pub(in crate::app) struct Observation {
    notifications: Arc<Mutex<Vec<Notification>>>,
    admissions: Mutex<Vec<Admission>>,
    ended: Mutex<std::collections::HashSet<u32>>,
    ready: Mutex<std::collections::HashSet<u32>>,
    source_samples: SourceSamples,
}
pub(in crate::app::audio) type SourceSamples=Arc<Mutex<Vec<SourceReceipt>>>;
#[derive(Clone,Debug)]
pub(in crate::app) struct SourceReceipt {
    pub event:u32,pub playing_id:u32,pub object:Emitter,pub selected_source:u32,
    pub frames:u64,pub terminal:bool,pub route_bus:Option<u32>,
    pub route_target_peak:Option<f32>,pub master_target_peak:Option<f32>,
    pub decoded_peak:f32,pub dry_peak:f32,pub aux_peak:f32,pub gain_peak:f32,
    pub route_next_block_gain:Option<f32>,pub master_next_block_gain:Option<f32>,
    pub reason:&'static str,
}
#[derive(Default)]
struct Consumed {
    samples: u64,
    nonzero_samples: u64,
    peak: f32,
    error: Option<String>,
}
#[derive(Default)]
struct Consumer {
    stop: AtomicBool,
    consumed: Mutex<Consumed>,
}
/// App owns this resource, so its consumer joins before the Runtime's outer
/// AudioSession/CatalogSession fences are released. Decoder/source Arcs keep
/// their actual banks, packet readers and source owners live until that join.
#[derive(Resource, Default)]
struct Output {
    worker: Option<JoinHandle<()>>,
    consumer: Arc<Consumer>,
    installed: bool,
}
impl Drop for Output {
    fn drop(&mut self) {
        self.consumer.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

pub(super) fn attach(world: &mut World) {
    let Some(observer) = world
        .get_resource::<Observation>()
        .map(|observer| observer.notifications.clone())
    else {
        return;
    };
    let source_samples=world.resource::<Observation>().source_samples.clone();
    if let Some(mut scene)=world.get_resource_mut::<spatial::Scene>(){scene.observe_sources(source_samples);}
    if let Some(mut completions) = world.get_resource_mut::<SoundCompletions>() {
        completions.playing.observe_notifications(observer.clone());
    }
    if let Some(sounds) =
        world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>()
    {
        sounds.lock().playing.observe_notifications(observer);
    }
}

/// Install before Startup/map acquisition. Hardware output and this consumer
/// are mutually exclusive: both must never pull the shared bus/sample clock.
pub(in crate::app) fn enable(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<Observation>() {
        return Ok(());
    }
    if world
        .get_resource::<PlaybackBackend>()
        .is_some_and(|mode| *mode == PlaybackBackend::Hardware)
    {
        return Err("disable hardware AudioPlugin before enabling offline audio".into());
    }
    if world
        .get_resource::<bevy_backend::Engine>()
        .is_some_and(|engine| engine.has_output())
    {
        return Err("offline audio must be enabled before output acquisition".into());
    }
    world.insert_resource(Observation::default());
    // Session storage must precede LevelLifetime::begin. The real output is
    // acquired later, but replacing this slot must not make it map-owned.
    world.init_resource::<Output>();
    if world.contains_resource::<PlaybackBackend>() {
        world.insert_resource(PlaybackBackend::Observation);
    }
    attach(world);
    Ok(())
}

pub(super) fn install_output(world: &mut World, output: bevy_backend::OutputAudio) {
    assert!(
        world.contains_resource::<Observation>(),
        "offline audio capability is absent"
    );
    assert!(
        world.get_resource::<Output>().is_none_or(|output| !output.installed),
        "shared output already has its sole offline consumer"
    );
    attach(world);
    let mut decoder = output.decoder();
    let frames = bevy_backend::buses::FRAMES;
    let channels = usize::from(decoder.channels().get());
    let rate = decoder.sample_rate().get();
    let consumer = Arc::new(Consumer::default());
    let thread_consumer = consumer.clone();
    let worker = std::thread::Builder::new()
        .name("audio output observation".into())
        .spawn(move || {
            let start = Instant::now();
            let mut blocks = 0u64;
            while !thread_consumer.stop.load(Ordering::Acquire) {
                // A device callback pulls a complete processing quantum. Use that
                // same Iterator and sample format, then pace its next real block.
                let mut consumed = Consumed::default();
                for _ in 0..frames * channels {
                    let Some(sample) = decoder.next() else {
                        consumed.error = Some("shared output decoder ended unexpectedly".into());
                        break;
                    };
                    consumed.samples += 1;
                    if sample != 0.0 {
                        consumed.nonzero_samples += 1;
                    }
                    consumed.peak = consumed.peak.max(sample.abs());
                }
                let failed = consumed.error.is_some();
                {
                    let mut total = thread_consumer
                        .consumed
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    total.samples += consumed.samples;
                    total.nonzero_samples += consumed.nonzero_samples;
                    total.peak = total.peak.max(consumed.peak);
                    if failed {
                        total.error = consumed.error;
                    }
                }
                if failed {
                    return;
                }
                blocks += 1;
                let deadline = start
                    + Duration::from_secs_f64(blocks as f64 * frames as f64 / f64::from(rate));
                while !thread_consumer.stop.load(Ordering::Acquire) {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    std::thread::park_timeout(deadline.duration_since(now));
                }
            }
        });
    let worker = match worker {
        Ok(worker) => Some(worker),
        Err(error) => {
            consumer
                .consumed
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .error = Some(format!("offline output worker: {error}"));
            None
        }
    };
    world.insert_resource(Output { worker, consumer, installed: true });
}

/// Called after every actual App::update, including ExitDrain updates. The
/// independent consumer keeps pulling the real production decoder during long
/// loading/update work, like a hardware callback; this never advances its clock.
pub(in crate::app) fn poll(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<Observation>() {
        return Err("offline audio is not enabled".into());
    }
    attach(world);
    if let Some(output) = world.get_resource::<Output>() {
        if let Some(error) = &output
            .consumer
            .consumed
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .error
        {
            return Err(error.clone());
        }
        if output
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            return Err("offline output consumer exited unexpectedly".into());
        }
    }
    let engine = world
        .get_resource::<bevy_backend::Engine>()
        .map(|engine| engine.observe_output())
        .unwrap_or_default();
    if engine.present && world.get_resource::<Output>().is_none_or(|output| !output.installed) {
        return Err("shared audio output has no offline consumer".into());
    }
    if let Some(error) = engine.error {
        return Err(format!("shared audio output failed: {error}"));
    }
    if engine.fallback_voices != 0 {
        return Err(
            "offline shared output cannot consume the hardware multichannel fallback".into(),
        );
    }
    // Observe only. Normal PostUpdate and the canonical ExitDrain own actual
    // completion/retirement processing; a second pass would change scheduling.
    Ok(())
}

#[derive(Default, Debug)]
pub(in crate::app) struct OutputSnapshot {
    pub present: bool,
    pub blocks: u64,
    pub voices: usize,
    pub fallback_voices: usize,
    pub pending_actions: usize,
    pub pending_plays: usize,
    pub due: usize,
    pub retiring_objects: usize,
    pub object_voices: usize,
    pub error: Option<String>,
}
#[derive(Debug)]
pub(in crate::app) struct Callback {
    pub playing_id: u32,
    pub declaration: u32,
    pub object: String,
    pub kind: &'static str,
}
#[derive(Clone, Debug)]
pub(in crate::app) struct Admission {
    pub message_id: Option<usize>,
    pub playing_id: u32,
    pub event: u32,
    pub object: String,
    pub cookies_after: u8,
    pub error: Option<String>,
}
impl Observation {
    pub(super) fn record_admission(
        &self,
        message_id: Option<usize>,
        playing_id: u32,
        event: u32,
        object: Emitter,
        cookies_after: u8,
        error: Option<&String>,
    ) {
        self.admissions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(Admission {
                message_id,
                playing_id,
                event,
                object: format!("{object:?}"),
                cookies_after,
                error: error.cloned(),
            });
    }
}
#[derive(Default, Debug)]
pub(in crate::app) struct Snapshot {
    pub source_samples: Vec<SourceReceipt>,
    pub source_processed_ids: Vec<u32>,
    pub ended_ids:Vec<u32>,
    pub output: OutputSnapshot,
    pub samples: u64,
    pub nonzero_samples: u64,
    pub peak: f32,
    pub pending_ids: Vec<u32>,
    pub ordinary_ids: Vec<u32>,
    pub ordinary_cookies: Vec<(String, u8)>,
    pub watched_waiting: usize,
    pub watched_active: usize,
    pub watch_waiting: Vec<(u32, String, u64)>,
    pub watch_active: Vec<(u32, String, u32)>,
    pub fx_pending_ids: Vec<u32>,
    pub fx_ids: Vec<u32>,
    pub fx_cookies: Vec<(String, u8)>,
    pub object_retirements: Vec<(String, String)>,
    pub callbacks: Vec<Callback>,
    pub admissions: Vec<Admission>,
    pub source_jobs: Vec<(u32, u8, bool)>,
}

#[derive(Debug)]
pub(in crate::app) struct StateSnapshot {
    pub value:u32,
    ///Last properties actually published by the canonical subscription owner.
    ///Reading this does not advance a curve, clock, callback or audio graph.
    pub properties:Vec<(u32,[f32;6])>,
}

pub(in crate::app) fn state_snapshot(world:&World,group:u32)->Result<StateSnapshot,String>{
    let registry=world.get_resource::<WwiseRegistry>().ok_or("Wwise registry absent")?;
    let state=registry.states.lock().map_err(|_|"Wwise state owner poisoned")?;
    let mut properties=Vec::new();
    state.visit_properties(|node,values|properties.push((node,values)));
    Ok(StateSnapshot{value:state.current(group),properties})
}

/// Snapshot never drains an owner queue or invokes a callback. Only the copy
/// of packets already consumed by PlayingEvents is drained from the observer.
pub(in crate::app) fn snapshot(world: &mut World) -> Snapshot {
    let mut result = Snapshot::default();
    if let Some(engine) = world.get_resource::<bevy_backend::Engine>() {
        result.output = engine.observe_output();
    }
    if let Some(output) = world.get_resource::<Output>() {
        let consumed = output
            .consumer
            .consumed
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        result.samples = consumed.samples;
        result.nonzero_samples = consumed.nonzero_samples;
        result.peak = consumed.peak;
    }
    if let Some(pending) = world.get_resource::<PendingSounds>() {
        result.pending_ids = pending
            .0
            .iter()
            .filter_map(|queued| queued.ticket.as_ref().map(|ticket| ticket.id().value()))
            .collect();
    }
    if let Some(completions) = world.get_resource::<SoundCompletions>() {
        let (ids, counts) = completions.playing.observation();
        result.ordinary_ids = ids.into_iter().map(|id| id.value()).collect();
        result.ordinary_cookies = counts
            .into_iter()
            .map(|(object, count)| (format!("{object:?}"), count))
            .collect();
        result.watched_waiting = completions.waiting.len();
        result.watched_active = completions.active.len();
        result.watch_waiting = completions
            .waiting
            .iter()
            .map(|(&(event, object), &generation)| (event, format!("{object:?}"), generation))
            .collect();
        result.watch_active = completions
            .active
            .iter()
            .map(|(&(event, object), id)| (event, format!("{object:?}"), id.value()))
            .collect();
    }
    if let Some(sounds) =
        world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>()
    {
        let sounds = sounds.lock();
        let (ids, counts) = sounds.playing.observation();
        result.fx_ids = ids.into_iter().map(|id| id.value()).collect();
        result.fx_cookies = counts
            .into_iter()
            .map(|(object, count)| (format!("{object:?}"), count))
            .collect();
        result.fx_pending_ids = sounds
            .queued
            .iter()
            .map(|ticket| ticket.id().value())
            .collect();
    }
    if let Some(retiring) = world.get_resource::<RetiringSounds>() {
        result.object_retirements = retiring
            .0
            .iter()
            .map(|(object, state)| {
                (
                    format!("{object:?}"),
                    match state {
                        ObjectRetirement::Queued => "Queued",
                        ObjectRetirement::Submitted => "Submitted",
                        ObjectRetirement::Complete => "Complete",
                    }
                    .to_owned(),
                )
            })
            .collect();
    }
    if let Some(decodes) = world.get_resource::<SoundDecodeQueue>() {
        result.source_jobs = decodes
            .0
            .iter()
            .map(|(&id, job)| (id, job.clients, job.result.is_some()))
            .collect();
    }
    if let Some(observer) = world.get_resource::<Observation>() {
        result.source_samples=observer.source_samples.lock().unwrap_or_else(|error|error.into_inner()).clone();
        result.admissions = observer
            .admissions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        result.callbacks = std::mem::take(
            &mut *observer
                .notifications
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
        .into_iter()
        .map(|note| Callback {
            playing_id: note.id.value(),
            declaration: note.event.declaration,
            object: format!("{:?}", note.event.object),
            kind: match note.kind {
                NotificationKind::SourceReady => "SourceReady",
                NotificationKind::End => "End",
                NotificationKind::EventPlaying => "EventPlaying",
            },
        })
        .collect();
        observer
            .ended
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend(
                result
                    .callbacks
                    .iter()
                    .filter(|note| note.kind == "End")
                    .map(|note| note.playing_id),
            );
        observer
            .ready
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend(
                result
                    .callbacks
                    .iter()
                    .filter(|note| note.kind == "SourceReady")
                    .map(|note| note.playing_id),
            );
        result.ended_ids=observer.ended.lock().unwrap_or_else(|error|error.into_inner()).iter().copied().collect();
        result.source_processed_ids=observer.ready.lock().unwrap_or_else(|error|error.into_inner()).iter().copied().collect();
    }
    result
}

// These are observation requests through the production API, not executable
// specifications. No cache is cleared to manufacture cold IO, no IDs/cookies
// are arranged, and only consumed native-owner End packets finish a phase.
use crate::gameplay::effects::sound::handle::SoundHandle;
use crate::gameplay::sound::playing::PlayingId;
use crate::rendering::bevy_backend::world_sounds::{SoundFrame, WorldSounds};

#[derive(Default)]
pub(in crate::app) struct ScenarioOptions {
    pub ordinary_event: Option<u32>,
    pub fx_event_name: Option<String>,
}
#[derive(Debug)]
enum Phase {
    ColdPost,
    ColdDrain,
    WatchOld,
    WatchNew,
    WatchDrain,
    Burst,
    BurstDrain,
    FxCold,
    FxColdDrain,
    FxNatural,
    FxReturn,
    Complete,
}
pub(in crate::app) struct Scenario {
    phase: Phase,
    event: u32,
    fx_reference: u32,
    fx_name: String,
    requests: Vec<usize>,
    accepted: std::collections::HashSet<u32>,
    fx_handle: SoundHandle<Emitter, PlayingId>,
    fx_object: Option<Emitter>,
    fx_id: Option<u32>,
    frames: u64,
    last_observation: String,
}
fn request(world: &mut World, event: u32) -> usize {
    world
        .resource_mut::<Messages<SoundEvent>>()
        .write(SoundEvent(Request::Post {
            event,
            emitter: Emitter::PlayerBody,
        }))
        .id
}
fn stop(world: &mut World, event: u32) {
    world
        .resource_mut::<Messages<SoundEvent>>()
        .write(SoundEvent(Request::Stop {
            event,
            emitter: Emitter::PlayerBody,
        }));
}
fn watch(world: &mut World, event: u32) {
    let mut completions = world.resource_mut::<SoundCompletions>();
    completions.cancel(event, Emitter::PlayerBody);
    completions.watch(event, Emitter::PlayerBody);
}

pub(in crate::app) fn begin_scenario(
    world: &mut World,
    options: ScenarioOptions,
) -> Result<Scenario, String> {
    if !world.contains_resource::<Observation>() || !world.contains_resource::<Output>() {
        return Err("audio scenario requires the live production offline output".into());
    }
    let registry = world.resource::<WwiseRegistry>();
    let has_play = |id: &u32| {
        registry.sources.get(id).is_some_and(|actions| {
            actions
                .iter()
                .any(|action| matches!(action, SourceAction::Play { .. }))
        })
    };
    let event = if let Some(event) = options.ordinary_event {
        if !has_play(&event) {
            return Err(format!(
                "installed ordinary event {event:#x} has no prepared Play source"
            ));
        }
        event
    } else if has_play(&0xc64fd6cf) {
        0xc64fd6cf
    } else {
        registry
            .sources
            .keys()
            .filter(|id| has_play(id))
            .copied()
            .min()
            .ok_or("no installed prepared ordinary Play event")?
    };
    let sounds = world
        .get_resource::<WorldSounds>()
        .ok_or("FX sound bridge is absent")?
        .lock();
    if !sounds.ready {
        return Err("FX sound bridge is not ready".into());
    }
    let (fx_reference, fx_event) = sounds
        .declarations
        .iter()
        .enumerate()
        .filter_map(|(reference, event)| event.as_ref().map(|event| (reference, event)))
        .filter(|(_, event)| {
            options
                .fx_event_name
                .as_ref()
                .is_none_or(|name| *name == event.name)
                && !event.declaration.persistent_loop()
                && event.declaration.duration[1] > 0.0
                && has_play(&event.declaration.id)
        })
        .min_by(|(_, a), (_, b)| a.name.cmp(&b.name))
        .ok_or("no installed finite FX Play declaration available for real EOF observation")?;
    let fx_reference = u32::try_from(fx_reference).map_err(|error| error.to_string())?;
    let fx_name = fx_event.name.clone();
    let baseline = world
        .resource::<SoundCompletions>()
        .playing
        .object_count(Emitter::PlayerBody);
    eprintln!("AUDIO_ACTIVE_LIFETIME begin ordinary={event:#x} fx={fx_name} fx_reference={fx_reference} baseline_player_cookies={baseline} ordinary_prepared=true cold_io_not_assumed=true");
    drop(sounds);
    watch(world, event);
    let first = request(world, event);
    Ok(Scenario {
        phase: Phase::ColdPost,
        event,
        fx_reference,
        fx_name,
        requests: vec![first],
        accepted: Default::default(),
        fx_handle: Default::default(),
        fx_object: None,
        fx_id: None,
        frames: 0,
        last_observation: String::new(),
    })
}

impl Scenario {
    /// An unexpected AppExit cannot stand in for completing the active scenario.
    pub(in crate::app) fn validate_complete(&self, world: &World) -> Result<(), String> {
        if !matches!(self.phase, Phase::Complete) || !self.ended(world) || !self.fx_returned(world) {
            return Err(format!("audio scenario exited before actual completion: {}", self.pending_description()));
        }
        Ok(())
    }
    pub(in crate::app) fn pending_description(&self) -> String {
        format!(
            "audio {:?}: ordinary={:#x}, FX={}, accepted_ids={:?}",
            self.phase, self.event, self.fx_name, self.accepted
        )
    }
    fn ended(&self, world: &World) -> bool {
        let observer = world.resource::<Observation>();
        let ended = observer
            .ended
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.accepted.iter().all(|id| ended.contains(id))
    }
    fn start_fx(&mut self, world: &mut World) -> Result<(), String> {
        let listener = world
            .resource::<spatial::Scene>()
            .listener()
            .ok_or("audio scenario has no actual player listener")?;
        let clock = world
            .get_resource::<crate::rendering::bevy_backend::world_effects::WorldEffectFrames>()
            .and_then(|frames| frames.clock)
            .ok_or("audio scenario has no published FX clock")?;
        let context = SoundFrame {
            entity_context: None,
            axis: listener.axis,
            scale: clock.scale,
            energy:0.,
        };
        let mut sounds = world.resource::<WorldSounds>().clone();
        if !self
            .fx_handle
            .post(&mut sounds, self.fx_reference, listener.position, &context)?
        {
            return Err("actual FX sound admission rejected the observation request".into());
        }
        let object = self
            .fx_handle
            .object()
            .ok_or("accepted FX request has no allocated object")?;
        let bridge = sounds.lock();
        let id = bridge
            .queued
            .iter()
            .rev()
            .find(|ticket| ticket.event().object == object)
            .ok_or("accepted FX post has no actual queued ticket")?
            .id()
            .value();
        self.fx_object = Some(object);
        self.fx_id = Some(id);
        self.accepted.insert(id);
        eprintln!(
            "AUDIO_ACTIVE_LIFETIME fx_post name={} object={object:?} id={id} queued=true",
            self.fx_name
        );
        Ok(())
    }
    fn return_fx(&mut self, world: &mut World) -> Result<(), String> {
        let mut sounds = world.resource::<WorldSounds>().clone();
        self.fx_handle.release(&mut sounds)
    }
    fn fx_returned(&self, world: &World) -> bool {
        let sounds = world.resource::<WorldSounds>().lock();
        self.fx_object.is_none_or(|object| {
            !sounds.objects.contains_key(&object) && !sounds.playing.references_object(object)
        })
    }
}

/// Call once after each full Runtime::step. A watchdog belongs to the caller;
/// elapsed duration cannot substitute for End or successful resource release.
pub(in crate::app) fn poll_scenario(
    world: &mut World,
    scenario: &mut Scenario,
) -> Result<bool, String> {
    let state = snapshot(world);
    scenario.frames += 1;
    for admission in state.admissions.iter().filter(|admission| {
        admission
            .message_id
            .is_some_and(|id| scenario.requests.contains(&id))
    }) {
        if admission.error.is_none() {
            scenario.accepted.insert(admission.playing_id);
        }
    }
    // Include actual queue/decode transitions and rejection reasons, even when
    // IO completes too quickly to observe a pending interval on this machine.
    let signature = format!(
        "{:?}",
        (
            &scenario.phase,
            &state.pending_ids,
            &state.ordinary_ids,
            &state.fx_pending_ids,
            &state.fx_ids,
            &state.source_jobs,
            &state.watch_waiting,
            &state.watch_active,
            &state.object_retirements,
            state.admissions.len(),
            &state.callbacks,
            (state.output.voices, state.output.pending_actions),
        )
    );
    if signature != scenario.last_observation {
        eprintln!(
            "AUDIO_ACTIVE_LIFETIME phase={:?} step={} state={state:?}",
            scenario.phase, scenario.frames
        );
        scenario.last_observation = signature;
    }
    let observed_requests = scenario.requests.iter().all(|id| {
        state
            .admissions
            .iter()
            .any(|admission| admission.message_id == Some(*id))
    });
    match scenario.phase {
        Phase::ColdPost if observed_requests => {
            let pending_seen = state
                .pending_ids
                .iter()
                .any(|id| scenario.accepted.contains(id));
            eprintln!("AUDIO_ACTIVE_LIFETIME ordinary_stop pending_preparation_observed={pending_seen} cold_interleaving_claimed={pending_seen}");
            stop(world, scenario.event);
            scenario.phase = Phase::ColdDrain;
        }
        Phase::ColdDrain if scenario.ended(world) => {
            watch(world, scenario.event);
            scenario.requests.push(request(world, scenario.event));
            scenario.phase = Phase::WatchOld;
        }
        Phase::WatchOld if observed_requests => {
            // Keep the actual old voice/ticket live; cancel only its completion
            // subscription, then re-watch and post through the same real API.
            watch(world, scenario.event);
            scenario.requests.push(request(world, scenario.event));
            scenario.phase = Phase::WatchNew;
        }
        Phase::WatchNew if observed_requests => {
            stop(world, scenario.event);
            scenario.phase = Phase::WatchDrain;
        }
        Phase::WatchDrain if scenario.ended(world) => {
            let replacement = world
                .resource_mut::<SoundCompletions>()
                .take(scenario.event, Emitter::PlayerBody);
            eprintln!("AUDIO_ACTIVE_LIFETIME replacement_completion={replacement:?}");
            if replacement.is_none() {
                return Err(
                    "replacement watched post ended without completing its subscription".into(),
                );
            }
            world
                .resource_mut::<SoundCompletions>()
                .cancel(scenario.event, Emitter::PlayerBody);
            let baseline = world
                .resource::<SoundCompletions>()
                .playing
                .object_count(Emitter::PlayerBody);
            eprintln!("AUDIO_ACTIVE_LIFETIME burst baseline_player_cookies={baseline} attempts=17 external_native_capacity=16");
            for _ in 0..17 {
                scenario.requests.push(request(world, scenario.event));
            }
            scenario.phase = Phase::Burst;
        }
        Phase::Burst if observed_requests => {
            let burst_messages: std::collections::HashSet<_> =
                scenario.requests.iter().rev().take(17).copied().collect();
            let attempts: Vec<_> = state
                .admissions
                .iter()
                .filter(|admission| {
                    admission
                        .message_id
                        .is_some_and(|id| burst_messages.contains(&id))
                })
                .collect();
            let admitted = attempts
                .iter()
                .filter(|admission| admission.error.is_none())
                .count();
            let before = attempts
                .first()
                .map(|admission| admission.cookies_after - u8::from(admission.error.is_none()));
            let maximum = attempts
                .iter()
                .map(|admission| admission.cookies_after)
                .max();
            eprintln!("AUDIO_ACTIVE_LIFETIME burst_result actual_before={before:?} admitted={admitted} rejected={} maximum_cookies={maximum:?} external_native_capacity=16", attempts.len() - admitted);
            stop(world, scenario.event);
            scenario.phase = Phase::BurstDrain;
        }
        Phase::BurstDrain if scenario.ended(world) => {
            scenario.start_fx(world)?;
            scenario.phase = Phase::FxCold;
        }
        Phase::FxCold => {
            scenario.return_fx(world)?;
            scenario.phase = Phase::FxColdDrain;
        }
        Phase::FxColdDrain if scenario.ended(world) && scenario.fx_returned(world) => {
            scenario.start_fx(world)?;
            scenario.phase = Phase::FxNatural;
        }
        Phase::FxNatural if scenario.ended(world) => {
            let id = scenario.fx_id.ok_or("natural FX post has no actual ID")?;
            if !world
                .resource::<Observation>()
                .ready
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .contains(&id)
            {
                return Err(format!("finite FX id {id} ended without actual SourceReady; natural EOF was not exercised"));
            }
            scenario.return_fx(world)?;
            scenario.phase = Phase::FxReturn;
        }
        Phase::FxReturn if scenario.fx_returned(world) && scenario.ended(world) => {
            scenario.phase = Phase::Complete;
        }
        _ => {}
    }
    Ok(matches!(scenario.phase, Phase::Complete))
}


/// Validate actual owner state after successful canonical SessionExit. This
/// observes without pumping queues, stopping sources or retiring any identity.
/// The retained session output and actual Menu contexts are intentionally live.
pub(in crate::app) fn validate_shutdown(world: &mut World) -> Result<Snapshot, String> {
    if !world.contains_resource::<Observation>() { return Err("audio shutdown observation was not enabled".into()); }
    if let Some(output) = world.get_resource::<Output>() {
        if let Some(error) = &output.consumer.consumed.lock().unwrap_or_else(|error| error.into_inner()).error {
            return Err(format!("audio observation consumer failed before shutdown: {error}"));
        }
        if output.installed && output.worker.as_ref().is_none_or(|worker| worker.is_finished()) {
            return Err("audio observation consumer is absent or exited before App drop".into());
        }
    }
    if let Some(engine) = world.get_resource::<bevy_backend::Engine>() {
        if engine.has_output() && world.get_resource::<Output>().is_none_or(|output| !output.installed) {
            return Err("audio output has no actual observation consumer at shutdown".into());
        }
        engine.validate_level_retirement()?;
    }
    let menu = Emitter::Menu;
    if let Some(pending) = world.get_resource::<PendingSounds>() {
        if pending.0.iter().any(|queued| request_emitter(&queued.request) != menu) {
            return Err("audio shutdown retained a non-Menu ordinary request".into());
        }
    }
    if let Some(completions) = world.get_resource::<SoundCompletions>() {
        let (_, cookies) = completions.playing.observation();
        if cookies.iter().any(|&(object, _)| object != menu)
            || completions.waiting.keys().chain(completions.active.keys()).any(|&(_, object)| object != menu) {
            return Err("audio shutdown retained a non-Menu cookie or watched subscription".into());
        }
    }
    if let Some(retiring) = world.get_resource::<RetiringSounds>() {
        if retiring.0.keys().any(|&object| object != menu) {
            return Err("audio shutdown retained a non-Menu object retirement record".into());
        }
    }
    if let Some(sounds) = world.get_resource::<WorldSounds>() {
        let sounds = sounds.lock();
        if !sounds.queued.is_empty() || !sounds.released.is_empty() || !sounds.playing.is_empty() || !sounds.objects.is_empty() {
            return Err("audio shutdown retained FX posts, cookies or allocated objects".into());
        }
    }
    if let Some(decodes) = world.get_resource::<SoundDecodeQueue>() {
        for (&event, job) in &decodes.0 {
            let only_menu_waits = world.get_resource::<SoundCompletions>().is_some_and(|completions| {
                let mut waits = completions.waiting.keys().filter(|&&(id, _)| id == event).peekable();
                waits.peek().is_some() && waits.all(|&(_, object)| object == menu)
            });
            if job.clients != DecodeClient::Watched as u8 || !only_menu_waits {
                return Err(format!("audio shutdown retained non-Menu preparation event={event:#x} clients={}", job.clients));
            }
        }
    }
    Ok(snapshot(world))
}
