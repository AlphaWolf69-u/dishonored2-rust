//! Wwise events from pause UI and player gameplay.
use bevy::{audio::{AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume}, prelude::*};
use std::{collections::HashMap, path::PathBuf, sync::Arc};
use crate::gameplay::sound::{Emitter, Request};
mod pcm;
mod graph;
mod bevy_backend;
mod fade;
mod contacts;
mod spatial;
pub(super) mod level;
pub(super) use contacts::ContactSound;
use pcm::PcmAudio;

#[derive(Message, Clone, Copy)]
pub struct SoundEvent(pub Request);

#[derive(Resource, Default)]
struct WwiseRegistry {
    banks: Vec<Bank>,
    attenuations: HashMap<u32, Arc<crate::assets::wwise::Attenuation>>,
    event_banks: HashMap<u32, Vec<usize>>,
    sources: HashMap<u32, Vec<SourceAction>>,
    scheduled_selections: graph::Selections,
}

#[derive(Resource, Default)]
struct AudioMix {
    generation: u64,
    mixer: Option<crate::gameplay::sound::mix::Mixer>,
    bindings: Vec<(u32, &'static str)>,
    time_dilation: Option<u32>,
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
struct AudioGroup {
    node: u32,
    global: bool,
    avoid_repeat: u16,
    mode: crate::assets::wwise::SelectionMode,
    reset_each_play: bool,
    restart_backward: bool,
    variants: Vec<AudioVariant>,
}

#[derive(Clone)]
struct AudioVariant {
    source: Handle<PcmAudio>,
    weight: u32,
    properties: Arc<crate::assets::wwise::MediaProperties>,
}

#[derive(Clone)]
enum SourceAction {
    Stop(crate::assets::wwise::StopAction),
    Play { target: u32, groups: Vec<AudioGroup> },
    Scheduled { target: u32, definition: Arc<graph::Definition> },
}

#[derive(Component, Clone)]
enum PlaybackControl {
    Flat(fade::Control),
    Scheduled(graph::Playing),
}
impl PlaybackControl {
    fn stop(&self, target: u32, milliseconds: i32, curve: u8) {
        match self {
            Self::Flat(control) => control.stop(milliseconds, curve),
            Self::Scheduled(control) => control.stop(target, milliseconds, curve),
        }
    }
}

#[derive(Component, Clone)]
struct AudioTarget {
    target: u32,
    event: u32,
    emitter: Emitter,
    properties: Arc<crate::assets::wwise::MediaProperties>,
    random_db: f32,
    parameter: Option<(u32, f32)>,
    paused: bool,
    world_paused: bool,
}

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<PcmAudio>()
            .add_audio_source::<graph::ScheduledAudio>()
            .add_audio_source::<bevy_backend::OutputAudio>()
            .add_message::<SoundEvent>()
            .add_message::<ContactSound>()
            .init_resource::<contacts::ContactSounds>()
            .init_resource::<WwiseRegistry>()
            .init_resource::<AudioMix>()
            .init_resource::<bevy_backend::Engine>()
            .init_resource::<spatial::Scene>()
            .add_systems(Startup, index_banks)
            .add_systems(Update, contacts::update.after(super::player::update_player))
            .add_systems(PostUpdate, (bevy_backend::retire, spatial::update, level::update, update_playing_volume, play_events)
                .chain().after(super::model_motion::publish_model_poses));
    }
}

fn index_banks(mut registry: ResMut<WwiseRegistry>, mut mix: ResMut<AudioMix>,
    mut engine: ResMut<bevy_backend::Engine>, mut outputs: ResMut<Assets<bevy_backend::OutputAudio>>, mut commands: Commands) {
    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
        let base = crate::assets::installation::game_base()?;
        let mut paths = std::fs::read_dir(base.join("pck"))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pck")))
            .collect::<Vec<_>>();
        paths.sort();

        let mut banks = Vec::new();
        let mut attenuations = HashMap::new();
        let mut output_mix = None;
        let mut event_banks = HashMap::<u32, Vec<usize>>::new();
        for path in paths {
            let Ok(package) = crate::assets::wwise::Package::open(&path) else { continue };
            for entry in package.banks.iter().copied().filter(|entry| entry.language_id == 0) {
                let indexed = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let mut file = std::fs::File::open(&path)?;
                    let bytes = package.read_entry(&mut file, entry)?;
                    Ok((crate::assets::wwise::bank_event_ids(&bytes)?, crate::assets::wwise::bank_mix(&bytes)?, crate::assets::wwise::bank_attenuations(&bytes)?))
                })();
                match indexed {
                    Ok((events, bank_mix, bank_attenuations)) => {
                        for attenuation in bank_attenuations { attenuations.insert(attenuation.id, Arc::new(attenuation)); }
                        if bank_mix.is_some() { output_mix = bank_mix; }
                        let index = banks.len();
                        banks.push(Bank { path: path.clone(), package: package.clone(), entry });
                        for event in events { event_banks.entry(event).or_default().push(index); }
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
        Ok((banks, attenuations, event_banks, output_mix.ok_or("Wwise initialization mix is absent")?, bindings, time_dilation))
    })();
    match result {
        Ok((banks, attenuations, event_banks, output_mix, bindings, time_dilation)) => {
            mix.time_dilation = Some(time_dilation);
            registry.attenuations = attenuations;
            info!("Indexed {} installed Wwise banks and {} event IDs", banks.len(), event_banks.len());
            registry.banks = banks;
            registry.event_banks = event_banks;
            mix.mixer = Some(output_mix.into());
            mix.bindings = bindings;
            mix.apply_settings();
            if let Some(mixer) = &mix.mixer {
                match bevy_backend::Engine::new(mixer) {
                    Ok((initialized, output)) => {
                        *engine = initialized;
                        commands.spawn((AudioPlayer(outputs.add(output)), PlaybackSettings::ONCE));
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
    mut sources: ResMut<Assets<PcmAudio>>,
    mut scheduled_sources: ResMut<Assets<graph::ScheduledAudio>>,
    mix: Res<AudioMix>,
    scene: Res<spatial::Scene>,
    engine: Res<bevy_backend::Engine>,
    playing: Query<(Entity, &AudioTarget, Option<&PlaybackControl>)>,
) {
    let Some(mixer) = &mix.mixer else { events.clear(); return };
    let mut spawned = Vec::<(Entity, AudioTarget, Option<PlaybackControl>)>::new();
    for message in events.read() {
        let (event, emitter, parameter, position) = match message.0 {
            Request::Post { event, emitter } => (event, emitter, None, None),
            Request::PostWithParameter { event, emitter, parameter, value } => (event, emitter, Some((parameter, value)), None),
            Request::PostAt { event, emitter, parameter, position, front } =>
                (event, emitter, parameter, Some(spatial::Position { position, front })),
            Request::Stop { event, emitter } => {
                for (entity, target, _) in &playing {
                    if target.event == event && target.emitter == emitter { commands.entity(entity).try_despawn(); }
                }
                spawned.retain(|(entity, target, _)| {
                    let stop = target.event == event && target.emitter == emitter;
                    if stop { commands.entity(*entity).try_despawn(); }
                    !stop
                });
                continue;
            }
        };
        // 1408abb10 pauses components on a zero owner clock; 140709110
        // forbids posting into a frozen component. Stop requests above still run.
        if scene.frozen(emitter) { continue; }
        if !registry.sources.contains_key(&event) {
            let decoded = (|| -> Result<Vec<SourceAction>, Box<dyn std::error::Error>> {
                let indexes = registry.event_banks.get(&event).ok_or("no installed bank contains this event")?;
                let mut actions = None;
                for &index in indexes {
                    let bank = &registry.banks[index];
                    let mut file = std::fs::File::open(&bank.path)?;
                    let bytes = bank.package.read_entry(&mut file, bank.entry)?;
                    let graph_actions = crate::assets::wwise::event_playback_with(&bytes, event, |media_id| {
                        let Some(media) = bank.package.media_file(media_id as u64, bank.entry.language_id) else { return Ok(None) };
                        bank.package.read_entry(&mut file, media).map(Some)
                    })?;
                    if let Some(graph_actions) = graph_actions {
                        use crate::assets::wwise::PlaybackCommand;
                        if graph_actions.iter().any(|action| matches!(action, PlaybackCommand::Play { node, .. } if graph::Definition::required(node))) {
                            return graph_actions.into_iter().map(|action| match action {
                                PlaybackCommand::Stop(stop) => Ok(SourceAction::Stop(stop)),
                                PlaybackCommand::Play { target, node } => Ok(SourceAction::Scheduled {
                                    target, definition: Arc::new(graph::Definition::decode(node, &registry.attenuations)?),
                                }),
                            }).collect();
                        }
                    }
                    let resolved = crate::assets::wwise::event_commands_with(&bytes, event, |media_id| {
                        let Some(media) = bank.package.media_file(media_id as u64, bank.entry.language_id) else { return Ok(None) };
                        bank.package.read_entry(&mut file, media).map(Some)
                    })?;
                    if resolved.is_some() { actions = resolved; break; }
                }
                let actions = actions.ok_or("event has no supported Wwise actions")?;
                actions.into_iter().map(|action| match action {
                    crate::assets::wwise::EventCommand::Stop(stop) => Ok(SourceAction::Stop(stop)),
                    crate::assets::wwise::EventCommand::Play { target, groups } => {
                        let groups = groups.into_iter().map(|group| {
                            let variants = group.variants.into_iter().map(|variant| {
                                let source = PcmAudio::from_wem(&variant.media).map_err(|error| format!(
                                    "event {event:#010x}, source {:#010x}: {error}",
                                    variant.properties.nodes.first().map_or(0, |node| node.id)
                                ))?;
                                Ok(AudioVariant {
                                    source: sources.add(source),
                                    weight: variant.weight, properties: Arc::new(variant.properties),
                                })
                            }).collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
                            Ok(AudioGroup {
                                node: group.node, global: group.global, avoid_repeat: group.avoid_repeat,
                                mode: group.mode, reset_each_play: group.reset_each_play, variants,
                                restart_backward: group.restart_backward,
                            })
                        }).collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
                        Ok(SourceAction::Play { target, groups })
                    }
                }).collect()
            })();
            match decoded {
                Ok(actions) => { registry.sources.insert(event, actions); }
                Err(error) => {
                    warn!("Wwise event {event:#010x} has no supported route: {error}");
                    continue;
                }
            }
        }
        if let Some(actions) = registry.sources.get(&event).cloned() {
            for action in actions {
                match action {
                    SourceAction::Stop(stop) => {
                        let target = stop.target;
                        let milliseconds = stop.transition.sample_ms(|| rand::random_range(0..=32767));
                        for (entity, audio_target, scheduled) in &playing {
                            if stop.game_object && audio_target.emitter != emitter { continue; }
                            let matches = audio_target.target == target || audio_target.properties.nodes.iter().any(|node| node.id == target);
                            if matches && milliseconds <= 0 { commands.entity(entity).try_despawn(); }
                            else if let Some(control) = scheduled {
                                if matches || matches!(control, PlaybackControl::Scheduled(_)) {
                                    control.stop(target, milliseconds, stop.transition.curve);
                                }
                            }
                        }
                        spawned.retain(|(entity, audio_target, scheduled)| {
                            if stop.game_object && audio_target.emitter != emitter { return true; }
                            let matches = audio_target.target == target || audio_target.properties.nodes.iter().any(|node| node.id == target);
                            if matches && milliseconds <= 0 { commands.entity(*entity).try_despawn(); return false; }
                            if let Some(control) = scheduled {
                                if matches || matches!(control, PlaybackControl::Scheduled(_)) {
                                    control.stop(target, milliseconds, stop.transition.curve);
                                }
                            }
                            true
                        });
                    }
                    SourceAction::Play { target, groups } => {
                        let mut random_values = HashMap::new();
                        for group in groups {
                            let owner = if group.global { None } else { Some(emitter) };
                            let selected = {
                                use crate::assets::wwise::SelectionMode;
                                let mut selections = registry.scheduled_selections.lock().unwrap_or_else(|error| error.into_inner());
                                let state = selections.entry((group.node, owner)).or_default();
                                if group.mode == SelectionMode::Sequence && group.reset_each_play { state.sequence = Default::default(); }
                                if group.mode == SelectionMode::Single { (!group.variants.is_empty()).then_some(0) }
                                else { state.next(group.mode, &group.variants.iter().map(|variant| variant.weight).collect::<Vec<_>>(),
                                    group.avoid_repeat, group.restart_backward, None) }
                            };
                            let Some(selected) = selected else { continue };
                            let variant = &group.variants[selected];
                            let random_db = variant.properties.nodes.iter().map(|node| *random_values.entry(node.id).or_insert_with(|| {
                                node.ranges.iter().filter(|range| matches!(range.property, 0 | 5 | 0x21 | 0x17)).map(|range| {
                                    if range.min == range.max { range.min } else { rand::random_range(range.min..=range.max) }
                                }).sum::<f32>()
                            })).sum::<f32>();
                            let gain = match mixer.gain_with_parameters(&variant.properties, random_db, &scene.parameters(emitter, parameter)) {
                                Ok(gain) => gain,
                                Err(error) => { warn!("Wwise event {event:#010x} mix unavailable: {error}"); continue; }
                            };
                            let audio_target = AudioTarget { target, event, emitter, properties: variant.properties.clone(), random_db, parameter, paused: false, world_paused: false };
                            let Some(source) = sources.get(&variant.source) else { continue };
                            let (source, control) = source.controlled();
                            let control = PlaybackControl::Flat(control);
                            let entity = commands.spawn((
                                AudioPlayer(sources.add(source)),
                                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain)),
                                audio_target.clone(),
                                control.clone(),
                            )).id();
                            spawned.push((entity, audio_target, Some(control)));
                        }
                    }
                    SourceAction::Scheduled { target, definition } => {
                        match definition.prepare(registry.scheduled_selections.clone(), emitter, mixer, parameter, &scene, position) {
                            Ok((source, scheduled)) => {
                                let audio_target = AudioTarget { target, event, emitter, properties: definition.root_properties(), random_db: 0.0, parameter, paused: false, world_paused: false };
                                let scheduled = PlaybackControl::Scheduled(scheduled);
                                let entity = if source.can_route() {
                                    match engine.submit(source) {
                                        Ok(voice) => commands.spawn((voice, audio_target.clone(), scheduled.clone())).id(),
                                        Err(error) => { error!("Wwise event {event:#010x} shared output unavailable: {error}"); continue; },
                                    }
                                } else {
                                    commands.spawn((AudioPlayer(scheduled_sources.add(source)), PlaybackSettings::DESPAWN,
                                        audio_target.clone(), scheduled.clone())).id()
                                };
                                spawned.push((entity, audio_target, Some(scheduled)));
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
    mut playing: Query<(&mut AudioTarget, Option<&mut AudioSink>, Option<&PlaybackControl>, Option<&bevy_backend::VoiceHandle>)>,
) {
    let shared = crate::platform::console::shared();
    let generation = shared.generation();
    let mix_changed = mix.generation != generation;
    if mix_changed { mix.apply_settings(); }
    let Some(mixer) = &mix.mixer else { return };
    if let Err(error) = engine.update(mixer) { error!("Shared Wwise audio mix update failed: {error}"); }
    let world_paused = paused.is_some_and(|paused| paused.0);
    let pause_changed = world_paused != *previous_pause;
    for (mut target, sink, scheduled, voice) in &mut playing {
        // 788940 -> c7d30/c7d40 -> c4300 snapshots the currently playing
        // IDs. It does not silence later UI posts: 02cbd0 uses the same
        // world's global object, and c4550 posts new IDs while paused.
        // Resume (788d70 -> c8070/c8080 -> c59c0) releases that pause.
        if pause_changed { target.world_paused = world_paused; }
        let frozen = scene.frozen(target.emitter) || target.world_paused;
        if let Some(voice) = voice {
            voice.pause(frozen);
            target.paused = frozen;
            if !frozen {
                if let Some(PlaybackControl::Scheduled(scheduled)) = scheduled {
                    if let Err(error) = scheduled.update(mixer, &scene) { warn!("Wwise shared voice update unavailable: {error}"); }
                }
            }
            continue;
        }
        let Some(mut sink) = sink else { continue };
        if frozen != target.paused {
            if frozen { sink.pause(); } else { sink.play(); }
            target.paused = frozen;
        }
        if frozen { continue; }
        if let Some(PlaybackControl::Scheduled(scheduled)) = scheduled {
            if let Err(error) = scheduled.update(mixer, &scene) { warn!("Wwise scheduled mix update unavailable: {error}"); }
            continue;
        }
        match mixer.gain_with_parameters(&target.properties, target.random_db, &scene.parameters(target.emitter, target.parameter)) {
            Ok(gain) => sink.set_volume(Volume::Linear(gain)),
            Err(error) => warn!("Wwise mix update unavailable: {error}"),
        }
    }
    *previous_pause = world_paused;
    mix.generation = generation;
}
