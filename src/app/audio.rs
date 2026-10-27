//! Wwise events from pause UI and player gameplay.
use bevy::{audio::{AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings}, prelude::*};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use crate::gameplay::sound::{objects, Emitter, Request};
mod pcm;
mod graph;
mod bevy_backend;
mod fade;
mod contacts;
mod effects;
mod spatial;
pub(super) mod level;
pub(super) use contacts::ContactSound;

#[derive(Message, Clone, Copy)]
pub struct SoundEvent(pub Request);

#[derive(Resource, Default)]
struct WwiseRegistry {
    banks: Vec<Bank>,
    attenuations: HashMap<u32, Arc<crate::assets::wwise::Attenuation>>,
    library: crate::assets::wwise::PlaybackBanks<'static>,
    sources: HashMap<u32, Vec<SourceAction>>,
    scheduled_selections: graph::Selections,
    states: bevy_backend::StateGroups,
}

impl WwiseRegistry {
    fn playback(&self, event: u32) -> Result<Vec<crate::assets::wwise::PlaybackCommand>, Box<dyn std::error::Error>> {
        let mut files = HashMap::<usize, std::fs::File>::new();
        self.library.event_playback(event, |owner, media_id| {
            let bank = self.banks.get(owner).ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData, "Wwise source bank ownership is absent"))?;
            let Some(media) = bank.package.media_file(media_id as u64, bank.entry.language_id) else { return Ok(None); };
            let file = match files.entry(owner) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(entry) => entry.insert(std::fs::File::open(&bank.path)?),
            };
            bank.package.read_entry(file, media).map(Some)
        })?.ok_or_else(|| "event has no supported Wwise actions".into())
    }
    fn prepare(&mut self, event: u32) -> Result<(), Box<dyn std::error::Error>> {
        if self.sources.contains_key(&event) { return Ok(()); }
        let graph_actions = self.playback(event)?;
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
                        definition: Arc::new(graph::Definition::decode(node, &self.attenuations)?) }),
                    } })();
                    match decoded {
                        Ok(action) => actions.push(action),
                        Err(error) => warn!("Wwise event {event:#010x} action unavailable: {error}"),
                    }
                }
        self.sources.insert(event, actions);
        Ok(())
    }

}

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

struct Bank {
    path: PathBuf,
    package: crate::assets::wwise::Package,
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

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<graph::ScheduledAudio>()
            .add_audio_source::<bevy_backend::OutputAudio>()
            .add_message::<SoundEvent>()
            .add_message::<ContactSound>()
            .init_resource::<contacts::ContactSounds>()
            .init_resource::<objects::EffectObjects>()
            .init_resource::<effects::EffectAudio>()
            .init_resource::<WwiseRegistry>()
            .init_resource::<AudioMix>()
            .init_resource::<bevy_backend::Engine>()
            .init_resource::<spatial::Scene>()
            .add_systems(Startup, index_banks)
            .add_systems(Update, contacts::update.after(super::player::update_player))
            .add_systems(PostUpdate, (bevy_backend::retire, spatial::update, level::update, effects::update, update_playing_volume, play_events)
                .chain().after(super::model_motion::publish_model_poses));
    }
}

fn index_banks(mut registry: ResMut<WwiseRegistry>, mut mix: ResMut<AudioMix>,
    mut engine: ResMut<bevy_backend::Engine>, mut outputs: ResMut<Assets<bevy_backend::OutputAudio>>, mut commands: Commands) {
    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
        let base = crate::assets::installation::game_base()?;
        let audio_language = crate::assets::lang::read_audio_language(&base)?;
        let package_language = crate::assets::wwise::language_name(audio_language).ok_or("unsupported audio package language")?;
        let mut paths = std::fs::read_dir(base.join("pck"))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pck")))
            .collect::<Vec<_>>();
        paths.sort();

        let mut banks = Vec::new();
        let mut attenuations = HashMap::new();
        let mut output_mix = None;
        let mut library = crate::assets::wwise::PlaybackBanks::default();
        for path in paths {
            let Ok(package) = crate::assets::wwise::Package::open(&path) else { continue };
            let language = package.language_id(package_language);
            for entry in package.banks.iter().copied().filter(|entry| entry.language_id == 0 || Some(entry.language_id) == language) {
                let indexed = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let mut file = std::fs::File::open(&path)?;
                    let bytes = package.read_entry(&mut file, entry)?;
                    Ok((crate::assets::wwise::bank_mix(&bytes)?, crate::assets::wwise::bank_attenuations(&bytes)?, bytes))
                })();
                match indexed {
                    Ok((bank_mix, bank_attenuations, bytes)) => {
                        for attenuation in bank_attenuations { attenuations.entry(attenuation.id).or_insert_with(|| Arc::new(attenuation)); }
                        if bank_mix.is_some() { output_mix = bank_mix; }
                        library.insert(bytes.into())?;
                        banks.push(Bank { path: path.clone(), package: package.clone(), entry });
                    }
                    Err(error) => debug!("Skipping Wwise bank in {}: {error}", path.display()),
                }
            }
        }
        let catalog = crate::archive::Catalog::open(&base)?;
        let source = catalog.read("generated/decls/soundengineinternals/default.soundengineinternals.decl")?
            .ok_or("sound engine internals declaration is absent")?;
        let fields = crate::entities::parse_declaration(&crate::entities::declaration_text(&source))?;
        use crate::entities::scene_motion::{atom, child};
        let mut bindings = Vec::new();
        for (field, cvar) in [
            ("m_RTPC_MasterVolume", "s_MasterBus_Gain"), ("m_RTPC_EffectsVolume", "s_SFX_Gain"),
            ("m_RTPC_SpeechVolume", "s_Voices_Gain"), ("m_RTPC_MusicVolume", "s_Music_Gain"),
            ("m_RTPC_VideoVolume", "s_Video_Gain"),
        ] {
            let path = atom(child(&fields, "edit"), field).ok_or("sound engine volume RTPC is absent")?;
            let source = catalog.read(&format!("generated/decls/soundrtpc/{path}.decl"))?.ok_or("volume RTPC declaration is absent")?;
            bindings.push((crate::assets::wwise::declared_rtpc_id(&crate::entities::declaration_text(&source))?, cvar));
        }
        let path = atom(child(&fields, "edit"), "m_RTPC_TimeDilation").ok_or("sound time-dilation RTPC is absent")?;
        let source = catalog.read(&format!("generated/decls/soundrtpc/{path}.decl"))?.ok_or("sound time-dilation declaration is absent")?;
        let time_dilation = crate::assets::wwise::declared_rtpc_id(&crate::entities::declaration_text(&source))?;
        let path = atom(child(&fields, "edit"), "m_RTPC_PhysicsScale").ok_or("physics sound RTPC is absent")?;
        let source = catalog.read(&format!("generated/decls/soundrtpc/{path}.decl"))?.ok_or("physics sound RTPC declaration is absent")?;
        let physics_scale = crate::assets::wwise::RtpcDeclaration::parse(&crate::entities::declaration_text(&source))?;
        Ok((banks, attenuations, library, output_mix.ok_or("Wwise initialization mix is absent")?, bindings, time_dilation, physics_scale))
    })();
    match result {
        Ok((banks, attenuations, library, output_mix, bindings, time_dilation, physics_scale)) => {
            registry.states = Arc::new(std::sync::Mutex::new(crate::gameplay::sound::states::States::new(output_mix.state_groups.iter().cloned())));
            mix.time_dilation = Some(time_dilation);
            mix.physics_scale = Some(physics_scale);
            registry.attenuations = attenuations;
            info!("Indexed {} installed Wwise banks, {} event IDs and {} global actor IDs", banks.len(), library.event_count(), library.actor_count());
            registry.banks = banks;
            registry.library = library;
            mix.mixer = Some(output_mix.into());
            mix.bindings = bindings;
            mix.apply_settings();
            if let Some(mixer) = &mix.mixer {
                match bevy_backend::Engine::new(mixer, registry.states.clone()) {
                    Ok((initialized, output)) => {
                        if let Some(mixer) = &mut mix.mixer { mixer.activity = initialized.activity(); }
                        *engine = initialized;
                        commands.spawn((AudioPlayer(outputs.add(output)), PlaybackSettings::ONCE));
                        info!("Shared stereo audio output initialized with installed bus effects");
                    },
                    Err(error) => error!("Shared Wwise audio output unavailable: {error}"),
                }
            }
        }
        Err(error) => warn!("Installed Wwise banks unavailable: {error}"),
    }
}


fn play_events(
    mut commands: Commands,
    mut events: MessageReader<SoundEvent>,
    mut registry: ResMut<WwiseRegistry>,
    mut scheduled_sources: ResMut<Assets<graph::ScheduledAudio>>,
    mix: Res<AudioMix>,
    scene: Res<spatial::Scene>,
    engine: Res<bevy_backend::Engine>,
) {
    let Some(mixer) = &mix.mixer else { events.clear(); return };
    for message in events.read() {
        let (event, emitter, parameter, position) = match message.0 {
            Request::Post { event, emitter } => (event, emitter, None, None),
            Request::PostWithParameter { event, emitter, parameter, value } => (event, emitter, Some((parameter, value)), None),
            Request::PostAt { event, emitter, parameter, position, front } =>
                (event, emitter, parameter, Some(spatial::Position { position, front })),
            Request::Stop { event, emitter } => {
                engine.stop_event(event, emitter);
                continue;
            }
        };
        // 1408abb10 pauses components on a zero owner clock; 140709110
        // forbids posting into a frozen component. Stop requests above still run.
        if scene.frozen(emitter) { continue; }
        if let Err(error) = registry.prepare(event) {
            warn!("Wwise event {event:#010x} has no supported route: {error}");
            continue;
        }
        if let Some(actions) = registry.sources.get(&event).cloned() {
            let playing_id = engine.playing_id();
            for action in actions {
                match action {
                    SourceAction::SetState { action, target_path } => {
                        if let Err(error) = engine.post_state(action, target_path, event, emitter) {
                            warn!("Wwise event {event:#010x} Set State unavailable: {error}");
                        }
                    },
                    SourceAction::Stop { action, target_path } => {
                        if let Err(error) = engine.post_stop(action, target_path, event, emitter) {
                            warn!("Wwise event {event:#010x} Stop unavailable: {error}");
                        }
                    },
                    SourceAction::Play { settings, definition } => {
                        match definition.prepare(registry.scheduled_selections.clone(), emitter, mixer, parameter, &scene, position, settings) {
                            Ok((source, scheduled)) => {
                                let audio_target = AudioTarget { event, emitter, paused: false, world_paused: false };
                                if source.can_route() {
                                    match engine.submit(source, playing_id, event) {
                                        Ok(voice) => { commands.spawn((voice, audio_target, scheduled)); },
                                        Err(error) => { error!("Wwise event {event:#010x} shared output unavailable: {error}"); continue; },
                                    }
                                } else {
                                    match engine.register_fallback(scheduled.clone(), event, emitter) {
                                        Ok(control) => { commands.spawn((AudioPlayer(scheduled_sources.add(source)), PlaybackSettings::DESPAWN,
                                            audio_target, scheduled, control)); },
                                        Err(error) => error!("Wwise event {event:#010x} action control unavailable: {error}"),
                                    }
                                }
                            },
                            Err(error) => warn!("Wwise event {event:#010x} scheduled mix unavailable: {error}"),
                        }
                    }
                }
            }
        }
    }
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
    for (mut target, sink, scheduled, voice, fallback) in &mut playing {
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
            continue;
        }
        let Some(sink) = sink else { continue };
        if frozen != target.paused {
            if frozen { sink.pause(); } else { sink.play(); }
            target.paused = frozen;
        }
        if frozen { continue; }
        if let Err(error) = scheduled.update(mixer, &scene) { warn!("Wwise scheduled mix update unavailable: {error}"); }
    }
    *previous_pause = world_paused;
    mix.generation = generation;
}
