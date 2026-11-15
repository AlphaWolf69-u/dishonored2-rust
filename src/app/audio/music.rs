//! Live worldspawn music/ambiance owner (1406637e0, 1406647d0, 1406641e0).
//! Host: player callbacks enter the frame's Bevy message stream; this adapter
//! owns no bank, source bytes, decoder cache, output device or second clock.
use super::*;
use crate::{entities::{Document, Value}, gameplay::{music::{self as logic, Definition, Event, Manager, Notification},
    entity_lifetime::{EntityId, LiveEntities}}};

/// The source manager's registered player/sensor callbacks. Entity ancestry
/// is the producer's resolved EntityDef chain, never an NPC-name heuristic.
#[derive(Message)]
pub(in crate::app) enum MusicEvent {
    Entity { event: u16, entity: EntityId, ancestry: Arc<[Box<str>]> },
    Stress(bool),
    Stinger(usize),
    Suspense(bool),
    Ambiance(Option<Arc<Event>>),
}

#[derive(Resource)]
pub(in crate::app) struct Music {
    definition: Arc<Definition>,
    owner: String,
    manager: Option<Manager<EntityId>>,
    emitter: Option<Emitter>,
    suspense: bool,
    ancestry: std::collections::HashMap<EntityId, Arc<[Box<str>]>>,
    receiving_requests: bool,
    receiving_player: bool,
    cursor: bevy::ecs::message::MessageCursor<MusicEvent>,
    ambiance_environment: Option<usize>,
}
impl Music {
    pub fn load(catalog: &crate::archive::Catalog, document: &Document) -> Result<Option<Self>, String> {
        fn find<'a>(fields: &'a [(String, Value)]) -> Option<&'a str> {
            for (key, value) in fields {
                if key == "m_musicAndAmbianceManagerDecl" {
                    if let Value::Atom(value) = value { return Some(crate::gameplay::options::decl::unquote(value)); }
                }
                if let Value::Scope(fields) = value { if let Some(found) = find(fields) { return Some(found); } }
            }
            None
        }
        let Some((owner, name)) = document.definitions.iter().find_map(|definition|
            find(&definition.fields).map(|name| (definition.id.clone(), name))) else { return Ok(None); };
        if name.is_empty() || name.eq_ignore_ascii_case("NULL") { return Ok(None); }
        let definition = Definition::load(name, |path| catalog.read(path)
            .map(|bytes| bytes.map(crate::entities::owned_declaration_text)).map_err(|error| error.to_string()))?;
        // Null start/stop leaves the manager absent in 1406637e0.
        if definition.start_music.is_none() || definition.stop_music.is_none() { return Ok(None); }
        Ok(Some(Self { definition: Arc::new(definition), owner, manager: None, emitter: None,
            suspense: false, ancestry: Default::default(), receiving_requests: false, receiving_player: false, cursor: Default::default(), ambiance_environment: None }))
    }
    pub(super) fn events(&self) -> impl Iterator<Item = u32> + '_ {
        self.definition.start_music.iter().chain(self.definition.stop_music.iter())
            .chain(self.definition.stages.iter().flatten()).chain(self.definition.start_ambiance.iter())
            .chain(self.definition.stop_ambiance.iter()).chain(self.definition.stingers.iter().filter_map(|stinger| stinger.event.as_ref()))
            .map(|event| event.id)
    }
    /// Synchronous request handles remain synchronous (140662fe0); callers
    /// must not substitute delayed event responses for the returned handle.
    pub fn add_request(&mut self, stage: i32, duration: f32, lifetime: i32, notify: bool) -> i32 {
        if !self.receiving_requests { return -1; }
        self.manager.as_mut().map_or(-1, |manager| manager.add_request(stage, duration, lifetime, notify))
    }
}
struct Host<'a, 'w> {
    emitter: Emitter,
    worldspawn: Option<EntityId>,
    events: &'a mut MessageWriter<'w, SoundEvent>,
    entity_events: Option<&'a mut crate::gameplay::entity_events::EntityEvents>,
    ancestry: &'a std::collections::HashMap<EntityId, Arc<[Box<str>]>>,
    receiving_requests: &'a mut bool,
    receiving_player: &'a mut bool,
}
impl logic::Host<EntityId> for Host<'_, '_> {
    fn post(&mut self, event: &Event) -> bool {
        if std::env::var("DH2_DEBUG_MUSIC").as_deref() == Ok("1") {
            eprintln!("MUSIC_EVENT id={:08x} declaration={} emitter={:?}", event.id, event.declaration, self.emitter);
        }
        self.events.write(SoundEvent(Request::Post { event: event.id, emitter: self.emitter }));
        true
    }
    fn request_receiver(&mut self, enabled: bool) { *self.receiving_requests = enabled; }
    fn player_receivers(&mut self, enabled: bool) { *self.receiving_player = enabled; }
    fn notify(&mut self, notification: Notification) {
        let (handle, operation) = match notification {
            Notification::Current(handle) => (handle, 0i32),
            Notification::Superseded(handle) => (handle, 1),
            Notification::Removed(handle) => (handle, 2),
        };
        if let (Some(worldspawn), Some(events)) = (self.worldspawn, self.entity_events.as_deref_mut()) {
            events.post(crate::gameplay::entity_events::Event { id: 16, target: Some(worldspawn),
                payload: crate::gameplay::entity_events::Payload::MusicRequest { handle, operation } });
        }
    }
    fn entity_inherits(&self, entity: EntityId, declaration: &str) -> Result<bool, String> {
        let ancestry = self.ancestry.get(&entity).ok_or("music entity has no resolved EntityDef ancestry")?;
        Ok(ancestry.iter().any(|name| name.eq_ignore_ascii_case(declaration)))
    }
}

pub(super) fn update(mut music: Option<ResMut<Music>>, mut physics: Option<ResMut<super::super::physics_scene::PhysicsScene>>,
    player: Option<Res<super::super::player::PlayerState>>, live: Option<Res<LiveEntities>>,
    mut entity_events: Option<ResMut<crate::gameplay::entity_events::EntityEvents>>,
    inputs: Res<Messages<MusicEvent>>, mut events: MessageWriter<SoundEvent>,
    water: Option<Res<super::super::water_scene::WaterScene>>,
    volumes: Option<Res<super::super::state::EnvironmentVolumes>>,
    clock: Res<crate::gameplay::game_clock_runtime::ClockRuntime>) {
    let Some(music) = music.as_deref_mut() else { return; };
    let Some(physics) = physics.as_deref_mut() else { return; };
    // This client admits a normal gameplay world (the source AllocGame flag
    // 0x20 also gates difficulty/Kiscule); it has no editor/preview world mode.
    if !player.as_ref().is_some_and(|player| player.player.is_some()) { return; }
    if music.emitter.is_none() {
        let owner = &music.owner;
        if let Err(error) = physics.visit_audio(|handle, binding, component| {
            if binding.entity_id == *owner && component.started {
                component.allocated = true;
                music.emitter = Some(Emitter::Component { handle: handle.packed(), generation: component.owner.generation });
            }
            Ok(())
        }) { error!("Music worldspawn audio admission: {error}"); return; }
    }
    let Some(emitter) = music.emitter else { return; };
    let Emitter::Component { handle, generation } = emitter else { return; };
    let Some(timespace) = physics.audio_owner_timespace(handle, generation) else { return; };
    let worldspawn = live.as_ref().and_then(|live| live.find(&music.owner));
    let selected_ambiance = water.as_ref().and_then(|water| water.ambiance_environment());
    let changed_ambiance = selected_ambiance.filter(|index| music.ambiance_environment != Some(*index));
    music.ambiance_environment = selected_ambiance;
    let Music { definition, manager, ancestry, receiving_requests, receiving_player, cursor, suspense, .. } = music;
    let mut ancestry_cursor = cursor.clone();
    for input in ancestry_cursor.read(&inputs) {
        if let MusicEvent::Entity { entity, ancestry: chain, .. } = input { ancestry.insert(*entity, chain.clone()); }
    }
    let mut host = Host { emitter, worldspawn, events: &mut events,
        entity_events: entity_events.as_deref_mut(), ancestry, receiving_requests, receiving_player };
    if manager.is_none() { *manager = Manager::start(definition.clone(), &mut host); }
    let Some(manager) = manager else { return; };
    if let Some(index) = changed_ambiance {
        // A missing selection leaves the previous event alone (140806a40).
        manager.set_ambiance(volumes.as_ref().and_then(|volumes| volumes.0.get(index)).and_then(|volume| volume.ambiance.clone()));
    }
    // Inputs carry source callbacks, not guesses derived from movement,
    // camera visibility or nearby prop names. No NPC means no combat callback.
    for input in cursor.read(&inputs) {
        match input {
            MusicEvent::Entity { event, entity, .. } => {
                if let Err(error) = manager.entity_event(*event, *entity, &mut host) { error!("Music player event {event:#x}: {error}"); }
            },
            MusicEvent::Stress(enabled) => manager.stress(*enabled, &mut host),
            MusicEvent::Stinger(level) => {
                let seconds = crate::platform::performance_counter::counter().and_then(|counter|
                    crate::platform::performance_counter::frequency().map(|frequency| counter as f64 / frequency as f64));
                match seconds.map_err(|error| error.to_string()).and_then(|seconds| manager.stinger(*level, seconds, &mut host)) {
                    Ok(()) => {}, Err(error) => error!("Music stinger: {error}"),
                }
            },
            MusicEvent::Suspense(enabled) => *suspense = *enabled,
            MusicEvent::Ambiance(event) => manager.set_ambiance(event.clone()),
        }
    }
    // 1408afccf updates this manager in the active-world mode; its world
    // clock supplies zero during pause. Do not suppress handle/state changes
    // merely because elapsed simulation time is zero.
    match clock.clock().and_then(|clock| clock.entity_delta(timespace).ok_or("worldspawn music clock is absent")) {
        Ok(dt) => manager.update(dt, *suspense, &mut host), Err(error) => error!("Music update clock: {error}"),
    }
    drop(host);
    ancestry.retain(|entity, _| manager.references_entity(*entity));
}

/// Run before component/voice retirement. StopAmbiance is submitted through
/// the same SoundEvent consumer; audio retirement then drains the actual
/// worldspawn voices/read jobs before their bank suffix can be unloaded.
pub(super) fn stop(world: &mut World) -> Result<(), String> {
    let Some(mut music) = world.remove_resource::<Music>() else { return Ok(()); };
    if let (Some(manager), Some(emitter)) = (music.manager.take(), music.emitter) {
        world.resource_scope(|world, mut messages: Mut<Messages<SoundEvent>>| {
            struct StopHost<'a> { messages: &'a mut Messages<SoundEvent>, emitter: Emitter }
            impl logic::Host<EntityId> for StopHost<'_> {
                fn post(&mut self, event: &Event) -> bool { self.messages.write(SoundEvent(Request::Post { event: event.id, emitter: self.emitter })); true }
                fn request_receiver(&mut self, _: bool) {}
                fn player_receivers(&mut self, _: bool) {}
                fn notify(&mut self, _: Notification) {}
                fn entity_inherits(&self, _: EntityId, _: &str) -> Result<bool, String> { Err("music owner is stopping".into()) }
            }
            manager.stop(&mut StopHost { messages: &mut messages, emitter });
            let _ = world;
        });
    }
    world.resource_mut::<Messages<MusicEvent>>().clear();
    Ok(())
}

pub(in crate::app) fn observe_live_owner(world: &World) -> Result<(), String> {
    let music = world.get_resource::<Music>().ok_or("level has no music declaration")?;
    if music.manager.is_none() || music.emitter.is_none() { return Err("level music owner did not start".into()); }
    eprintln!("MUSIC_OWNER_LIFETIME owner={} emitter={:?} running=true retained_entities={}", music.owner, music.emitter, music.ancestry.len());
    Ok(())
}

/// Installed-data observation through the production bank, event, source and
/// music owners. No window, GPU, hardware audio, fake decoder EOF or test data.
/// Every map retires its suffix before the next map acquires its definitions.
pub(in crate::app) fn observe_resources(base: &std::path::Path) -> Result<(), String> {
    use bevy::ecs::system::RunSystemOnce;
    let _session = super::session::Session::begin().map_err(str::to_owned)?;
    let catalog = Arc::new(crate::archive::Catalog::open(base).map_err(|error| error.to_string())?);
    let mut maps = catalog.paths_with_suffix(".mapresources").into_iter().filter_map(|path|
        path.strip_prefix("generated/buildgame/").and_then(|path| path.strip_suffix(".mapresources"))
            .filter(|name| !matches!(*name, "init" | "common")).map(|name| format!("maps/{name}.entities"))).collect::<Vec<_>>();
    maps.sort();
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()));
    super::initialize_resources(&mut app);
    let mut admitted = 0;
    let mut routes = 0;
    for map in &maps {
        super::begin_level(app.world_mut());
        let bytes = catalog.read(map).map_err(|error| error.to_string())?.ok_or_else(|| format!("missing {map}"))?;
        let document = crate::entities::parse(&crate::entities::declaration_text(&bytes))?;
        let music = Music::load(&catalog, &document)?;
        drop(document);
        drop(bytes);
        let Some(music) = music else { eprintln!("MUSIC_RESOURCES map={map} manager=absent"); continue; };
        let events = music.events().collect::<Vec<_>>();
        let start = music.definition.start_music.as_ref().unwrap().id;
        let source = catalog.clone();
        let name = map.clone();
        app.world_mut().run_system_once(move |mut registry: ResMut<WwiseRegistry>, mut mix: ResMut<AudioMix>,
            mut engine: ResMut<super::bevy_backend::Engine>, mut outputs: ResMut<Assets<super::bevy_backend::OutputAudio>>, commands: Commands| {
            let mut commands = crate::app::setup_commands::SetupCommands::Direct(commands);
            super::prepare_banks(&mut registry, &mut mix, &mut engine, &mut *outputs, &mut commands, &source, &name, PlaybackBackend::Absent);
        }).map_err(|error| error.to_string())?;
        let mut registry = app.world_mut().resource_mut::<WwiseRegistry>();
        super::prepare_all(&mut registry, &events, None);
        let mut music_routes = 0;
        let mut actors = 0;
        let mut sources = 0;
        for event in &events {
            let actions = registry.sources.get(event).ok_or_else(|| format!("music event {event:08x} was not prepared in {map}"))?;
            for action in actions.iter() {
                if let SourceAction::Play { definition, .. } = action {
                    if let Some((root, actor_count, source_count)) = definition.music_inventory() {
                        music_routes += 1;
                        actors += actor_count;
                        sources += source_count;
                        eprintln!("MUSIC_RESOURCE_ROUTE map={map} event={event:08x} root={root:08x} actors={actor_count} sources={source_count} streams={}", definition.streamed().count());
                    }
                }
            }
        }
        let started = registry.sources.get(&start).is_some_and(|actions|
            actions.iter().any(|action| matches!(action, SourceAction::Play { definition, .. } if definition.music_inventory().is_some())));
        if !started { return Err(format!("{map}: declared StartMusic event {start:08x} has no music playback route")); }
        eprintln!("MUSIC_RESOURCES map={map} events={} routes={music_routes} actors={actors} sources={sources} resident_sources={:?}", events.len(), super::graph::resident_media_stats());
        admitted += 1;
        routes += music_routes;
    }
    super::begin_level(app.world_mut());
    drop(app);
    drop(_session);
    eprintln!("MUSIC_RESOURCES_RETIRED resident_sources={:?}", super::graph::resident_media_stats());
    eprintln!("MUSIC_RESOURCES_COMPLETE maps={} managers={admitted} routes={routes}", maps.len());
    Ok(())
}
