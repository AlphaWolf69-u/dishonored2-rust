//! Wwise events from pause UI and player gameplay.
use bevy::{audio::{AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings, Volume}, prelude::*};
use std::{collections::{HashMap, HashSet, VecDeque}, path::PathBuf, sync::Arc};
use crate::gameplay::sound::{Emitter, Request};
mod pcm;
use pcm::PcmAudio;

#[derive(Message, Clone, Copy)]
pub struct SoundEvent(pub Request);

#[derive(Resource, Default)]
struct WwiseRegistry {
    banks: Vec<Bank>,
    event_banks: HashMap<u32, Vec<usize>>,
    sources: HashMap<u32, Vec<SourceAction>>,
    selections: HashMap<(u32, Option<Emitter>), SelectionState>,
}

#[derive(Resource, Default)]
struct AudioMix {
    generation: u64,
    mixer: Option<crate::gameplay::sound::mix::Mixer>,
    bindings: Vec<(u32, &'static str)>,
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
    variants: Vec<AudioVariant>,
}

#[derive(Clone)]
struct AudioVariant {
    source: Handle<PcmAudio>,
    weight: u32,
    properties: Arc<crate::assets::wwise::MediaProperties>,
}

#[derive(Default)]
struct SelectionState {
    position: usize,
    history: VecDeque<usize>,
    played: HashSet<usize>,
}

impl SelectionState {
    fn select(&mut self, group: &AudioGroup) -> Option<usize> {
        use crate::assets::wwise::SelectionMode;
        match group.mode {
            SelectionMode::Single => (!group.variants.is_empty()).then_some(0),
            SelectionMode::Sequence => {
                if group.variants.is_empty() { return None; }
                let selected = self.position % group.variants.len();
                if !group.reset_each_play { self.position = self.position.wrapping_add(1); }
                Some(selected)
            }
            SelectionMode::Random | SelectionMode::Shuffle => {
                let available = group.variants.iter().filter(|variant| variant.weight > 0).count();
                let repeat_limit = usize::from(group.avoid_repeat).min(available.saturating_sub(1));
                while self.history.len() > repeat_limit { self.history.pop_front(); }
                let shuffle = group.mode == SelectionMode::Shuffle;
                if shuffle && self.played.len() >= available { self.played.clear(); }
                let eligible = |i: usize| !self.history.contains(&i) && (!shuffle || !self.played.contains(&i));
                let total = group.variants.iter().enumerate().filter(|(i, _)| eligible(*i))
                    .map(|(_, variant)| u64::from(variant.weight)).sum::<u64>();
                if total == 0 { return None; }
                let mut pick = rand::random_range(0..total);
                let selected = group.variants.iter().enumerate().position(|(i, variant)| {
                    if !eligible(i) { return false; }
                    if pick < u64::from(variant.weight) { true } else { pick -= u64::from(variant.weight); false }
                })?;
                if shuffle { self.played.insert(selected); }
                self.history.push_back(selected);
                while self.history.len() > repeat_limit { self.history.pop_front(); }
                Some(selected)
            }
        }
    }
}

#[derive(Clone)]
enum SourceAction {
    Stop(u32),
    Play { target: u32, groups: Vec<AudioGroup> },
}

#[derive(Component, Clone)]
struct AudioTarget {
    target: u32,
    event: u32,
    emitter: Emitter,
    properties: Arc<crate::assets::wwise::MediaProperties>,
    random_db: f32,
}

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<PcmAudio>()
            .add_message::<SoundEvent>()
            .init_resource::<WwiseRegistry>()
            .init_resource::<AudioMix>()
            .add_systems(Startup, index_banks)
            .add_systems(Update, (update_playing_volume, play_events).chain());
    }
}

fn index_banks(mut registry: ResMut<WwiseRegistry>, mut mix: ResMut<AudioMix>) {
    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
        let base = crate::assets::installation::game_base()?;
        let mut paths = std::fs::read_dir(base.join("pck"))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("pck")))
            .collect::<Vec<_>>();
        paths.sort();

        let mut banks = Vec::new();
        let mut output_mix = None;
        let mut event_banks = HashMap::<u32, Vec<usize>>::new();
        for path in paths {
            let Ok(package) = crate::assets::wwise::Package::open(&path) else { continue };
            for entry in package.banks.iter().copied().filter(|entry| entry.language_id == 0) {
                let indexed = (|| -> Result<_, Box<dyn std::error::Error>> {
                    let mut file = std::fs::File::open(&path)?;
                    let bytes = package.read_entry(&mut file, entry)?;
                    Ok((crate::assets::wwise::bank_event_ids(&bytes)?, crate::assets::wwise::bank_mix(&bytes)?))
                })();
                match indexed {
                    Ok((events, bank_mix)) => {
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
        Ok((banks, event_banks, output_mix.ok_or("Wwise initialization mix is absent")?, bindings))
    })();
    match result {
        Ok((banks, event_banks, output_mix, bindings)) => {
            info!("Indexed {} installed Wwise banks and {} event IDs", banks.len(), event_banks.len());
            registry.banks = banks;
            registry.event_banks = event_banks;
            mix.mixer = Some(output_mix.into());
            mix.bindings = bindings;
            mix.apply_settings();
        }
        Err(error) => warn!("Installed Wwise banks unavailable: {error}"),
    }
}

fn play_events(
    mut commands: Commands,
    mut events: MessageReader<SoundEvent>,
    mut registry: ResMut<WwiseRegistry>,
    mut sources: ResMut<Assets<PcmAudio>>,
    mix: Res<AudioMix>,
    playing: Query<(Entity, &AudioTarget)>,
) {
    let Some(mixer) = &mix.mixer else { events.clear(); return };
    let mut spawned = Vec::<(Entity, AudioTarget)>::new();
    for message in events.read() {
        let (event, emitter) = match message.0 {
            Request::Post { event, emitter } => (event, emitter),
            Request::Stop { event, emitter } => {
                for (entity, target) in &playing {
                    if target.event == event && target.emitter == emitter { commands.entity(entity).try_despawn(); }
                }
                spawned.retain(|(entity, target)| {
                    let stop = target.event == event && target.emitter == emitter;
                    if stop { commands.entity(*entity).try_despawn(); }
                    !stop
                });
                continue;
            }
        };
        if !registry.sources.contains_key(&event) {
            let decoded = (|| -> Result<Vec<SourceAction>, Box<dyn std::error::Error>> {
                let indexes = registry.event_banks.get(&event).ok_or("no installed bank contains this event")?;
                let mut actions = None;
                for &index in indexes {
                    let bank = &registry.banks[index];
                    let mut file = std::fs::File::open(&bank.path)?;
                    let bytes = bank.package.read_entry(&mut file, bank.entry)?;
                    let resolved = crate::assets::wwise::event_commands_with(&bytes, event, |media_id| {
                        let Some(media) = bank.package.media_file(media_id as u64, bank.entry.language_id) else { return Ok(None) };
                        bank.package.read_entry(&mut file, media).map(Some)
                    })?;
                    if resolved.is_some() { actions = resolved; break; }
                }
                let actions = actions.ok_or("event has no supported Wwise actions")?;
                actions.into_iter().map(|action| match action {
                    crate::assets::wwise::EventCommand::Stop { target } => Ok(SourceAction::Stop(target)),
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
                    SourceAction::Stop(target) => {
                        for (entity, audio_target) in &playing {
                            if audio_target.emitter == emitter && (audio_target.target == target || audio_target.properties.nodes.iter().any(|node| node.id == target)) {
                                commands.entity(entity).try_despawn();
                            }
                        }
                        spawned.retain(|(entity, audio_target)| {
                            let stop = audio_target.emitter == emitter && (audio_target.target == target || audio_target.properties.nodes.iter().any(|node| node.id == target));
                            if stop { commands.entity(*entity).try_despawn(); }
                            !stop
                        });
                    }
                    SourceAction::Play { target, groups } => {
                        let mut random_values = HashMap::new();
                        for group in groups {
                            let owner = if group.global { None } else { Some(emitter) };
                            let Some(selected) = registry.selections.entry((group.node, owner)).or_default().select(&group) else { continue };
                            let variant = &group.variants[selected];
                            let random_db = variant.properties.nodes.iter().map(|node| *random_values.entry(node.id).or_insert_with(|| {
                                node.ranges.iter().filter(|range| matches!(range.property, 0 | 5 | 0x21 | 0x17)).map(|range| {
                                    if range.min == range.max { range.min } else { rand::random_range(range.min..=range.max) }
                                }).sum::<f32>()
                            })).sum::<f32>();
                            let gain = match mixer.gain(&variant.properties, random_db) {
                                Ok(gain) => gain,
                                Err(error) => { warn!("Wwise event {event:#010x} mix unavailable: {error}"); continue; }
                            };
                            let audio_target = AudioTarget { target, event, emitter, properties: variant.properties.clone(), random_db };
                            let entity = commands.spawn((
                                AudioPlayer(variant.source.clone()),
                                PlaybackSettings::DESPAWN.with_volume(Volume::Linear(gain)),
                                audio_target.clone(),
                            )).id();
                            spawned.push((entity, audio_target));
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
    mut playing: Query<(&mut AudioSink, &AudioTarget)>,
) {
    let shared = crate::platform::console::shared();
    let generation = shared.generation();
    if mix.generation == generation { return; }
    mix.apply_settings();
    let Some(mixer) = &mix.mixer else { return };
    for (mut sink, target) in &mut playing {
        match mixer.gain(&target.properties, target.random_db) {
            Ok(gain) => sink.set_volume(Volume::Linear(gain)),
            Err(error) => warn!("Wwise mix update unavailable: {error}"),
        }
    }
    mix.generation = generation;
}
