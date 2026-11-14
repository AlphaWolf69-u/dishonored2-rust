//! Host observation: the sole consumer of the production output without a
//! hardware device. Samples are rendered, decoded and discarded at real wall
//! time/sample-rate pace; no readiness, EOF, End or retirement is synthesized.
use super::*;
use bevy::audio::{Decodable, Source};
use std::{sync::{Mutex, atomic::{AtomicBool, Ordering}}, thread::JoinHandle, time::{Duration, Instant}};
use crate::gameplay::sound::playing::{Notification, NotificationKind};

#[derive(Resource, Default)]
pub(in crate::app) struct Observation {
    notifications: Arc<Mutex<Vec<Notification>>>,
}
#[derive(Default)]
struct Consumed { samples: u64, nonzero_samples: u64, peak: f32, error: Option<String> }
#[derive(Default)]
struct Consumer { stop: AtomicBool, consumed: Mutex<Consumed> }
/// App owns this resource, so its consumer joins before the Runtime's outer
/// AudioSession/CatalogSession fences are released. Decoder/source Arcs keep
/// their actual banks, packet readers and source owners live until that join.
#[derive(Resource)]
struct Output { worker: Option<JoinHandle<()>>, consumer: Arc<Consumer> }
impl Drop for Output {
    fn drop(&mut self) {
        self.consumer.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() { worker.thread().unpark(); let _ = worker.join(); }
    }
}

pub(super) fn attach(world: &mut World) {
    let Some(observer) = world.get_resource::<Observation>().map(|observer| observer.notifications.clone()) else { return };
    if let Some(mut completions) = world.get_resource_mut::<SoundCompletions>() {
        completions.playing.observe_notifications(observer.clone());
    }
    if let Some(sounds) = world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>() {
        sounds.lock().playing.observe_notifications(observer);
    }
}

/// Install before Startup/map acquisition. Hardware output and this consumer
/// are mutually exclusive: both must never pull the shared bus/sample clock.
pub(in crate::app) fn enable(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<Observation>() { return Ok(()); }
    if world.get_resource::<PlaybackBackend>().is_some_and(|mode| *mode == PlaybackBackend::Hardware) {
        return Err("disable hardware AudioPlugin before enabling offline audio".into());
    }
    if world.get_resource::<bevy_backend::Engine>().is_some_and(|engine| engine.has_output()) {
        return Err("offline audio must be enabled before output acquisition".into());
    }
    world.insert_resource(Observation::default());
    if world.contains_resource::<PlaybackBackend>() { world.insert_resource(PlaybackBackend::Observation); }
    attach(world);
    Ok(())
}

pub(super) fn install_output(world: &mut World, output: bevy_backend::OutputAudio) {
    assert!(world.contains_resource::<Observation>(), "offline audio capability is absent");
    assert!(!world.contains_resource::<Output>(), "shared output already has its sole offline consumer");
    attach(world);
    let mut decoder = output.decoder();
    let frames = bevy_backend::buses::FRAMES;
    let channels = usize::from(decoder.channels().get());
    let rate = decoder.sample_rate().get();
    let consumer = Arc::new(Consumer::default());
    let thread_consumer = consumer.clone();
    let worker = std::thread::Builder::new().name("audio output observation".into()).spawn(move || {
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
                if sample != 0.0 { consumed.nonzero_samples += 1; }
                consumed.peak = consumed.peak.max(sample.abs());
            }
            let failed = consumed.error.is_some();
            {
                let mut total = thread_consumer.consumed.lock().unwrap_or_else(|error| error.into_inner());
                total.samples += consumed.samples; total.nonzero_samples += consumed.nonzero_samples;
                total.peak = total.peak.max(consumed.peak);
                if failed { total.error = consumed.error; }
            }
            if failed { return; }
            blocks += 1;
            let deadline = start + Duration::from_secs_f64(blocks as f64 * frames as f64 / f64::from(rate));
            while !thread_consumer.stop.load(Ordering::Acquire) {
                let now = Instant::now();
                if now >= deadline { break; }
                std::thread::park_timeout(deadline.duration_since(now));
            }
        }
    });
    let worker = match worker {
        Ok(worker) => Some(worker),
        Err(error) => {
            consumer.consumed.lock().unwrap_or_else(|error| error.into_inner()).error = Some(format!("offline output worker: {error}"));
            None
        },
    };
    world.insert_resource(Output { worker, consumer });
}

/// Called after every actual App::update, including ExitDrain updates. The
/// independent consumer keeps pulling the real production decoder during long
/// loading/update work, like a hardware callback; this never advances its clock.
pub(in crate::app) fn poll(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<Observation>() { return Err("offline audio is not enabled".into()); }
    attach(world);
    if let Some(output) = world.get_resource::<Output>() {
        if let Some(error) = &output.consumer.consumed.lock().unwrap_or_else(|error| error.into_inner()).error {
            return Err(error.clone());
        }
        if output.worker.as_ref().is_some_and(|worker| worker.is_finished()) {
            return Err("offline output consumer exited unexpectedly".into());
        }
    }
    let engine = world.get_resource::<bevy_backend::Engine>().map(|engine| engine.observe_output()).unwrap_or_default();
    if engine.present && !world.contains_resource::<Output>() {
        return Err("shared audio output has no offline consumer".into());
    }
    if let Some(error) = engine.error { return Err(format!("shared audio output failed: {error}")); }
    if engine.fallback_voices != 0 {
        return Err("offline shared output cannot consume the hardware multichannel fallback".into());
    }
    // Run the same completion/retirement consumer used by exclusive lifecycle
    // callers. FX packet consumption remains in its normal update system.
    if world.contains_resource::<SoundCompletions>() { super::poll_object_retirements(world)?; }
    Ok(())
}

#[derive(Default, serde::Serialize)]
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
#[derive(serde::Serialize)]
pub(in crate::app) struct Callback {
    pub playing_id: u32,
    pub declaration: u32,
    pub object: String,
    pub kind: &'static str,
}
#[derive(Default, serde::Serialize)]
pub(in crate::app) struct Snapshot {
    pub output: OutputSnapshot,
    pub samples: u64,
    pub nonzero_samples: u64,
    pub peak: f32,
    pub pending_ids: Vec<u32>,
    pub ordinary_ids: Vec<u32>,
    pub ordinary_cookies: Vec<(String, u8)>,
    pub watched_waiting: usize,
    pub watched_active: usize,
    pub fx_pending_ids: Vec<u32>,
    pub fx_ids: Vec<u32>,
    pub fx_cookies: Vec<(String, u8)>,
    pub object_retirements: Vec<(String, String)>,
    pub callbacks: Vec<Callback>,
}

/// Snapshot never drains an owner queue or invokes a callback. Only the copy
/// of packets already consumed by PlayingEvents is drained from the observer.
pub(in crate::app) fn snapshot(world: &mut World) -> Snapshot {
    let mut result = Snapshot::default();
    if let Some(engine) = world.get_resource::<bevy_backend::Engine>() { result.output = engine.observe_output(); }
    if let Some(output) = world.get_resource::<Output>() {
        let consumed = output.consumer.consumed.lock().unwrap_or_else(|error| error.into_inner());
        result.samples = consumed.samples; result.nonzero_samples = consumed.nonzero_samples; result.peak = consumed.peak;
    }
    if let Some(pending) = world.get_resource::<PendingSounds>() {
        result.pending_ids = pending.0.iter().filter_map(|queued| queued.ticket.as_ref().map(|ticket| ticket.id().value())).collect();
    }
    if let Some(completions) = world.get_resource::<SoundCompletions>() {
        let (ids, counts) = completions.playing.observation();
        result.ordinary_ids = ids.into_iter().map(|id| id.value()).collect();
        result.ordinary_cookies = counts.into_iter().map(|(object, count)| (format!("{object:?}"), count)).collect();
        result.watched_waiting = completions.waiting.len(); result.watched_active = completions.active.len();
    }
    if let Some(sounds) = world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>() {
        let sounds = sounds.lock();
        let (ids, counts) = sounds.playing.observation();
        result.fx_ids = ids.into_iter().map(|id| id.value()).collect();
        result.fx_cookies = counts.into_iter().map(|(object, count)| (format!("{object:?}"), count)).collect();
        result.fx_pending_ids = sounds.queued.iter().map(|ticket| ticket.id().value()).collect();
    }
    if let Some(retiring) = world.get_resource::<RetiringSounds>() {
        result.object_retirements = retiring.0.iter().map(|(object, state)| (format!("{object:?}"), match state { ObjectRetirement::Queued => "Queued", ObjectRetirement::Submitted => "Submitted", ObjectRetirement::Complete => "Complete" }.to_owned())).collect();
    }
    if let Some(observer) = world.get_resource::<Observation>() {
        result.callbacks = std::mem::take(&mut *observer.notifications.lock().unwrap_or_else(|error| error.into_inner())).into_iter().map(|note| Callback {
            playing_id: note.id.value(), declaration: note.event.declaration, object: format!("{:?}", note.event.object),
            kind: match note.kind { NotificationKind::SourceReady => "SourceReady", NotificationKind::End => "End", NotificationKind::EventPlaying => "EventPlaying" },
        }).collect();
    }
    result
}
