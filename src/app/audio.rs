//! Wwise events from pause UI and player gameplay.
use bevy::{audio::{AddAudioSource, AudioPlayer, AudioSink, AudioSinkPlayback, PlaybackSettings}, prelude::*};
use hashbrown::HashMap;
use std::{path::PathBuf, sync::Arc};
use crate::gameplay::sound::{objects, Emitter, Request};
mod pcm;
mod graph;
mod bevy_backend;
mod fade;
mod contacts;
use crate::assets::declaration_texts;
mod effects;
mod spatial;
pub(super) mod level;
pub(super) use contacts::ContactSound;

#[derive(Message, Clone, Copy)]
pub struct SoundEvent(pub Request);

#[derive(Resource, Default)]
struct WwiseRegistry {
    banks: Arc<Vec<Bank>>,
    packages: HashMap<PathBuf, Arc<crate::assets::wwise::Package>>,
    shared_banks: usize,
    shared_attenuations: std::collections::HashSet<u32>,
    attenuations: Arc<HashMap<u32, Arc<crate::assets::wwise::Attenuation>>>,
    library: Arc<crate::assets::wwise::PlaybackBanks<'static>>,
    sources: HashMap<u32, Arc<Vec<SourceAction>>>,
    scheduled_selections: graph::Selections,
    states: bevy_backend::StateGroups,
    /// The level's listed sound events were prepared at load.
    preloaded: bool,
    level_prepared: bool,
    /// Package location of every media read, for streamed sources.
    locators: Arc<std::sync::Mutex<HashMap<u32, graph::StreamLocator>>>,
    stream_acquisition: Arc<std::sync::Mutex<(u64, u64, u64, std::time::Duration)>>,
    stream_io: Arc<crate::assets::wwise::stream_io::ReadPool>,
}

impl WwiseRegistry {
    /// Immutable bank ownership for a background event decode. The encoded
    /// banks and decoded index are shared; a level change can retire its own
    /// references while an already-running read finishes safely.
    fn decode_snapshot(&self) -> Self {
        Self { banks: self.banks.clone(), attenuations: self.attenuations.clone(),
            library: self.library.clone(), locators: Arc::default(),
            stream_acquisition: self.stream_acquisition.clone(), stream_io: self.stream_io.clone(),
            ..Default::default() }
    }
    fn playback(&self, event: u32) -> Result<Vec<crate::assets::wwise::PlaybackCommand>, Box<dyn std::error::Error>> {
        let mut files = HashMap::<usize, std::fs::File>::new();
        self.library.event_playback_shared(event, |owner, media_id, stream_type| {
            let bank = self.banks.get(owner).ok_or_else(|| std::io::Error::new(
                std::io::ErrorKind::InvalidData, "Wwise source bank ownership is absent"))?;
            let Some(media) = bank.package.media_file(media_id as u64, bank.entry.language_id) else { return Ok(None); };
            self.locators.lock().unwrap_or_else(|e| e.into_inner()).insert(media_id, graph::StreamLocator {
                path: bank.path.clone(), offset: media.offset,
                size: usize::try_from(media.size).map_err(|_| std::io::Error::other("Wwise media too large"))?,
                pool: self.stream_io.clone(),
            });
            if stream_type == 1 && crate::assets::wwise::stream_io::enabled() {
                // The bank owns the prefetch bytes. Publish the package extent
                // without reading the full file or duplicating that prefix.
                return Ok(Some(crate::assets::wwise::MediaBytes::stream_header(Vec::new(), media.size as usize)));
            }
            let file = match files.entry(owner) {
                hashbrown::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                hashbrown::hash_map::Entry::Vacant(entry) => entry.insert(std::fs::File::open(&bank.path)?),
            };
            if stream_type == 2 {
                use std::io::{Seek, SeekFrom};
                let started = std::time::Instant::now();
                let size = usize::try_from(media.size).map_err(|_| std::io::Error::other("Wwise stream too large"))?;
                let header_only = std::env::var("DH2_DEBUG_EAGER_STREAM_ACQUISITION").as_deref() != Ok("1")
                    && std::env::var("DH2_DEBUG_RESIDENT_STREAMS").as_deref() != Ok("1");
                let acquired = if header_only {
                    file.seek(SeekFrom::Start(media.offset))?;
                    let header = crate::assets::wwise::read_stream_header(file, size)?;
                    if crate::assets::wwise::stream_format(&header, size)?.is_some() {
                        crate::assets::wwise::MediaBytes::stream_header(header, size)
                    } else { bank.package.read_entry(file, media)?.into() }
                } else { bank.package.read_entry(file, media)?.into() };
                let mut totals = self.stream_acquisition.lock().unwrap_or_else(|e| e.into_inner());
                totals.0 += 1;
                totals.1 += acquired.len() as u64;
                totals.2 += size as u64;
                totals.3 += started.elapsed();
                return Ok(Some(acquired));
            }
            bank.package.read_entry(file, media).map(|bytes| Some(bytes.into()))
        })?.ok_or_else(|| "event has no supported Wwise actions".into())
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
        let graph_actions = {
            let _tag = crate::platform::allocation_tags::tag("sound bank lookup");
            self.playback(event)?
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
                definition: Arc::new(graph::Definition::decode(node, &self.attenuations, &|id| {
                    // Diagnostic A/B: DH2_DEBUG_RESIDENT_STREAMS=1 keeps streamed sources resident.
                    if std::env::var_os("DH2_DEBUG_RESIDENT_STREAMS").is_some_and(|v| v == "1") { return None; }
                    self.locators.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned()
                })?) }),
            } })();
            match decoded {
                Ok(action) => actions.push(action),
                Err(error) => warn!("Wwise event {event:#010x} action unavailable: {error}"),
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
impl Plugin for AudioAssetsPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<graph::ScheduledAudio>()
            .add_audio_source::<bevy_backend::OutputAudio>();
    }
}

#[derive(Resource, Default)]
struct PendingSounds(Vec<PendingSound>);

struct PendingSound {
    request: Request,
    // A retry must not release its already completed reads while another
    // source is pending; the source's own resident lookup is deliberately weak.
    media: Vec<graph::StreamHold>,
}

pub(super) fn begin_level(world: &mut World) {
    world.resource::<bevy_backend::Engine>().release_level();
    let mut registry = world.resource_mut::<WwiseRegistry>();
    registry.sources.clear();
    registry.locators.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
    graph::retain_media(&sound_ids);
    world.insert_resource(PendingSounds::default());
    world.insert_resource(spatial::Scene::default());
    world.insert_resource(effects::EffectAudio::default());
    if let Some(contacts) = world.get_resource::<contacts::ContactSounds>() {
        contacts.release_level_history();
    }
    world.insert_resource(contacts::ContactSounds::default());
    world.resource_mut::<Messages<SoundEvent>>().clear();
    world.resource_mut::<Messages<ContactSound>>().clear();
}

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<AudioAssetsPlugin>() { app.add_plugins(AudioAssetsPlugin); }
        crate::loading::gate(LISTED_SOUNDS_GATE);
        crate::loading::gate(LEVEL_SOUNDS_GATE);
        app.add_message::<SoundEvent>()
            .add_message::<ContactSound>()
            .init_resource::<contacts::ContactSounds>()
            .init_resource::<objects::EffectObjects>()
            .init_resource::<effects::EffectAudio>()
            .init_resource::<WwiseRegistry>()
            .init_resource::<PendingSounds>()
            .init_resource::<AudioMix>()
            .init_resource::<bevy_backend::Engine>()
            .init_resource::<spatial::Scene>()
            .add_systems(Startup, (contacts::initialize, index_banks, preload_level_events).chain())
            .add_systems(Update, contacts::update.after(super::player::update_player))
            .add_systems(PostUpdate, (bevy_backend::retire, spatial::update, prepare_level_sounds, level::update, effects::update, update_playing_volume, play_events)
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
fn level_banks(base: &std::path::Path, language: &str) -> Result<LevelBanks, Box<dyn std::error::Error>> {
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
    let map = crate::gameplay::map_info::current_map().ok_or("no current map for its sound banks")?;
    let map = map.trim_start_matches("maps/").trim_end_matches(".map").to_owned();
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
    mut engine: ResMut<bevy_backend::Engine>, mut outputs: ResMut<Assets<bevy_backend::OutputAudio>>, mut commands: Commands) {
    if registry.preloaded { return; }
    let mut commands = super::setup_commands::SetupCommands::Direct(commands);
    prepare_banks(&mut registry, &mut mix, &mut engine, &mut outputs, &mut commands);
}

fn prepare_banks(registry: &mut WwiseRegistry, mix: &mut AudioMix,
    engine: &mut bevy_backend::Engine,
    outputs: &mut dyn crate::rendering::bevy_backend::asset_preparation::AssetWrite<bevy_backend::OutputAudio>,
    commands: &mut super::setup_commands::SetupCommands) {
    let result = (|| -> Result<_, Box<dyn std::error::Error>> {
        let base = crate::assets::installation::game_base()?;
        let audio_language = crate::assets::lang::read_audio_language(&base)?;
        let package_language = crate::assets::wwise::language_name(audio_language).ok_or("unsupported audio package language")?;
        // 1400bd510 (LoadSharedCookedGamePackages): the "common", "init" and
        // current-map entries of wwisedescription.txt name the packages
        // (prefix + language / "sfx.pck") and banks to load, plus main-init.pck.
        let selection = level_banks(&base, package_language)?;
        let paths = selection.packages.iter().map(|name| base.join("pck").join(name))
            .filter(|path| path.is_file()).collect::<Vec<_>>();

        let mut output_mix = None;
        let before = registry.banks.len();
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
                    let bytes = package.read_entry(&mut file, entry)?;
                    Ok((crate::assets::wwise::bank_mix(&bytes)?, crate::assets::wwise::bank_attenuations(&bytes)?, bytes))
                })();
                match indexed {
                    Ok((bank_mix, bank_attenuations, bytes)) => {
                        for attenuation in bank_attenuations { Arc::make_mut(&mut registry.attenuations).entry(attenuation.id).or_insert_with(|| Arc::new(attenuation)); }
                        if bank_mix.is_some() { output_mix = bank_mix; }
                        Arc::make_mut(&mut registry.library).insert(bytes.into())?;
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
        let catalog = crate::archive::Catalog::shared(&base)?;
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
        Ok(Some((output_mix.ok_or("Wwise initialization mix is absent")?, bindings, time_dilation, physics_scale)))
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
            if let Some(mixer) = &mix.mixer {
                match bevy_backend::Engine::new(mixer, registry.states.clone()) {
                    Ok((initialized, output)) => {
                        if let Some(mixer) = &mut mix.mixer { mixer.activity = initialized.activity(); }
                        *engine = initialized;
                        commands.spawn((AudioPlayer(outputs.add(output)), PlaybackSettings::ONCE, super::level_lifetime::SessionEntity));
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
fn preload_level_events(mut registry: ResMut<WwiseRegistry>, contacts: Option<Res<contacts::ContactSounds>>) {
    if !registry.preloaded { prepare_listed_events(&mut registry, None, contacts.as_deref().and_then(|c| c.declarations())); }
}
fn prepare_listed_events(registry: &mut WwiseRegistry, acquisition_pool: Option<&bevy::tasks::TaskPool>, declarations: Option<&declaration_texts::DeclarationTexts>) {
    let started = std::time::Instant::now();
    let result = (|| -> Result<Vec<u32>, String> {
        let base = crate::assets::installation::game_base().map_err(|e| e.to_string())?;
        let catalog = crate::archive::Catalog::shared(&base).map_err(|e| e.to_string())?;
        let map = crate::gameplay::map_info::current_map().ok_or("no current map")?;
        let map = map.trim_start_matches("maps/").trim_end_matches(".map");
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
            match asset.resolve_with(|path| {
                if let Some(value) = declarations.filter(|_| reuse).and_then(|texts| texts.get(path)) {
                    if trace { hits += 1; }
                    return Ok(Some(value.to_owned()));
                }
                if trace { reads += 1; }
                let bytes = catalog.read(path).map_err(|error| format!("{path}: {error}"))?;
                if trace { decoded_bytes += bytes.as_ref().map_or(0, Vec::len); }
                Ok(bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
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
    super::load_timing::mark(&format!("sound events: {count} of {} listed prepared in {:.1}s", events.len(), started.elapsed().as_secs_f64()));
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
    mut registry: ResMut<WwiseRegistry>)
{
    if registry.level_prepared { return; }
    let (Some(mut physics), Some(sources)) = (physics, sources) else { return };
    let mut events: Vec<u32> = sources.0.sound_events().collect();
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
    super::load_timing::mark(&format!("level sounds: {count} more of {} emitter and FX events prepared in {:.1}s", events.len(), started.elapsed().as_secs_f64()));
}

fn play_events(
    mut commands: Commands,
    mut events: MessageReader<SoundEvent>,
    mut registry: ResMut<WwiseRegistry>,
    mut scheduled_sources: ResMut<Assets<graph::ScheduledAudio>>,
    mix: Res<AudioMix>,
    scene: Res<spatial::Scene>,
    engine: Res<bevy_backend::Engine>,
    mut pending: ResMut<PendingSounds>,
) {
    let Some(mixer) = &mix.mixer else { events.clear(); return };
    // A request whose streamed media is still being read waits, keeping its
    // place ahead of later requests for the same emitter.
    let requests: Vec<PendingSound> = std::mem::take(&mut pending.0).into_iter()
        .chain(events.read().map(|message| PendingSound { request: message.0, media: Vec::new() })).collect();
    let mut waiting = Vec::new();
    for queued in requests {
        let request = queued.request;
        let (event, emitter, parameter, position) = match request {
            Request::Post { event, emitter } => (event, emitter, None, None),
            Request::PostWithParameter { event, emitter, parameter, value } => (event, emitter, Some((parameter, value)), None),
            Request::PostAt { event, emitter, parameter, position, front } =>
                (event, emitter, parameter, Some(spatial::Position { position, front })),
            Request::Stop { event, emitter } => {
                if waiting.contains(&emitter) { pending.0.push(queued); continue; }
                engine.stop_event(event, emitter);
                continue;
            }
        };
        if waiting.contains(&emitter) { pending.0.push(queued); continue; }
        // 1408abb10 pauses components on a zero owner clock; 140709110
        // forbids posting into a frozen component. Stop requests above still run.
        if scene.frozen(emitter) { continue; }
        if let Err(error) = registry.prepare(event) {
            warn!("Wwise event {event:#010x} has no supported route: {error}");
            continue;
        }
        if let Some(actions) = registry.sources.get(&event).cloned() {
            let mut holds = Vec::new();
            let mut loading = false;
            if graph::preload_event_streams() {
                for action in actions.iter() {
                    let SourceAction::Play { definition, .. } = action else { continue };
                    for source in definition.streamed() {
                        match source.acquire() {
                            Ok(hold) => { loading |= !hold.is_ready(); holds.push(hold); },
                            Err(error) => warn!("Wwise event {event:#010x} streamed media unavailable: {error}"),
                        }
                    }
                }
            }
            // All previously ready sources now have replacement holds. Keep
            // the old ones alive through acquisition, then transfer ownership.
            drop(queued.media);
            if loading {
                waiting.push(emitter);
                pending.0.push(PendingSound { request, media: holds });
                continue;
            }
            let playing_id = engine.playing_id();
            for action in actions.iter().cloned() {
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
                                        Ok(voice) => { commands.spawn((voice, audio_target, scheduled, StreamedMedia(holds.clone()))); },
                                        Err(error) => { error!("Wwise event {event:#010x} shared output unavailable: {error}"); continue; },
                                    }
                                } else {
                                    match engine.register_fallback(scheduled.clone(), event, emitter) {
                                        Ok(control) => { commands.spawn((AudioPlayer(scheduled_sources.add(source)), PlaybackSettings::DESPAWN,
                                            audio_target, scheduled, control, StreamedMedia(holds.clone()))); },
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
}
impl AudioPreparation {
    pub fn take(world: &mut World) -> Self {
        Self {
            registry: std::mem::take(&mut *world.resource_mut::<WwiseRegistry>()),
            mix: std::mem::take(&mut *world.resource_mut::<AudioMix>()),
            engine: std::mem::take(&mut *world.resource_mut::<bevy_backend::Engine>()),
            outputs: crate::rendering::bevy_backend::asset_preparation::AssetBatch::new(world.resource::<Assets<bevy_backend::OutputAudio>>()),
        }
    }
    pub fn prepare(&mut self, commands: &mut super::setup_commands::SetupCommands,
        seed: declaration_texts::PhysicsSeed) {
        let contacts = contacts::ContactSounds::acquire_with_seed(seed);
        prepare_banks(&mut self.registry, &mut self.mix, &mut self.engine, &mut self.outputs, commands);
        // Host: cold media reads/decodes must not occupy the frame executor.
        // Keep its existing parallelism rather than the smaller async pool,
        // and release these workers before scene preparation continues.
        let pool = (std::env::var("DH2_DEBUG_SOUND_FRAME_POOL").as_deref() != Ok("1"))
            .then(|| bevy::tasks::TaskPoolBuilder::new()
                .num_threads(bevy::tasks::ComputeTaskPool::get().thread_num().max(1))
                .thread_name("sound acquisition".to_owned())
                .build());
        prepare_listed_events(&mut self.registry, pool.as_ref(), contacts.declarations());
        commands.insert_resource(contacts);
    }
    pub fn publish(self, world: &mut World) {
        world.insert_resource(self.registry);
        world.insert_resource(self.mix);
        world.insert_resource(self.engine);
        self.outputs.publish(&mut world.resource_mut::<Assets<bevy_backend::OutputAudio>>());
    }
}
