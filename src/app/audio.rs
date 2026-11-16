//! Wwise events from pause UI and player gameplay.
use bevy::{audio::{AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings}, prelude::*};
use hashbrown::HashMap;
use std::{path::PathBuf, sync::Arc};
use crate::gameplay::sound::{objects, Emitter, Request};
mod pcm;
mod graph;
mod bevy_backend;
mod fade;
pub(super) mod contacts;
mod acquisition;
mod reservations;
pub(super) use reservations::{reserve_events, events_ready, release_events};
pub(super) fn contact_callbacks_complete(world:&World,owner:crate::gameplay::entity_lifetime::EntityId)->bool{
    world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>()
        .is_none_or(|sounds| !sounds.lock().references_stimulus_owner(owner))
}
pub(super) mod observation;
use crate::assets::declaration_texts;
mod effects;
mod stimulus;
mod spatial;
mod session;
pub(super) use session::{close_admission as close_session_admission, work as acquire_audio_work, Session as AudioSession};
pub(super) mod level;
pub(super) mod music;
pub(super) mod audiographs;
pub(super) use contacts::ContactSound;

#[derive(Message, Clone, Copy)]
pub struct SoundEvent(pub Request);

/// The scheduled and exclusive lifecycle consumer share one message cursor.
#[derive(Resource, Default)]
struct SoundEventCursor(bevy::ecs::message::MessageCursor<SoundEvent>);

/// One in-flight decode per event and level bank generation. Each client has
/// its own lease, so canceling an FX or UI request does not cancel another.
#[derive(Clone, Copy)]
enum DecodeClient { Effects = 1, Watched = 2, Prototype = 4 }
struct DecodeJob {
    clients: u8,
    task: Option<std::sync::Mutex<acquisition::Job<Result<Vec<SourceAction>, String>>>>,
    result: Option<Result<Arc<Vec<SourceAction>>, String>>,
}
#[derive(Resource, Default)]
struct SoundDecodeQueue(std::collections::HashMap<u32, DecodeJob>);
impl SoundDecodeQueue {
    fn request(&mut self, event: u32, client: DecodeClient, registry: &WwiseRegistry) {
        let job = self.0.entry(event).or_insert_with(|| {
            let work = match session::work() {
                Ok(work) => work,
                Err(error) => return DecodeJob { clients: 0, task: None, result: Some(Err(error.into())) },
            };
            let snapshot = registry.decode_snapshot();
            let task = registry.source_jobs.upgrade().ok_or_else(|| "sound preparation owner is absent".to_owned())
                .map(|pool| pool.submit("event", event, move || {
                    let _work = work;
                    if _work.is_closing() { return Err("audio session is closing".into()); }
                    let started = crate::rendering::bevy_backend::world_effects::trace_lifecycle().then(std::time::Instant::now);
                    let result = snapshot.decode_event(event).map_err(|error| error.to_string());
                    if let Some(started) = started {
                        eprintln!("FX_LIFECYCLE audio={event:#010x} success={} decode_ms={:.3}", result.is_ok(), started.elapsed().as_secs_f64() * 1000.0);
                    }
                    result
                }));
            match task {
                Ok(task) => DecodeJob { clients: 0, result: None, task: Some(std::sync::Mutex::new(task)) },
                Err(error) => DecodeJob { clients: 0, result: Some(Err(error)), task: None },
            }
        });
        job.clients |= client as u8;
    }
    fn poll(&mut self) {
        for job in self.0.values_mut() {
            let Some(task) = job.task.as_mut() else { continue };
            let task = task.get_mut().unwrap_or_else(|error| error.into_inner());
            if let Some(result) = task.take() {
                job.result = (job.clients != 0).then(|| result.map(Arc::new));
                job.task = None;
            }
        }
        self.0.retain(|_, job| job.clients != 0 || job.task.is_some());
    }
    fn result(&self, event: u32) -> Option<Result<Arc<Vec<SourceAction>>, String>> {
        self.0.get(&event).and_then(|job| job.result.clone())
    }
    fn release(&mut self, event: u32, client: DecodeClient) {
        if let Some(job) = self.0.get_mut(&event) { job.clients &= !(client as u8); }
        self.0.retain(|_, job| job.clients != 0 || job.task.is_some());
    }
    fn retain_client(&mut self, client: DecodeClient, wanted: &std::collections::HashSet<u32>) {
        self.0.retain(|event, job| {
            if !wanted.contains(event) { job.clients &= !(client as u8); }
            job.clients != 0 || job.task.is_some()
        });
    }
}

/// Optional completion subscriptions for ordinary Wwise posts. A successful
/// post is finished only after every submitted voice releases its event ticket.
#[derive(Resource, Default)]
pub(super) struct SoundCompletions {
    // Host subscriptions have generations: cancel/re-watch must not resurrect
    // an older pending post or let its End complete the replacement.
    waiting: std::collections::HashMap<(u32, Emitter), u64>,
    next_watch: u64,
    completed: std::collections::HashMap<(u32, Emitter), bool>,
    active: std::collections::HashMap<(u32, Emitter), crate::gameplay::sound::playing::PlayingId>,
    playing: crate::gameplay::sound::playing::PlayingEvents,
    started: std::collections::HashSet<crate::gameplay::sound::playing::PlayingId>,
    decode_failures: std::collections::HashMap<u32, String>,
    dynamic_sources: std::collections::HashSet<u32>,
    retired_sources: Vec<u32>,
}
impl SoundCompletions {
    pub fn watch(&mut self, event: u32, emitter: Emitter) {
        self.completed.remove(&(event, emitter));
        self.next_watch = self.next_watch.checked_add(1).expect("sound watch identity exhausted");
        self.waiting.insert((event, emitter), self.next_watch);
        self.active.remove(&(event, emitter));
    }
    /// `true` means at least one voice was submitted and has now ended;
    /// `false` means preparation, streaming or submission failed.
    pub fn take(&mut self, event: u32, emitter: Emitter) -> Option<bool> {
        self.completed.remove(&(event, emitter))
    }
    pub fn cancel(&mut self, event: u32, emitter: Emitter) {
        self.waiting.remove(&(event, emitter));
        self.completed.remove(&(event, emitter));
        self.active.remove(&(event, emitter));
        self.retire_if_unused(event);
    }
    fn failed(&mut self, event: u32, emitter: Emitter) {
        if self.waiting.remove(&(event, emitter)).is_some() { self.completed.insert((event, emitter), false); }
        self.retire_if_unused(event);
    }
    fn failed_watch(&mut self, event: u32, emitter: Emitter, watch: Option<u64>) {
        if watch.is_some() && self.waiting.get(&(event, emitter)).copied() == watch {
            self.failed(event, emitter);
        }
    }
    fn retire_if_unused(&mut self, event: u32) {
        if !self.waiting.keys().any(|(id, _)| *id == event)
            && !self.active.keys().any(|(id, _)| *id == event)
            && self.dynamic_sources.remove(&event) {
            self.retired_sources.push(event);
        }
    }
    fn pump(&mut self) {
        use crate::gameplay::sound::playing::NotificationKind;
        for note in self.playing.take_notifications() {
            // Unwatched posts still own a real object cookie and source ticket,
            // but cannot complete a separate watched post with the same key.
            if note.event.callbacks & 8 == 0 { continue; }
            match note.kind {
                NotificationKind::SourceReady => { self.started.insert(note.id); },
                NotificationKind::End => {
                    let submitted = self.started.remove(&note.id);
                    let key = (note.event.declaration, note.event.object);
                    if self.active.get(&key) == Some(&note.id) {
                        self.active.remove(&key);
                        self.completed.insert(key, submitted);
                        self.retire_if_unused(key.0);
                    }
                },
                NotificationKind::EventPlaying => {},
            }
        }
    }
}
#[derive(Resource, Default)]
struct WwiseRegistry {
    banks: Arc<Vec<Bank>>,
    packages: HashMap<PathBuf, Arc<crate::assets::wwise::Package>>,
    shared_banks: usize,
    shared_attenuations: std::collections::HashSet<u32>,
    attenuations: Arc<HashMap<u32, Arc<crate::assets::wwise::Attenuation>>>,
    library: Arc<crate::assets::wwise::PlaybackBanks<'static>>,
    sources: HashMap<u32, Arc<Vec<SourceAction>>>,
    /// Runtime source admission shares one event graph. The bits represent
    /// the existing FX/watched owners and the prototype reservation owner.
    dynamic_source_clients: std::collections::HashMap<u32, u8>,
    scheduled_selections: graph::Selections,
    states: bevy_backend::StateGroups,
    /// The level's listed sound events were prepared at load.
    preloaded: bool,
    level_prepared: bool,
    stream_acquisition: Arc<std::sync::Mutex<(u64, u64, u64, std::time::Duration)>>,
    stream_io: Arc<crate::assets::wwise::stream_io::ReadPool>,
    /// The plugin owns the worker. Snapshots/queued inputs use a weak link so
    /// the worker queue cannot form an ownership cycle with its own jobs.
    source_jobs: std::sync::Weak<acquisition::PreparationQueue>,
    media_generation: u64,
}

impl WwiseRegistry {
    fn publish_dynamic_source(&mut self, event: u32, actions: Arc<Vec<SourceAction>>, client: DecodeClient) {
        if let hashbrown::hash_map::Entry::Vacant(entry) = self.sources.entry(event) {
            entry.insert(actions);
            self.dynamic_source_clients.insert(event, client as u8);
        } else {
            self.claim_dynamic_source(event, client);
        }
    }
    fn claim_dynamic_source(&mut self, event: u32, client: DecodeClient) -> bool {
        if let Some(clients) = self.dynamic_source_clients.get_mut(&event) {
            *clients |= client as u8;
            true
        } else { false }
    }
    fn release_dynamic_source(&mut self, event: u32, client: DecodeClient) {
        let Some(clients) = self.dynamic_source_clients.get_mut(&event) else { return; };
        *clients &= !(client as u8);
        if *clients == 0 {
            self.dynamic_source_clients.remove(&event);
            self.sources.remove(&event);
        }
    }

    /// Immutable bank ownership for a background event decode. The encoded
    /// banks and decoded index are shared; a level change can retire its own
    /// references while an already-running read finishes safely.
    fn decode_snapshot(&self) -> Self {
        Self { banks: self.banks.clone(), attenuations: self.attenuations.clone(),
            library: self.library.clone(),
            stream_acquisition: self.stream_acquisition.clone(), stream_io: self.stream_io.clone(),
            source_jobs: self.source_jobs.clone(),
            media_generation: self.media_generation,
            ..Default::default() }
    }
    fn playback(&self, event: u32, resident_streams: bool,
        locators: &mut HashMap<(usize, u32), graph::StreamLocator>,
    ) -> Result<Vec<crate::assets::wwise::PlaybackCommand>, Box<dyn std::error::Error>> {
        let mut files = HashMap::<usize, std::fs::File>::new();
        let actions = self.library.event_playback_shared(event, |owner, media_id, stream_type| {
            let bank = self.banks.get(owner).ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData, "Wwise source bank ownership is absent"))?;
            let Some(media) = bank.package.media_file(media_id as u64, bank.entry.language_id) else { return Ok(None); };
            let size = usize::try_from(media.size).map_err(|_| std::io::Error::other("Wwise media too large"))?;
            if !resident_streams && stream_type != 0 {
                locators.entry((owner, media_id)).or_insert_with(|| graph::StreamLocator {
                    path: bank.path.clone(), offset: media.offset, size,
                    pool: self.stream_io.clone(),
                    source_jobs: self.source_jobs.clone(),
                });
            }
            if stream_type == 1 && crate::assets::wwise::stream_io::enabled() {
                // The bank owns the prefetch bytes. Publish the package extent
                // without reading the full file or duplicating that prefix.
                return Ok(Some(crate::assets::wwise::MediaBytes::stream_header(Vec::new(), size)));
            }
            let file = match files.entry(owner) {
                hashbrown::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                hashbrown::hash_map::Entry::Vacant(entry) => entry.insert(std::fs::File::open(&bank.path)?),
            };
            if stream_type == 2 {
                use std::io::{Seek, SeekFrom};
                let started = std::time::Instant::now();
                let header_only = std::env::var("DH2_DEBUG_EAGER_STREAM_ACQUISITION").as_deref() != Ok("1")
                    && !resident_streams;
                let acquired = if header_only {
                    file.seek(SeekFrom::Start(media.offset))?;
                    let header = crate::assets::wwise::read_stream_header(file, size)?;
                    if crate::assets::wwise::stream_format(&header, size)?.is_some() {
                        crate::assets::wwise::MediaBytes::stream_header(header, size)
                    } else { bank.package.read_resident_entry(file, media)? }
                } else { bank.package.read_resident_entry(file, media)? };
                let mut totals = self.stream_acquisition.lock().unwrap_or_else(|e| e.into_inner());
                totals.0 += 1;
                totals.1 += acquired.len() as u64;
                totals.2 += size as u64;
                totals.3 += started.elapsed();
                return Ok(Some(acquired));
            }
            bank.package.read_resident_entry(file, media).map(Some)
        })?.ok_or("event has no supported Wwise actions")?;
        // A second event may reuse the bank owner's immutable music graph.
        // Its source callback then does not run again: derive package extents
        // from the source's owning bank, without rereading encoded headers.
        if !resident_streams {
            for action in &actions {
                let crate::assets::wwise::PlaybackCommand::Play { node: crate::assets::wwise::PlaybackNode::Music(graph), .. } = action else { continue; };
                for (owner, media_id) in graph.media.iter().filter_map(|source| source.variant.streamed) {
                    if locators.contains_key(&(owner, media_id)) { continue; }
                    let bank = self.banks.get(owner).ok_or("music source bank ownership is absent")?;
                    let media = bank.package.media_file(u64::from(media_id), bank.entry.language_id).ok_or("music package extent is absent")?;
                    locators.insert((owner, media_id), graph::StreamLocator { path: bank.path.clone(), offset: media.offset,
                        size: usize::try_from(media.size)?, pool: self.stream_io.clone(), source_jobs: self.source_jobs.clone() });
                }
            }
        }
        Ok(actions)
    }
    fn prepare(&mut self, event: u32) -> Result<(), Box<dyn std::error::Error>> {
        if self.sources.contains_key(&event) { return Ok(()); }
        if self.preloaded {
            super::load_timing::late("sound event", &format!("{event:#010x}"));
        }
        let actions = self.decode_event(event)?;
        self.sources.insert(event, Arc::new(actions));
        Ok(())
    }

    /// The event's actions with every Play source decoded.
    fn decode_event(&self, event: u32) -> Result<Vec<SourceAction>, Box<dyn std::error::Error>> {
        use crate::assets::wwise::PlaybackCommand;
        let resident_streams = std::env::var_os("DH2_DEBUG_RESIDENT_STREAMS").is_some_and(|v| v == "1");
        let mut locators = HashMap::new();
        let graph_actions = {
            let _tag = crate::platform::allocation_tags::tag("sound bank lookup");
            self.playback(event, resident_streams, &mut locators)?
        };
        let _tag = crate::platform::allocation_tags::tag("sound graph");
        let mut actions = Vec::new();
        for action in graph_actions {
            // Each action is dispatched independently in 1410e5600 /
            // 1410e8260. A host decoder or unsupported actor must not
            // discard the event's other Play and Stop actions.
            let decoded = (|| -> Result<SourceAction, Box<dyn std::error::Error>> { match action {
            PlaybackCommand::Failed { action, target, reason } => Err(format!(
                "action {action:#010x}, target {target:#010x}: {reason}").into()),
            PlaybackCommand::Stop(action) => {
                let target_path = self.library.actor_hierarchy(action.target)?.map(Arc::<[u32]>::from);
                Ok(SourceAction::Stop { action, target_path })
            },
            PlaybackCommand::SetState(action) => {
                let target_path = if action.bus { None } else {
                    self.library.actor_hierarchy(action.target)?.map(Arc::<[u32]>::from)
                };
                Ok(SourceAction::SetState { action, target_path })
            },
            // 14113b5c0 sends every Play through the same source
            // and bus clock, including plain PCM and UI sounds.
            PlaybackCommand::Play { settings, node, .. } => Ok(SourceAction::Play { settings,
                definition: Arc::new(graph::Definition::decode(node, &self.attenuations,
                    &|source| locators.get(&source).cloned(), self.media_generation)?) }),
            } })();
            match decoded {
                Ok(action) => actions.push(action),
                Err(error) => {
                    if graph::media_generation() != self.media_generation {
                        return Err(std::io::Error::new(std::io::ErrorKind::Interrupted,
                            "sound bank generation retired").into());
                    }
                    warn!("Wwise event {event:#010x} action unavailable: {error}");
                },
            }
        }
        Ok(actions)
    }

}

/// The streamed media a playback reads; dropped with its voice entity.
#[derive(Component, Clone)]
struct StreamedMedia(Vec<graph::StreamHold>);

#[derive(Resource, Default)]
struct AudioMix {
    generation: u64,
    mixer: Option<crate::gameplay::sound::mix::Mixer>,
    bindings: Vec<(u32, &'static str)>,
    time_dilation: Option<u32>,
    physics_scale: Option<crate::assets::wwise::RtpcDeclaration>,
}

impl AudioMix {
    fn apply_settings(&mut self) {
        let Some(mixer) = self.mixer.as_mut() else { return };
        let shared = crate::platform::console::shared();
        let generation = shared.generation();
        let console = shared.read();
        for &(id, name) in &self.bindings {
            if let Some(value) = console.cvars.float(name) { mixer.parameters.insert(id, value); }
        }
        self.generation = generation;
    }
}

#[derive(Clone)]
struct Bank {
    path: PathBuf,
    package: Arc<crate::assets::wwise::Package>,
    entry: crate::assets::wwise::Entry,
}

#[derive(Clone)]
enum SourceAction {
    Stop { action: crate::assets::wwise::StopAction, target_path: bevy_backend::ActionTarget },
    SetState { action: crate::assets::wwise::StateAction, target_path: bevy_backend::ActionTarget },
    Play { settings: crate::assets::wwise::PlaySettings, definition: Arc<graph::Definition> },
}

#[derive(Component, Clone)]
struct AudioTarget {
    event: u32,
    emitter: Emitter,
    paused: bool,
    world_paused: bool,
}

pub struct SoundPlugin;

pub(super) struct AudioAssetsPlugin;

/// Host capability: source acquisition also runs without AudioPlugin. Such
/// Apps must not create an output mixer with no playback consumer.
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
enum PlaybackBackend { Absent, Hardware, Observation }
impl Plugin for AudioAssetsPlugin {
    fn build(&self, app: &mut App) {
        if app.is_plugin_added::<bevy::audio::AudioPlugin>() {
            // Bevy's playback systems require AudioOutput, GlobalVolume and
            // DefaultSpatialScale, supplied only by its hardware AudioPlugin.
            app.add_audio_source::<graph::ScheduledAudio>()
                .add_audio_source::<bevy_backend::OutputAudio>();
        } else {
            // Acquisition and the sole offline consumer need asset ownership,
            // not hardware playback/cleanup systems or a dummy output device.
            app.init_asset::<graph::ScheduledAudio>()
                .init_asset::<bevy_backend::OutputAudio>();
        }
    }
}

#[derive(Resource, Default)]
struct PendingSounds(Vec<PendingSound>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum ObjectRetirement { Queued, Submitted, Complete }
#[derive(Resource, Default)]
struct RetiringSounds(std::collections::HashMap<Emitter, ObjectRetirement>);

fn request_emitter(request: &Request) -> Emitter {
    match *request {
        Request::Post { emitter, .. } | Request::PostWithParameter { emitter, .. }
        | Request::PostAt { emitter, .. } | Request::Stop { emitter, .. } | Request::Retire { emitter } => emitter,
    }
}

fn request_object_retirement(world: &mut World, emitter: Emitter) -> Result<(), String> {
    if !world.contains_resource::<RetiringSounds>() || !world.contains_resource::<Messages<SoundEvent>>() {
        return Err("sound object retirement requires the active audio owners".into());
    }
    if world.resource::<RetiringSounds>().0.contains_key(&emitter) { return Ok(()); }
    world.resource_mut::<RetiringSounds>().0.insert(emitter, ObjectRetirement::Queued);
    world.write_message(SoundEvent(Request::Retire { emitter }));
    Ok(())
}

struct PendingSound {
    request: Request,
    message_id: Option<usize>,
    // A retry must not release its already completed reads while another
    // source is pending; the source's own resident lookup is deliberately weak.
    media: Vec<graph::StreamHold>,
    watched: Option<u64>,
    // c4550/c6030 admit the playing ID and object cookie before source preparation.
    ticket: Option<crate::gameplay::sound::playing::Ticket>,
}

fn cancel_pending_posts(pending: &mut Vec<PendingSound>, completions: &mut SoundCompletions,
    emitter: Emitter, event: Option<u32>) {
    pending.retain(|queued| {
        let id = match queued.request {
            Request::Post { event, .. } | Request::PostWithParameter { event, .. }
                | Request::PostAt { event, .. } => event,
            _ => return true,
        };
        if request_emitter(&queued.request) != emitter || event.is_some_and(|event| event != id) { return true; }
        completions.failed_watch(id, emitter, queued.watched);
        false
    });
}

/// Retire one actual component object before its generation can be recycled.
/// Decode jobs remain shared with their other admitted clients.
pub(super) fn retire_component_object(world: &mut World, emitter: Emitter) -> Result<bool, String> {
    // 708340/709f70 allocate joint objects below the same actual component.
    // Their output acknowledgements must precede releasing its generation.
    let mut joints = world.resource::<bevy_backend::Engine>().level_emitters();
    joints.extend(world.resource::<PendingSounds>().0.iter().map(|queued| request_emitter(&queued.request)));
    joints.extend(world.resource::<RetiringSounds>().0.keys().copied());
    joints.retain(|joint| *joint != emitter && joint.player_owner() == emitter);
    for joint in &joints { request_object_retirement(world, *joint)?; }
    request_object_retirement(world, emitter)?;
    let retiring = &world.resource::<RetiringSounds>().0;
    Ok(retiring.get(&emitter) == Some(&ObjectRetirement::Complete)
        && joints.iter().all(|joint| retiring.get(joint) == Some(&ObjectRetirement::Complete)))
}

/// Called after the component pool actually releases the acknowledged generation.
pub(super) fn forget_retired_component_object(world: &mut World, emitter: Emitter) -> Result<(), String> {
    let mut retiring = world.resource_mut::<RetiringSounds>();
    if retiring.0.get(&emitter) != Some(&ObjectRetirement::Complete) {
        return Err("component sound object has not acknowledged retirement".into());
    }
    retiring.0.remove(&emitter);
    retiring.0.retain(|joint, _| joint.player_owner() != emitter);
    drop(retiring);
    if let Some(mut state) = world.get_resource_mut::<super::player::PlayerState>() {
        if let Some(player) = &mut state.player { player.forget_animation_sound_owner(emitter); }
    }
    Ok(())
}

/// Advance the existing consumer after exclusive callers restore scene owners.
/// The output callback progresses independently; never spin waiting for it.
pub(in crate::app) fn has_output(world: &World) -> bool { world.resource::<bevy_backend::Engine>().has_output() }

/// Progress accepted FX sound cookies while their source actors, scene and
/// entity-event owner still exist. The caller must drain EntityEvents before
/// releasing actor IDs. This does not close level/audio admission or banks.
pub(in crate::app) fn poll_accepted_effects(world: &mut World) -> Result<(), String> {
    world.run_system_cached(effects::update).map_err(|error| error.to_string())?;
    stimulus::update(world)?;
    poll_object_retirements(world)?;
    if let Some(error) = world.resource::<effects::EffectAudio>().last_error.as_ref() {
        return Err(format!("accepted FX sound delivery failed: {error}"));
    }
    Ok(())
}

pub(in crate::app) fn poll_object_retirements(world: &mut World) -> Result<(), String> {
    world.run_system_cached(bevy_backend::retire).map_err(|error| error.to_string())?;
    world.run_system_cached(play_events).map_err(|error| error.to_string())?;
    world.run_system_cached(reap_retiring_objects).map_err(|error| error.to_string())?;
    if let Some(error) = world.resource::<RetiringSounds>().0.iter().find_map(|(&emitter, &state)|
        (state != ObjectRetirement::Complete).then(|| world.resource::<bevy_backend::Engine>().object_retirement_pending(emitter).err()).flatten()) {
        return Err(error);
    }
    Ok(())
}

/// Keep the exact sound object and its histories until its accepted posts,
/// output voices and event tickets have all retired.
fn reap_retiring_objects(engine: Res<bevy_backend::Engine>, mut retiring: ResMut<RetiringSounds>,
    mut registry: ResMut<WwiseRegistry>, mut completions: ResMut<SoundCompletions>,
    level: Option<ResMut<level::LevelAudio>>, mut scene: ResMut<spatial::Scene>,
    sounds: Option<Res<crate::rendering::bevy_backend::world_sounds::WorldSounds>>) {
    if retiring.0.is_empty() { return; }
    let mut level = level;
    for (&emitter, state) in &mut retiring.0 {
        if *state != ObjectRetirement::Submitted { continue; }
        if !matches!(engine.object_retirement_pending(emitter), Ok(false)) { continue; }
        if completions.playing.references_object(emitter) { continue; }
        if sounds.as_ref().is_some_and(|sounds| sounds.lock().playing.references_object(emitter)) { continue; }
        let events: std::collections::HashSet<_> = completions.waiting.keys().chain(completions.active.keys())
            .filter_map(|&(event, object)| (object == emitter).then_some(event)).collect();
        completions.waiting.retain(|(_, object), _| *object != emitter);
        completions.active.retain(|(_, object), _| *object != emitter);
        completions.completed.retain(|(_, object), _| *object != emitter);
        for event in events { completions.retire_if_unused(event); }
        registry.scheduled_selections.lock().unwrap_or_else(|error| error.into_inner())
            .retain(|(_, object), _| *object != Some(emitter));
        if let Some(level) = level.as_deref_mut() { level.release_object(emitter); }
        scene.release_object(emitter);
        if let Some(sounds) = &sounds { sounds.lock().objects.remove(&emitter); }
        *state = ObjectRetirement::Complete;
    }
    // Actual component/equipment generations retain acknowledgement until
    // their owner releases them; a later request must not restart retirement.
    retiring.0.retain(|emitter, state| *state != ObjectRetirement::Complete
        || matches!(emitter, Emitter::Component { .. } | Emitter::Equipment { .. } | Emitter::PlayerJoint { .. }));
}

pub(in crate::app) fn begin_level_retirement(world: &mut World) -> Result<(), String> {
    reservations::close(world);
    audiographs::stop(world)?;
    music::stop(world)?;
    world.run_system_cached(effects::update).map_err(|error| error.to_string())?;
    stimulus::update(world)?;
    world.run_system_cached(play_events).map_err(|error| error.to_string())?;
    let mut emitters = world.resource::<bevy_backend::Engine>().level_emitters();
    if let Some(level) = world.get_resource::<level::LevelAudio>() { emitters.extend(level.emitters()); }
    emitters.extend(world.resource::<PendingSounds>().0.iter().map(|queued| request_emitter(&queued.request)));
    emitters.extend(world.query::<&AudioTarget>().iter(world).map(|target| target.emitter));
    emitters.remove(&Emitter::Menu);
    if let Some(sounds) = world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>() {
        let sounds = sounds.lock();
        for ticket in &sounds.queued { emitters.remove(&ticket.event().object); }
        for emitter in &sounds.released { emitters.remove(emitter); }
    }
    for emitter in emitters { request_object_retirement(world, emitter)?; }
    Ok(())
}

pub(in crate::app) fn poll_level_retirement(world: &mut World) -> Result<bool, String> {
    world.run_system_cached(effects::update).map_err(|error| error.to_string())?;
    stimulus::update(world)?;
    poll_object_retirements(world)?;
    let pending = world.resource::<PendingSounds>().0.iter().any(|queued| request_emitter(&queued.request) != Emitter::Menu);
    let retiring = world.resource::<RetiringSounds>().0.iter().any(|(&emitter, &state)| emitter != Emitter::Menu && state != ObjectRetirement::Complete);
    let fx_pending = world.get_resource::<crate::rendering::bevy_backend::world_sounds::WorldSounds>()
        .is_some_and(|sounds| { let sounds = sounds.lock(); !sounds.queued.is_empty() || !sounds.released.is_empty() });
    if fx_pending {
        if let Some(error) = world.resource::<effects::EffectAudio>().last_error.as_ref() {
            return Err(format!("world FX sound retirement failed: {error}"));
        }
    }
    let reserved = reservations::drain(world)?;
    Ok(audiographs::drained(world)? && reserved && !pending && !retiring && !fx_pending)
}

pub(super) fn begin_level(world: &mut World) {
    world.resource::<bevy_backend::Engine>().release_level();
    world.resource_mut::<RetiringSounds>().0.retain(|emitter, _| *emitter == Emitter::Menu);
    let mut registry = world.resource_mut::<WwiseRegistry>();
    registry.sources.clear();
    registry.dynamic_source_clients.clear();
    let keep = registry.shared_banks;
    Arc::make_mut(&mut registry.library).truncate(keep);
    Arc::make_mut(&mut registry.banks).truncate(keep);
    let actors: std::collections::HashSet<_> = registry.library.actor_ids().collect();
    // Global history belongs to its retained actor; object-scoped history
    // belongs to the outgoing objects. Keep the same Arc for live UI voices.
    registry.scheduled_selections.lock().unwrap_or_else(|e| e.into_inner())
        .retain(|(actor, emitter), _| actors.contains(actor) && emitter.is_none_or(|e| e == Emitter::Menu));
    let sound_ids = registry.library.sound_ids().collect();
    let shared_attenuations = registry.shared_attenuations.clone();
    Arc::make_mut(&mut registry.attenuations).retain(|id, _| shared_attenuations.contains(id));
    let packages: std::collections::HashSet<_> = registry.banks.iter().map(|bank| bank.path.clone()).collect();
    registry.packages.retain(|path, _| packages.contains(path));
    registry.stream_io.retain_packages(&packages);
    registry.preloaded = false;
    registry.level_prepared = false;
    drop(registry);
    world.resource::<bevy_backend::Engine>().retain_nodes(&actors);
    world.insert_resource(SoundDecodeQueue::default());
    world.insert_resource(reservations::EventReservations::default());
    world.insert_resource(PendingSounds::default());
    world.resource_mut::<WwiseRegistry>().media_generation = graph::retain_media(&sound_ids);
    if let Some(mut completions) = world.get_resource_mut::<SoundCompletions>() {
        let waiting = std::mem::take(&mut completions.waiting);
        for (key, _) in waiting { completions.completed.insert(key, false); }
        completions.decode_failures.clear();
        // Active menu voices can outlive a level, but their old acquisition
        // keys must never evict a new level's event with the same numeric ID.
        completions.dynamic_sources.clear();
        completions.retired_sources.clear();
    }
    world.insert_resource(spatial::Scene::default());
    world.insert_resource(effects::EffectAudio::default());
    if let Some(contacts) = world.get_resource::<contacts::ContactSounds>() {
        contacts.release_level_history();
    }
    world.insert_resource(contacts::ContactSounds::default());
    world.resource_mut::<Messages<SoundEvent>>().clear();
    world.resource_mut::<Messages<ContactSound>>().clear();
}

/// Acquisition owners without gameplay schedules or an audio output device.
/// The client and windowless resource observations use the same initialization.
pub(super) fn initialize_resources(app: &mut App) {
    let hardware = app.is_plugin_added::<bevy::audio::AudioPlugin>();
    let offline = app.world().contains_resource::<observation::Observation>();
    assert!(!(hardware && offline), "offline audio requires hardware AudioPlugin disabled");
    app.insert_resource(if offline { PlaybackBackend::Observation } else if hardware { PlaybackBackend::Hardware } else { PlaybackBackend::Absent });
    if !app.world().contains_resource::<Assets<graph::ScheduledAudio>>() {
        app.init_asset::<graph::ScheduledAudio>();
    }
    if !app.world().contains_resource::<Assets<bevy_backend::OutputAudio>>() {
        app.init_asset::<bevy_backend::OutputAudio>();
    }
    crate::loading::gate(LISTED_SOUNDS_GATE);
    crate::loading::gate(LEVEL_SOUNDS_GATE);
    app.add_message::<SoundEvent>()
        .add_message::<music::MusicEvent>()
        .add_message::<ContactSound>()
        .init_resource::<contacts::ContactSounds>()
        .init_resource::<objects::EffectObjects>()
        .init_resource::<effects::EffectAudio>()
        .init_resource::<WwiseRegistry>()
        .init_resource::<acquisition::SourceJobs>()
        .init_resource::<SoundEventCursor>()
        .init_resource::<RetiringSounds>()
        .init_resource::<PendingSounds>()
        .init_resource::<SoundCompletions>()
        .init_resource::<audiographs::Audiographs>()
        .init_resource::<SoundDecodeQueue>()
        .init_resource::<reservations::EventReservations>()
        .init_resource::<AudioMix>()
        .init_resource::<bevy_backend::Engine>()
        .init_resource::<spatial::Scene>();
    let source_jobs = Arc::downgrade(&app.world().resource::<acquisition::SourceJobs>().0);
    let mut registry = app.world_mut().resource_mut::<WwiseRegistry>();
    registry.source_jobs = source_jobs;
    registry.media_generation = graph::retain_media(&Default::default());
    drop(registry);
    observation::attach(app.world_mut());
}

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AudioAssetsPlugin>() { app.add_plugins(AudioAssetsPlugin); }
        initialize_resources(app);
        app.add_systems(Startup, (contacts::initialize, index_banks, preload_level_events).chain())
            .add_systems(Update, contacts::update.after(super::player::update_player))
            .add_systems(PostUpdate, (bevy_backend::retire, spatial::update, reservations::advance, prepare_level_sounds, level::update, music::update.run_if(super::loading_screen::is_playing), audiographs::update, effects::update, stimulus::scheduled, update_playing_volume, play_events, reap_retiring_objects)
                .chain().after(super::model_motion::publish_model_poses));
    }
}

/// The packages and bank ids the original loads for the current map.
struct LevelBanks {
    packages: Vec<String>, banks: std::collections::HashSet<u32>,
    shared_packages: std::collections::HashSet<String>, shared_banks: std::collections::HashSet<u32>,
}

/// Wwise names its ids by FNV-1 of the lowercase name (verified on the
/// installed main-common-sfx.pck: common_default).
fn wwise_id(name: &str) -> u32 {
    name.bytes().fold(0x811c_9dc5u32, |hash, byte| hash.wrapping_mul(0x0100_0193) ^ u32::from(byte.to_ascii_lowercase()))
}

/// wwisedescription.txt ("cooked level descriptions", 1400c0cb0): "V1", an
/// entry count, then per entry its name, package prefixes and bank names.
fn level_banks(base: &std::path::Path, language: &str, map: &str) -> Result<LevelBanks, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(base.join("wwisedescription.txt"))?;
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("V1") { return Err("wwisedescription.txt is not V1".into()); }
    let count: usize = lines.next().ok_or("wwisedescription.txt has no entry count")?.trim().parse()?;
    let mut entries = HashMap::new();
    for _ in 0..count {
        let name = lines.next().ok_or("wwisedescription.txt entry truncated")?.trim();
        let packages = lines.next().unwrap_or("").split_whitespace().collect::<Vec<_>>();
        let banks = lines.next().unwrap_or("").split_whitespace().collect::<Vec<_>>();
        entries.insert(name, (packages, banks));
    }
    let map = map.trim_start_matches("maps/").trim_end_matches(".entities").trim_end_matches(".map").to_owned();
    let mut selection = LevelBanks { packages: vec!["main-init.pck".to_owned()], banks: Default::default(),
        shared_packages: ["main-init.pck".to_owned()].into(), shared_banks: Default::default() };
    for name in ["common", "init", map.as_str()] {
        let (packages, banks) = entries.get(name).ok_or_else(|| format!("wwisedescription.txt has no entry {name}"))?;
        for prefix in packages {
            selection.packages.push(format!("{prefix}{language}.pck"));
            selection.packages.push(format!("{prefix}sfx.pck"));
        }
        selection.banks.extend(banks.iter().map(|bank| wwise_id(bank)));
        if name == "common" || name == "init" {
            for prefix in packages {
                selection.shared_packages.insert(format!("{prefix}{language}.pck"));
                selection.shared_packages.insert(format!("{prefix}sfx.pck"));
            }
            selection.shared_banks.extend(banks.iter().map(|bank| wwise_id(bank)));
        }
    }
    selection.packages.sort();
    selection.packages.dedup();
    Ok(selection)
}

fn index_banks(mut registry: ResMut<WwiseRegistry>, mut mix: ResMut<AudioMix>,
    mut engine: ResMut<bevy_backend::Engine>, mut outputs: ResMut<Assets<bevy_backend::OutputAudio>>, commands: Commands,
    source: Option<Res<super::level_prepare::LevelSourceReads>>, backend: Res<PlaybackBackend>) {
    if registry.preloaded { return; }
    let Some(source) = source else { return };
    let (catalog, map) = source.acquisition_source();
    let mut commands = super::setup_commands::SetupCommands::Direct(commands);
    prepare_banks(&mut registry, &mut mix, &mut engine, &mut outputs, &mut commands, catalog, map, *backend);
}

fn prepare_banks(registry: &mut WwiseRegistry, mix: &mut AudioMix,
    engine: &mut bevy_backend::Engine,
    outputs: &mut dyn crate::rendering::bevy_backend::asset_preparation::AssetWrite<bevy_backend::OutputAudio>,
    commands: &mut super::setup_commands::SetupCommands, catalog: &crate::archive::Catalog, map: &str, playback: PlaybackBackend) {
    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
        let base = catalog.base();
        let audio_language = crate::assets::lang::read_audio_language(&base)?;
        let package_language = crate::assets::wwise::language_name(audio_language).ok_or("unsupported audio package language")?;
        // 1400bd510 (LoadSharedCookedGamePackages): the "common", "init" and
        // current-map entries of wwisedescription.txt name the packages
        // (prefix + language / "sfx.pck") and banks to load, plus main-init.pck.
        let selection = level_banks(base, package_language, map)?;
        let paths = selection.packages.iter().map(|name| base.join("pck").join(name))
            .filter(|path| path.is_file()).collect::<Vec<_>>();

        let mut output_mix = None;
        let before = registry.banks.len();
        let trace_backing = std::env::var_os("DH2_DEBUG_WWISE_BANK_BACKING").is_some_and(|v| v == "1");
        let copy_backing = std::env::var_os("DH2_DEBUG_COPY_WWISE_BANK_BACKING").is_some_and(|v| v == "1");
        for shared in [true, false] {
        for path in &paths {
            let package = if let Some(package) = registry.packages.get(path) {
                package.clone()
            } else {
                let Ok(package) = crate::assets::wwise::Package::open(path) else { continue };
                let package = Arc::new(package);
                registry.stream_io.allow_package(path);
                registry.packages.insert(path.clone(), package.clone());
                package
            };
            let language = package.language_id(package_language);
            let init = path.file_name().is_some_and(|name| name.eq_ignore_ascii_case("main-init.pck"));
            for entry in package.banks.iter().copied().filter(|entry| entry.language_id == 0 || Some(entry.language_id) == language)
                .filter(|entry| init || u32::try_from(entry.id).is_ok_and(|id| selection.banks.contains(&id))) {
                let shared_entry = init || (path.file_name().and_then(|name| name.to_str()).is_some_and(|name| selection.shared_packages.contains(name))
                    && u32::try_from(entry.id).is_ok_and(|id| selection.shared_banks.contains(&id)));
                if shared_entry != shared { continue; }
                // 1400bd510 deduplicates shared packages; PlaybackBanks keeps
                // its verified first-owner HIRC identity. Bank indices remain
                // aligned with the media locator table across map loads.
                if registry.banks.iter().any(|bank| bank.path == *path
                    && bank.entry.id == entry.id && bank.entry.language_id == entry.language_id) { continue; }
                let indexed = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let mut file = std::fs::File::open(&path)?;
                    let start = trace_backing.then(crate::loading::thread_cycles).flatten();
                    let bytes: crate::assets::wwise::MediaBytes = if copy_backing {
                        package.read_entry(&mut file, entry)?.into()
                    } else {
                        package.read_resident_entry(&mut file, entry)?
                    };
                    if trace_backing {
                        let cycles = start.zip(crate::loading::thread_cycles())
                            .map_or(0, |(start, end)| end.saturating_sub(start));
                        let mut crc = flate2::Crc::new();
                        crc.update(&bytes);
                        let copied_bytes = if copy_backing { bytes.len() } else { 0 };
                        eprintln!(
                            "WWISE_BANK_BACKING package={} id={} language={} bytes={} copied_bytes={copied_bytes} cycles={cycles} crc={:08x}",
                            path.display(), entry.id, entry.language_id, bytes.len(), crc.sum(),
                        );
                    }
                    Ok((crate::assets::wwise::bank_mix(&bytes)?, crate::assets::wwise::bank_attenuations(&bytes)?, bytes))
                })();
                match indexed {
                    Ok((bank_mix, bank_attenuations, bytes)) => {
                        for attenuation in bank_attenuations { Arc::make_mut(&mut registry.attenuations).entry(attenuation.id).or_insert_with(|| Arc::new(attenuation)); }
                        if bank_mix.is_some() { output_mix = bank_mix; }
                        Arc::make_mut(&mut registry.library).insert_resident(bytes)?;
                        Arc::make_mut(&mut registry.banks).push(Bank { path: path.clone(), package: package.clone(), entry });
                    }
                    Err(error) => debug!("Skipping Wwise bank in {}: {error}", path.display()),
                }
            }
        }
        if shared {
            registry.shared_banks = registry.banks.len();
            registry.shared_attenuations = registry.attenuations.keys().copied().collect();
        }
        }
        info!("Wwise map acquisition: {} banks acquired, {before} reused, {} packages resident",
            registry.banks.len() - before, registry.packages.len());
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("LOAD shared sound banks: count={} first_owners={:?}", registry.shared_banks,
                registry.library.resident_allocations().take(registry.shared_banks).take(4).collect::<Vec<_>>());
        }
        if mix.mixer.is_some() { return Ok(None); }
        let controls = crate::assets::wwise::engine_controls::EngineControls::acquire(catalog)?;
        let bindings = controls.volumes.into_iter().zip([
            "s_MasterBus_Gain", "s_SFX_Gain", "s_Voices_Gain", "s_Music_Gain", "s_Video_Gain",
        ]).collect::<Vec<_>>();
        Ok(Some((output_mix.ok_or("Wwise initialization mix is absent")?, bindings,
            controls.time_dilation, controls.physics_scale)))
    })();
    match result {
        Ok(Some((output_mix, bindings, time_dilation, physics_scale))) => {
            registry.states = Arc::new(std::sync::Mutex::new(crate::gameplay::sound::states::States::new(output_mix.state_groups.iter().cloned())));
            mix.time_dilation = Some(time_dilation);
            mix.physics_scale = Some(physics_scale);
            info!("Indexed {} installed Wwise banks, {} event IDs and {} global actor IDs", registry.banks.len(), registry.library.event_count(), registry.library.actor_count());
            mix.mixer = Some(output_mix.into());
            mix.bindings = bindings;
            mix.apply_settings();
            if let Some(mixer) = mix.mixer.as_ref().filter(|_| playback != PlaybackBackend::Absent) {
                match bevy_backend::Engine::new(mixer, registry.states.clone(), catalog.base()) {
                    Ok((initialized, output)) => {
                        if let Some(mixer) = &mut mix.mixer { mixer.activity = initialized.activity(); }
                        *engine = initialized;
                        if playback == PlaybackBackend::Observation {
                            commands.queue(move |world: &mut World| observation::install_output(world, output));
                        } else {
                            commands.spawn((AudioPlayer(outputs.add(output)), PlaybackSettings::ONCE, super::level_lifetime::SessionEntity));
                        }
                        info!("Shared stereo audio output initialized with installed bus effects");
                    },
                    Err(error) => error!("Shared Wwise audio output unavailable: {error}"),
                }
            }
        }
        Ok(None) => {},
        Err(error) => warn!("Installed Wwise banks unavailable: {error}"),
    }
}


/// The map load (`FUN_1400661e0`) loads the `soundevent` decls of the
/// `init`, `common` and map lists before play, with their Wwise media; Vorbis
/// is then decoded while it plays. Every listed event's media is read and
/// prepared here, on all cores, instead of on the main thread at its first
/// post.
fn preload_level_events(mut registry: ResMut<WwiseRegistry>, contacts: Option<Res<contacts::ContactSounds>>,
    source: Option<Res<super::level_prepare::LevelSourceReads>>) {
    if registry.preloaded { return; }
    let Some(source) = source else { return };
    let (catalog, map) = source.acquisition_source();
    prepare_listed_events(&mut registry, None, contacts.as_deref().and_then(|c| c.declarations()), catalog, map);
}
fn prepare_listed_events(registry: &mut WwiseRegistry, acquisition_pool: Option<&bevy::tasks::TaskPool>,
    declarations: Option<&declaration_texts::DeclarationTexts>, catalog: &crate::archive::Catalog, map: &str) {
    let started = std::time::Instant::now();
    let result = (|| -> Result<Vec<u32>, String> {
        let base = catalog.base();
        let map = map.trim_start_matches("maps/").trim_end_matches(".entities").trim_end_matches(".map");
        let language = crate::assets::lang::read_text_language(&base)?;
        let list = crate::assets::map_resources::acquire_level_list(&base, &catalog, map, language)?;
        let mut events = Vec::new();
        let mut hits = 0usize;
        let mut reads = 0usize;
        let mut decoded_bytes = 0usize;
        let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        // Mutable overlays can change between the two acquisition phases.
        let reuse = !catalog.has_overlays() && std::env::var("DH2_DEBUG_SEPARATE_CONTACT_TEXTS").as_deref() != Ok("1");
        for resource in list.iter().filter(|r| r.kind == "soundevent") {
            let Some(name) = resource.path.strip_prefix("generated/decls/soundevent/").and_then(|p| p.strip_suffix(".decl")) else { continue };
            let asset = crate::assets::fx::SoundEventAsset { delay: [0.0; 2], event: Some(name.to_owned()) };
            match asset.resolve_with(&catalog, |path| {
                if let Some(value) = declarations.filter(|_| reuse).and_then(|texts| texts.get(path)) {
                    if trace { hits += 1; }
                    return Ok(Some(value.to_owned()));
                }
                if trace { reads += 1; }
                let bytes = catalog.read(path).map_err(|error| format!("{path}: {error}"))?;
                if trace { decoded_bytes += bytes.as_ref().map_or(0, Vec::len); }
                Ok(bytes.map(crate::entities::owned_declaration_text))
            }) {
                Ok(Some(declaration)) => events.push(declaration.id),
                Ok(None) => {}
                Err(error) => debug!(target: "dh2::loading", "Listed sound event {name}: {error}"),
            }
        }
        events.sort_unstable();
        events.dedup();
        if trace {
            let mut crc = flate2::Crc::new();
            for event in &events { crc.update(&event.to_le_bytes()); }
            eprintln!("MEMORY listed sound declaration sources: map={map} cache_hits={hits} reads={reads} decoded_bytes={decoded_bytes} events={} signature={:08x}", events.len(), crc.sum());
        }
        Ok(events)
    })();
    let events = match result {
        Ok(events) => events,
        Err(error) => {
            crate::loading::complete(LISTED_SOUNDS_GATE);
            return warn!("Level sound events not preloaded: {error}");
        }
    };
    let count = prepare_all(registry, &events, acquisition_pool);
    registry.preloaded = true;
    crate::loading::complete(LISTED_SOUNDS_GATE);
    super::load_timing::mark_detail("sound events", &format!("sound events: {count} of {} listed prepared in {:.1}s", events.len(), started.elapsed().as_secs_f64()));
}

/// Level loading gates of this plugin.
const LISTED_SOUNDS_GATE: &str = "listed sound events";
const LEVEL_SOUNDS_GATE: &str = "level emitter and world FX sound events";

/// Prepare `events` not prepared yet on all cores. Returns how many were.
fn prepare_all(registry: &mut WwiseRegistry, events: &[u32], acquisition_pool: Option<&bevy::tasks::TaskPool>) -> usize {
    let prepare_started = std::time::Instant::now();
    let events: Vec<u32> = events.iter().copied().filter(|e| !registry.sources.contains_key(e)).collect();
    if events.is_empty() { return 0; }
    let shared = &*registry;
    let pool = acquisition_pool.unwrap_or_else(|| bevy::tasks::ComputeTaskPool::get());
    let threads = pool.thread_num().max(1);
    let prepared: Vec<(u32, Vec<SourceAction>)> = pool.scope(|scope| {
        for chunk in events.chunks(events.len().div_ceil(threads).max(1)) { scope.spawn(async move {
            let _tag = crate::platform::allocation_tags::tag("sound preparation");
            chunk.iter().filter_map(|&event| match shared.decode_event(event) {
                Ok(actions) => Some((event, actions)),
                Err(error) => {
                    debug!(target: "dh2::loading", "Level Wwise event {event:#010x}: {error}");
                    None
                }
            }).collect::<Vec<_>>()
        }); }
    }).into_iter().flatten().collect();
    let count = prepared.len();
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|v| v == "1") {
        println!("PERF sound preparation: events={count} wall_ms={:.3}", prepare_started.elapsed().as_secs_f64() * 1000.0);
        let streams = registry.stream_acquisition.lock().unwrap_or_else(|e| e.into_inner());
        println!("MEMORY stream acquisition: requests={} read_bytes={} source_bytes={} read_ms={:.3}",
            streams.0, streams.1, streams.2, streams.3.as_secs_f64() * 1000.0);
        drop(streams);
        let banks = registry.library.resident_allocations().collect::<HashMap<_, _>>();
        let sources = graph::resident_allocations();
        let mut owners = banks.clone();
        owners.extend(sources.iter().copied());
        println!("MEMORY sound encoded residency: bank_bytes={} source_allocation_bytes={} unique_bytes={} source_owners={} shared_bank_owners={}",
            banks.values().sum::<usize>(), sources.iter().copied().collect::<HashMap<_, _>>().values().sum::<usize>(),
            owners.values().sum::<usize>(), sources.iter().copied().collect::<HashMap<_, _>>().len(),
            sources.iter().filter(|(id, _)| banks.contains_key(id)).count());
        graph::capture_media_signatures();
        registry.library.report_media_sections();
        crate::assets::wwise::Package::report_media_lookup();
        crate::assets::wwise::report_bank_index_lookup();
        crate::assets::wwise::capture_property_chains();
        for (representation, (n, installed, views)) in graph::resident_media_stats() {
            eprintln!("LOAD sound media representation {representation}: {n} current sources, {} MB installed, {} MB logical views", installed >> 20, views >> 20);
        }
        let streamed = crate::assets::wwise::STREAMED_MEDIA.lock().unwrap_or_else(|e| e.into_inner());
        let mut by_type = std::collections::BTreeMap::<u8, (usize, usize, usize, usize)>::new();
        for &(kind, size, prefix) in streamed.values() {
            let entry = by_type.entry(kind).or_default();
            entry.0 += 1; entry.1 += size;
            if let Some(prefix) = prefix { entry.2 += 1; entry.3 += prefix; }
        }
        for (kind, (count, bytes, prefixed, prefix_bytes)) in by_type {
            eprintln!("LOAD sound streamed type {kind}: {count} media, {} MB; {prefixed} with a DIDX entry, {} KB in them", bytes >> 20, prefix_bytes >> 10);
        }
    }
    crate::assets::wwise::report_stream_diagnostics();
    registry.sources.extend(prepared.into_iter().map(|(event, actions)| (event, Arc::new(actions))));
    count
}

/// The events of the level's audio components and world FX, which the
/// original loads as dependencies of the map's entities and FX decls during
/// the map load, once both exist and before they are first evaluated.
fn prepare_level_sounds(physics: Option<ResMut<super::physics_scene::PhysicsScene>>,
    sources: Option<Res<crate::rendering::bevy_backend::world_effects::WorldEffectSources>>,
    music: Option<Res<music::Music>>,
    environments: Option<Res<super::state::EnvironmentVolumes>>,
    mut registry: ResMut<WwiseRegistry>)
{
    if registry.level_prepared { return; }
    let (Some(mut physics), Some(sources)) = (physics, sources) else { return };
    let mut events: Vec<u32> = sources.0.sound_events().collect();
    if let Some(music) = music { events.extend(music.events()); }
    if let Some(environments) = environments { events.extend(environments.0.iter().filter_map(|volume| volume.ambiance.as_ref().map(|event| event.id))); }
    let visited = physics.visit_audio(|_, binding, _| {
        events.extend(binding.events.iter().flatten().map(|event| event.id));
        Ok(())
    });
    if let Err(error) = visited { warn!("Level audio events not collected: {error}"); }
    events.sort_unstable();
    events.dedup();
    let started = std::time::Instant::now();
    let count = prepare_all(&mut registry, &events, None);
    registry.level_prepared = true;
    crate::loading::complete(LEVEL_SOUNDS_GATE);
    super::load_timing::mark_detail("level sounds", &format!("level sounds: {count} more of {} emitter and FX events prepared in {:.1}s", events.len(), started.elapsed().as_secs_f64()));
}

fn play_events(
    mut commands: Commands,
    (physics, live): (Option<Res<super::physics_scene::PhysicsScene>>,
        Option<Res<crate::gameplay::entity_lifetime::LiveEntities>>),
    events: Res<Messages<SoundEvent>>,
    mut event_cursor: ResMut<SoundEventCursor>,
    mut registry: ResMut<WwiseRegistry>,
    mut scheduled_sources: ResMut<Assets<graph::ScheduledAudio>>,
    mix: Res<AudioMix>,
    scene: Res<spatial::Scene>,
    engine: Res<bevy_backend::Engine>,
    mut pending: ResMut<PendingSounds>,
    mut completions: ResMut<SoundCompletions>,
    mut decodes: ResMut<SoundDecodeQueue>,
    mut retiring: ResMut<RetiringSounds>,
    observer: Option<Res<observation::Observation>>,
    (loading, exit): (Option<Res<super::loading_screen::Acquisition>>, Option<Res<super::loading_screen::ExitState>>),
) {
    completions.pump();
    for event in std::mem::take(&mut completions.retired_sources) { registry.release_dynamic_source(event, DecodeClient::Watched); }
    let wanted_decodes: std::collections::HashSet<_> = completions.waiting.keys().map(|(event, _)| *event).collect();
    decodes.retain_client(DecodeClient::Watched, &wanted_decodes);
    decodes.poll();
    for event in wanted_decodes {
        let Some(result) = decodes.result(event) else { continue };
        match result {
            Ok(actions) => {
                registry.publish_dynamic_source(event, actions, DecodeClient::Watched);
                if registry.claim_dynamic_source(event, DecodeClient::Watched) {
                    completions.dynamic_sources.insert(event);
                }
            }
            Err(error) => { completions.decode_failures.insert(event, error); }
        }
        decodes.release(event, DecodeClient::Watched);
    }
    let Some(mixer) = &mix.mixer else {
        for queued in std::mem::take(&mut pending.0) {
            if let Request::Retire { emitter } = queued.request {
                match engine.release_object(emitter) {
                    Ok(()) => { retiring.0.insert(emitter, ObjectRetirement::Submitted); },
                    Err(error) => { warn!("Sound object retirement failed: {error}"); pending.0.push(queued); },
                }
                continue;
            }
            match queued.request {
                Request::Post { event, emitter } | Request::PostWithParameter { event, emitter, .. }
                    | Request::PostAt { event, emitter, .. } if queued.watched.is_some() => completions.failed_watch(event, emitter, queued.watched),
                _ => {},
            }
        }
        for message in event_cursor.0.read(&events) {
            match message.0 {
                Request::Post { event, emitter } | Request::PostWithParameter { event, emitter, .. }
                    | Request::PostAt { event, emitter, .. } => completions.failed(event, emitter),
                Request::Stop { .. } => {},
                Request::Retire { emitter } => {
                    match engine.release_object(emitter) {
                        Ok(()) => { retiring.0.insert(emitter, ObjectRetirement::Submitted); },
                        Err(error) => {
                            warn!("Sound object retirement failed: {error}");
                            pending.0.push(PendingSound { request: message.0, media: Vec::new(), watched: None, ticket: None, message_id: None });
                        },
                    }
                },
            }
        }
        return;
    };
    // Posts retain FIFO order while preparation waits. Stop/Retire bypass a
    // cold post to cancel its real preparation owner, as they do queued actions.
    let requests: Vec<PendingSound> = std::mem::take(&mut pending.0).into_iter()
        .chain(event_cursor.0.read_with_id(&events).map(|(message, message_id)| {
            let watched = match message.0 {
                Request::Post { event, emitter } | Request::PostWithParameter { event, emitter, .. }
                    | Request::PostAt { event, emitter, .. } => completions.waiting.get(&(event, emitter)).copied(),
                Request::Stop { .. } | Request::Retire { .. } => None,
            };
            PendingSound { request: message.0, message_id: Some(message_id.id), media: Vec::new(), watched, ticket: None }
        })).collect();
    let mut waiting = Vec::new();
    for mut queued in requests {
        let emitter = request_emitter(&queued.request);
        let is_retire = matches!(queued.request, Request::Retire { .. });
        let state = retiring.0.get(&emitter).copied();
        let stale_component = matches!(emitter, Emitter::Component { handle, generation }
            if state != Some(ObjectRetirement::Queued)
                && !physics.as_ref().is_some_and(|scene| scene.audio_component_live(handle, generation)));
        let stale_equipment = matches!(emitter.player_owner(), Emitter::Equipment { entity, .. }
            if state != Some(ObjectRetirement::Queued)
                && !live.as_ref().is_some_and(|live| live.stage(entity)
                    == Some(crate::gameplay::entity_lifetime::Stage::Spawned)));
        if !is_retire && (stale_component || stale_equipment
            || matches!(state, Some(ObjectRetirement::Submitted | ObjectRetirement::Complete))) {
            if queued.watched.is_some() {
                match queued.request {
                    Request::Post { event, .. } | Request::PostWithParameter { event, .. } | Request::PostAt { event, .. } => completions.failed_watch(event, emitter, queued.watched),
                    _ => {}
                }
            }
            continue;
        }
        let request = queued.request;
        let (event, emitter, parameter, position) = match request {
            Request::Post { event, emitter } => (event, emitter, None, None),
            Request::PostWithParameter { event, emitter, parameter, value } => (event, emitter, Some((parameter, value)), None),
            Request::PostAt { event, emitter, parameter, position, front } =>
                (event, emitter, parameter, Some(spatial::Position { position, front })),
            Request::Retire { emitter } => {
                cancel_pending_posts(&mut pending.0, &mut completions, emitter, None);
                waiting.retain(|object| *object != emitter);
                match engine.release_object(emitter) {
                    Ok(()) => { retiring.0.insert(emitter, ObjectRetirement::Submitted); },
                    Err(error) => { warn!("Sound object retirement failed: {error}"); pending.0.push(queued); },
                }
                continue;
            }
            Request::Stop { event, emitter } => {
                // 141127000 stops voices AND cancels pending actions (1410e9950).
                cancel_pending_posts(&mut pending.0, &mut completions, emitter, Some(event));
                if !pending.0.iter().any(|post| request_emitter(&post.request) == emitter) {
                    waiting.retain(|object| *object != emitter);
                }
                engine.stop_event(event, emitter);
                continue;
            }
        };
        // Loading is not the pause menu's snapshot of existing playing IDs.
        // Keep world posts ordered and owned until the real input handoff;
        // stop/retire requests above continue to drain, and UI stays audible.
        if emitter != Emitter::Menu && !exit.as_ref().is_some_and(|exit| exit.requested())
            && loading.as_ref().is_some_and(|state| super::loading_screen::is_loading(state)) {
            pending.0.push(queued);
            continue;
        }
        if queued.watched.is_some() && completions.waiting.get(&(event, emitter)).copied() != queued.watched { continue; }
        if queued.ticket.is_none() {
            let playing_id = engine.playing_id();
            let admitted = completions.playing.register_with_id(playing_id, crate::gameplay::sound::playing::Event {
                declaration: event, object: emitter, callbacks: if queued.watched.is_some() { 8 } else { 0 },
            });
            if let Some(observer) = &observer {
                observer.record_admission(queued.message_id, playing_id, event, emitter,
                    completions.playing.object_count(emitter), admitted.as_ref().err());
            }
            match admitted {
                Ok(ticket) => queued.ticket = Some(ticket),
                Err(error) => {
                    warn!("Wwise event registration failed: {error}");
                    completions.failed_watch(event, emitter, queued.watched);
                    continue;
                },
            }
        }
        if waiting.contains(&emitter) { pending.0.push(queued); continue; }
        // 1408abb10 pauses components on a zero owner clock; 140709110
        // forbids posting into a frozen component. Stop requests above still run.
        if scene.frozen(emitter) { completions.failed_watch(event, emitter, queued.watched); continue; }
        if queued.watched.is_some() && !registry.sources.contains_key(&event) {
            if let Some(error) = completions.decode_failures.remove(&event) {
                warn!("Wwise event {event:#010x} background preparation failed: {error}");
                completions.failed_watch(event, emitter, queued.watched);
                continue;
            }
            decodes.request(event, DecodeClient::Watched, &registry);
            waiting.push(emitter);
            pending.0.push(queued);
            continue;
        }
        if queued.watched.is_some() && registry.claim_dynamic_source(event, DecodeClient::Watched) {
            completions.dynamic_sources.insert(event);
        }
        if let Err(error) = registry.prepare(event) {
            completions.failed_watch(event, emitter, queued.watched);
            warn!("Wwise event {event:#010x} has no supported route: {error}");
            continue;
        }
        if let Some(actions) = registry.sources.get(&event).cloned() {
            let mut holds = Vec::new();
            let mut loading = false;
            let mut stream_failed = false;
            if graph::preload_event_streams() {
                for action in actions.iter() {
                    let SourceAction::Play { definition, .. } = action else { continue };
                    for source in definition.streamed() {
                        match source.acquire() {
                            Ok(hold) => { loading |= !hold.is_ready(); holds.push(hold); },
                            Err(error) => { stream_failed = true; warn!("Wwise event {event:#010x} streamed media unavailable: {error}"); },
                        }
                    }
                }
            }
            // All previously ready sources now have replacement holds. Keep
            // the old ones alive through acquisition, then transfer ownership.
            drop(queued.media);
            if stream_failed && queued.watched.is_some() {
                completions.failed_watch(event, emitter, queued.watched);
                continue;
            }
            if loading {
                waiting.push(emitter);
                pending.0.push(PendingSound { request, message_id: queued.message_id, media: holds, watched: queued.watched, ticket: queued.ticket });
                continue;
            }
            let tracked = queued.ticket.take();
            let playing_id = tracked.as_ref().expect("admitted ordinary post").id().value();
            if queued.watched.is_some() {
                completions.waiting.remove(&(event, emitter));
                completions.active.insert((event, emitter), tracked.as_ref().unwrap().id());
            }
            for action in actions.iter().cloned() {
                match action {
                    SourceAction::SetState { action, target_path } => {
                        if let Err(error) = engine.post_state_with_ticket(action, target_path, event, emitter, tracked.clone()) {
                            warn!("Wwise event {event:#010x} Set State unavailable: {error}");
                        }
                    },
                    SourceAction::Stop { action, target_path } => {
                        if let Err(error) = engine.post_stop_with_ticket(action, target_path, event, emitter, tracked.clone()) {
                            warn!("Wwise event {event:#010x} Stop unavailable: {error}");
                        }
                    },
                    SourceAction::Play { settings, definition } => {
                        match definition.prepare(registry.scheduled_selections.clone(), emitter, mixer, parameter, &scene, position, settings) {
                            Ok((mut source, scheduled)) => {
                                // The audio source survives ECS removal while
                                // its output tail/stop is still being processed.
                                // End belongs to that real owner, not its proxy.
                                if let Some(ticket) = &tracked { source.retain_event(ticket.clone()); }
                                let audio_target = AudioTarget { event, emitter, paused: false, world_paused: false };
                                if source.can_route() {
                                    match engine.submit(source, playing_id, event) {
                                        Ok(voice) => {
                                            commands.spawn((voice, audio_target, scheduled, StreamedMedia(holds.clone())));
                                        },
                                        Err(error) => { error!("Wwise event {event:#010x} shared output unavailable: {error}"); continue; },
                                    }
                                } else {
                                    match engine.register_fallback(scheduled.clone(), event, emitter) {
                                        Ok(control) => {
                                            commands.spawn((AudioPlayer(scheduled_sources.add(source)), PlaybackSettings::DESPAWN,
                                                audio_target, scheduled, control, StreamedMedia(holds.clone())));
                                        },
                                        Err(error) => error!("Wwise event {event:#010x} action control unavailable: {error}"),
                                    }
                                }
                            },
                            Err(error) => warn!("Wwise event {event:#010x} scheduled mix unavailable: {error}"),
                        }
                    }
                }
            }
        } else { completions.failed_watch(event, emitter, queued.watched); }
    }
    // Cancellation releases only this client's preparation lease. A shared FX
    // decode keeps its own client, and a running task cannot republish here
    // once the last watched request has gone.
    let wanted: std::collections::HashSet<_> = completions.waiting.keys().map(|(event, _)| *event).collect();
    decodes.retain_client(DecodeClient::Watched, &wanted);
}

/// Wwise RTPC changes affect sounds already playing. Mirror that when a
/// settings apply/restore advances the shared cvar generation.
fn update_playing_volume(
    mut mix: ResMut<AudioMix>,
    scene: Res<spatial::Scene>,
    paused: Option<Res<crate::gameplay::game_clock_runtime::GamePaused>>,
    mut previous_pause: Local<bool>,
    engine: Res<bevy_backend::Engine>,
    mut playing: Query<(&mut AudioTarget, Option<&AudioSink>, &graph::Playing, Option<&bevy_backend::VoiceHandle>, Option<&bevy_backend::FallbackHandle>)>,
) {
    let shared = crate::platform::console::shared();
    let generation = shared.generation();
    let mix_changed = mix.generation != generation;
    if mix_changed { mix.apply_settings(); }
    let Some(mixer) = &mix.mixer else { return };
    if let Err(error) = engine.update(mixer) { error!("Shared Wwise audio mix update failed: {error}"); }
    let world_paused = paused.is_some_and(|paused| paused.0);
    let pause_changed = world_paused != *previous_pause;
    engine.update_action_pause(pause_changed.then_some(world_paused), |emitter| scene.frozen(emitter));
    // Each voice publishes only its own targets; the update is on the main
    // world's critical path while walking, so voices update in parallel.
    playing.par_iter_mut().batching_strategy(bevy::ecs::batching::BatchingStrategy::fixed(1))
        .for_each(|(mut target, sink, scheduled, voice, fallback)| {
        // 788940 -> c7d30/c7d40 -> c4300 snapshots the currently playing
        // IDs. It does not silence later UI posts: 02cbd0 uses the same
        // world's global object, and c4550 posts new IDs while paused.
        // Resume (788d70 -> c8070/c8080 -> c59c0) releases that pause.
        if pause_changed { target.world_paused = world_paused; }
        let frozen = scene.frozen(target.emitter) || target.world_paused;
        if let Some(fallback) = fallback { fallback.pause(frozen); }
        if let Some(voice) = voice {
            voice.pause(frozen);
            target.paused = frozen;
            if !frozen {
                if let Err(error) = scheduled.update(mixer, &scene) { warn!("Wwise shared voice update unavailable: {error}"); }
            }
            return;
        }
        let Some(sink) = sink else { return };
        if frozen != target.paused {
            if frozen { sink.pause(); } else { sink.play(); }
            target.paused = frozen;
        }
        if frozen { return; }
        if let Err(error) = scheduled.update(mixer, &scene) { warn!("Wwise scheduled mix update unavailable: {error}"); }
    });
    *previous_pause = world_paused;
    mix.generation = generation;
}


/// Move the session owner's acquisition data to the loading worker while map
/// sound systems are suspended. The existing output callback keeps its Arc;
/// no bank/media copy or second audio output is made. Empty slots retain the
/// ECS resource identities until the same owner data is published back.
pub(super) struct AudioPreparation {
    registry: WwiseRegistry, mix: AudioMix, engine: bevy_backend::Engine,
    outputs: crate::rendering::bevy_backend::asset_preparation::AssetBatch<bevy_backend::OutputAudio>,
    playback: PlaybackBackend,
}
impl AudioPreparation {
    pub fn take(world: &mut World) -> Self {
        Self {
            registry: std::mem::take(&mut *world.resource_mut::<WwiseRegistry>()),
            mix: std::mem::take(&mut *world.resource_mut::<AudioMix>()),
            engine: std::mem::take(&mut *world.resource_mut::<bevy_backend::Engine>()),
            outputs: crate::rendering::bevy_backend::asset_preparation::AssetBatch::new(world.resource::<Assets<bevy_backend::OutputAudio>>()),
            playback: *world.resource::<PlaybackBackend>(),
        }
    }
    pub fn prepare(&mut self, commands: &mut super::setup_commands::SetupCommands,
        catalog: Arc<crate::archive::Catalog>, map: &str, seed: declaration_texts::PhysicsSeed) {
        let contacts = contacts::ContactSounds::acquire_with_seed(catalog.clone(), map, seed);
        prepare_banks(&mut self.registry, &mut self.mix, &mut self.engine, &mut self.outputs, commands, &catalog, map, self.playback);
        // Host: cold media reads/decodes must not occupy the frame executor.
        // Keep its existing parallelism rather than the smaller async pool,
        // and release these workers before scene preparation continues.
        let pool = (std::env::var("DH2_DEBUG_SOUND_FRAME_POOL").as_deref() != Ok("1"))
            .then(|| bevy::tasks::TaskPoolBuilder::new()
                .num_threads(bevy::tasks::ComputeTaskPool::get().thread_num().max(1))
                .thread_name("sound acquisition".to_owned())
                .build());
        prepare_listed_events(&mut self.registry, pool.as_ref(), contacts.declarations(), &catalog, map);
        commands.insert_resource(contacts);
    }
    pub fn publish(self, world: &mut World) {
        world.insert_resource(self.registry);
        world.insert_resource(self.mix);
        world.insert_resource(self.engine);
        self.outputs.publish(&mut world.resource_mut::<Assets<bevy_backend::OutputAudio>>());
    }
}

/// Resolve every installed map's bank selection without acquiring media.
pub(super) fn observe_bank_inventory(base: &std::path::Path) -> Result<(), String> {
    let catalog = crate::archive::Catalog::open(base).map_err(|error| error.to_string())?;
    let language = crate::assets::lang::read_audio_language(base)?;
    let language = crate::assets::wwise::language_name(language).ok_or("unsupported audio package language")?;
    let mut count = 0;
    for path in catalog.paths_with_suffix(".mapresources") {
        let Some(name) = path.strip_prefix("generated/buildgame/")
            .and_then(|path| path.strip_suffix(".mapresources")) else { continue };
        if matches!(name, "init" | "common") { continue; }
        let entities = format!("maps/{name}.entities");
        if !catalog.contains(&entities) { return Err(format!("manifest has no entities companion: {entities}")); }
        let map = name;
        let selection = level_banks(base, language, &entities).map_err(|error| error.to_string())?;
        let mut banks = selection.banks.into_iter().collect::<Vec<_>>();
        let mut shared_banks = selection.shared_banks.into_iter().collect::<Vec<_>>();
        let mut shared_packages = selection.shared_packages.into_iter().collect::<Vec<_>>();
        banks.sort_unstable();
        shared_banks.sort_unstable();
        shared_packages.sort_unstable();
        eprintln!(r#"AUDIO_BANK_CONTEXT {{"map":{map:?},"language":{language:?},"packages":{:?},"banks":{banks:?},"shared_packages":{shared_packages:?},"shared_banks":{shared_banks:?}}}"#,
            selection.packages);
        count += 1;
    }
    if count == 0 { return Err("no installed audio bank maps were resolved".into()); }
    eprintln!("AUDIO_BANK_CONTEXT_COMPLETE maps={count}");
    Ok(())
}
