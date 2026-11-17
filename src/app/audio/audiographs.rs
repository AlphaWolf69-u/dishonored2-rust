//! Audiocard playback (140520240, 140496300/1404961c0/140496430).
//! Host: Journal speech uses its menu sound object and the existing asynchronous
//! preparation pool, event completion tickets and streamed Wwise output.
use super::*;
use crate::assets::{archive::Catalog, inventory_item::PickableItem, speech::Recording};
use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Selection {
    item: u32,
    declaration: Arc<PickableItem>,
    catalog: Arc<Catalog>,
}
struct Pending {
    job: Mutex<acquisition::Job<Result<Recording, String>>>,
    cancelled: Arc<AtomicBool>,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
struct Active {
    recording: Recording,
    line: usize,
    event: Option<u32>,
}

#[derive(Resource, Default)]
pub(in crate::app) struct Audiographs {
    requests: VecDeque<Selection>,
    player: Playback,
}
#[derive(Default)]
struct Playback {
    selected: Option<u32>,
    pending: Option<Pending>,
    active: Option<Active>,
    retiring: Vec<Pending>,
}
impl Audiographs {
    pub(in crate::app) fn playing(&self) -> Option<u32> {
        self.player.selected
    }
    pub(in crate::app) fn request(
        &mut self,
        item: u32,
        declaration: Arc<PickableItem>,
        catalog: Arc<Catalog>,
    ) -> Result<(), String> {
        if declaration.audiograph_card.is_none() {
            return Err("item has no authored audiograph card".into());
        }
        self.requests.push_back(Selection {
            item,
            declaration,
            catalog,
        });
        Ok(())
    }
}
impl Playback {
    fn stop(
        &mut self,
        emitter: Emitter,
        completions: &mut SoundCompletions,
        events: &mut MessageWriter<SoundEvent>,
    ) {
        if let Some(pending) = self.pending.take() {
            pending.cancelled.store(true, Ordering::Release);
            self.retiring.push(pending);
        }
        if let Some(active) = self.active.take() {
            if let Some(event) = active.event {
                events.write(SoundEvent(Request::Stop { event, emitter }));
                completions.cancel(event, emitter);
            }
        }
        self.selected = None;
    }
}

impl Playback {
    fn start(&mut self, selection: Selection, jobs: &acquisition::SourceJobs) {
        let work = match session::work() {
            Ok(work) => work,
            Err(error) => {
                warn!("Audiograph: {error}");
                return;
            }
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        self.selected = Some(selection.item);
        let corvo = crate::app::player::character_name(crate::app::pause_menu::debug_character())
            == "corvo";
        let job = jobs.0.submit("audiograph", selection.item, move || {
            let _work = work;
            if _work.is_closing() || cancellation.load(Ordering::Acquire) {
                return Err("audiograph acquisition cancelled".into());
            }
            let card = selection
                .declaration
                .audiograph_card
                .as_deref()
                .ok_or("audiograph card missing")?;
            let recording = crate::assets::speech::recording(&selection.catalog, card, corvo)?;
            if _work.is_closing() || cancellation.load(Ordering::Acquire) {
                return Err("audiograph acquisition cancelled".into());
            }
            Ok(recording)
        });
        self.pending = Some(Pending {
            job: Mutex::new(job),
            cancelled,
        });
    }
    fn poll_jobs(&mut self) -> bool {
        self.retiring.retain_mut(|pending| {
            pending
                .job
                .get_mut()
                .unwrap_or_else(|error| error.into_inner())
                .take()
                .is_none()
        });
        self.retiring.is_empty()
    }
    fn advance(
        &mut self,
        emitter: Emitter,
        position: Option<super::spatial::Position>,
        completions: &mut SoundCompletions,
        events: &mut MessageWriter<SoundEvent>,
    ) {
        self.poll_jobs();
        let ready = self.pending.as_mut().and_then(|pending| {
            pending
                .job
                .get_mut()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        });
        if let Some(result) = ready {
            self.pending = None;
            match result {
                Ok(recording) => {
                    self.active = Some(Active {
                        recording,
                        line: 0,
                        event: None,
                    })
                }
                Err(error) => {
                    self.selected = None;
                    warn!("Audiograph preparation: {error}");
                }
            }
        }
        let Some(active) = self.active.as_mut() else {
            return;
        };
        if let Some(event) = active.event {
            match completions.take(event, emitter) {
                None => return,
                Some(false) => {
                    self.stop(emitter, completions, events);
                    warn!("Audiograph speech event {event:08x} failed");
                    return;
                }
                Some(true) => {
                    active.line += 1;
                    active.event = None;
                }
            }
        }
        if let Some(&event) = active.recording.events.get(active.line) {
            completions.watch(event, emitter);
            events.write(SoundEvent(if let Some(position) = position {
                Request::PostAt {
                    event,
                    emitter,
                    position: position.position,
                    front: position.front,
                    parameter: None,
                }
            } else {
                Request::Post { event, emitter }
            }));
            active.event = Some(event);
        } else {
            self.stop(emitter, completions, events);
        }
    }
}

pub(super) fn update(
    mut owner: ResMut<Audiographs>,
    jobs: Res<acquisition::SourceJobs>,
    mut devices: Option<ResMut<Devices>>,
    mut interactions: Option<ResMut<crate::app::interactions::Interactions>>,
    mut inventory: ResMut<crate::app::interaction_focus::DiscoveredDocuments>,
    live: Option<Res<crate::gameplay::entity_lifetime::LiveEntities>>,
    objects: Res<objects::EffectObjects>,
    mut entity_events: Option<ResMut<crate::gameplay::entity_events::EntityEvents>>,
    loading: Res<crate::app::loading_screen::Acquisition>,
    mut completions: ResMut<SoundCompletions>,
    mut events: MessageWriter<SoundEvent>,
    hud: Option<NonSend<crate::app::pause_menu::PauseMenu>>,
) {
    while let Some(selection) = owner.requests.pop_front() {
        let toggle_off = owner.player.selected == Some(selection.item);
        owner
            .player
            .stop(Emitter::Menu, &mut completions, &mut events);
        if !toggle_off {
            owner.player.start(selection, &jobs);
        }
    }
    owner
        .player
        .advance(Emitter::Menu, None, &mut completions, &mut events);
    let (Some(devices), Some(interactions), Some(live)) = (
        devices.as_deref_mut(),
        interactions.as_deref_mut(),
        live.as_deref(),
    ) else {
        return;
    };
    if crate::app::loading_screen::is_loading(&loading) {
        return;
    }
    for device in &mut devices.entries {
        device.player.poll_jobs();
        let actor = &mut interactions.actors[device.actor];
        let entity = live.find(&actor.entity);
        let wanted = entity.is_some()
            && actor.started
            && actor
                .state
                .as_ref()
                .is_some_and(|state| state.state == device.on);
        if !wanted {
            if let Some(emitter) = device.emitter.take() {
                device.player.stop(emitter, &mut completions, &mut events);
                events.write(SoundEvent(Request::Retire { emitter }));
            }
            device.started = false;
            continue;
        }
        if !device.started
            && actor
                .state
                .as_ref()
                .is_some_and(|state| !state.transitioning())
        {
            let emitter = match objects.allocate() {
                Ok(emitter) => emitter,
                Err(error) => {
                    error!("Audiograph object: {error}");
                    continue;
                }
            };
            device.emitter = Some(emitter);
            device.started = true;
            device.player.start(
                Selection {
                    item: device.item,
                    declaration: device.declaration.clone(),
                    catalog: interactions.catalog.clone(),
                },
                &jobs,
            );
            // 496760 admits the recording once and leaves the machine present.
            if !inventory
                .audiographs()
                .iter()
                .any(|entry| entry.item_id == device.item)
            {
                if inventory.admit_audiograph(
                    device.path.clone(),
                    device.item,
                    1,
                    device.declaration.clone(),
                ) { device.pickup_log_pending = true; }
            }
        }
        //496760 directly requests6734b0 only for the recording's first
        //actual acquisition; ordinary inventory messages are not a log.
        if device.pickup_log_pending {
            match hud.as_ref().map(|hud| hud.queue_pickup(device.declaration.clone(), 1)).transpose() {
                Ok(Some(())) => device.pickup_log_pending = false,
                Ok(None) => {},
                Err(error) => error!("Audiograph pickup log delivery failed: {error}"),
            }
        }
        let Some(emitter) = device.emitter else {
            continue;
        };
        device.player.advance(
            emitter,
            // 4961c0 plays through the actor's speech/audio component; retain
            // its complete emitter frame, including the cone's forward axis.
            Some(super::spatial::Position {
                position: Vec3::from_array(actor.frame.position),
                front: Some(Vec3::from_array(actor.frame.basis[0])),
            }),
            &mut completions,
            &mut events,
        );
        if device.player.selected.is_none() {
            if let Some(state) = actor.state.as_mut() {
                let change = state.set_state(device.off);
                let animation = change.animation;
                let stop_effect = change.stop_effect;
                let path = change.start_effect.map(str::to_owned);
                interactions
                    .pending
                    .push(crate::app::interactions::PendingChange {
                        actor: device.actor,
                        animation,
                        stop_effect,
                        start_effect: path
                            .as_deref()
                            .map(|path| crate::app::interactions::effect_spawn(state, path)),
                    });
            }
            events.write(SoundEvent(Request::Retire { emitter }));
            device.emitter = None;
            if let (Some(entity), Some(messages)) = (entity, entity_events.as_deref_mut()) {
                messages.post(crate::gameplay::entity_events::Event {
                    id: 0x155,
                    target: Some(entity),
                    payload: crate::gameplay::entity_events::Payload::None,
                });
            }
        }
    }
}

struct Device {
    actor: usize,
    on: usize,
    off: usize,
    item: u32,
    path: String,
    declaration: Arc<PickableItem>,
    emitter: Option<Emitter>,
    player: Playback,
    started: bool,
    pickup_log_pending: bool,
}
/// Level-scoped links to actual usables; no retained generic component trees.
#[derive(Resource, Default)]
pub(in crate::app) struct Devices {
    entries: Vec<Device>,
}
impl Devices {
    pub(in crate::app) fn load(
        catalog: &Catalog,
        document: &crate::entities::Document,
        interactions: &crate::app::interactions::Interactions,
    ) -> Result<Self, String> {
        let bindings = crate::entities::usable::audiograph_bindings(document, |path| {
            catalog
                .read(path)
                .map_err(|e| e.to_string())
                .map(|bytes| bytes.map(crate::entities::owned_declaration_text))
        })?;
        let sources = crate::assets::inventory_item::PickableSourceLease::for_catalog(catalog)?;
        let mut entries = Vec::new();
        for (entity, path, on, off) in bindings {
            let actor = *interactions
                .actor_by_entity
                .get(&entity)
                .ok_or_else(|| format!("audiograph {entity} has no usable owner"))?;
            let steps = &interactions.actors[actor].definition.steps;
            let on = steps
                .iter()
                .position(|step| step.name == on)
                .ok_or("audiograph playback state missing")?;
            let off = steps
                .iter()
                .position(|step| step.name == off)
                .ok_or("audiograph stop state missing")?;
            let declaration = sources
                .0
                .lock()
                .map_err(|_| "item owner poisoned")?
                .acquire(catalog, &path)?;
            let (name, _, _) = declaration
                .audiograph_text()
                .ok_or("audiograph collectible has wrong type")?;
            let item = if name.is_empty() {
                0
            } else {
                crate::assets::lang::key_id(name)
            };
            entries.push(Device {
                actor,
                on,
                off,
                item,
                path,
                declaration,
                emitter: None,
                player: Playback::default(),
                started: false,
                pickup_log_pending: false,
            });
        }
        eprintln!("AUDIOGRAPH_DEVICES admitted={}", entries.len());
        Ok(Self { entries })
    }
}

pub(super) fn stop(world: &mut World) -> Result<(), String> {
    world
        .run_system_cached(
            |mut owner: ResMut<Audiographs>,
             mut devices: Option<ResMut<Devices>>,
             mut completions: ResMut<SoundCompletions>,
             mut events: MessageWriter<SoundEvent>| {
                owner.requests.clear();
                owner
                    .player
                    .stop(Emitter::Menu, &mut completions, &mut events);
                if let Some(devices) = devices.as_deref_mut() {
                    for device in &mut devices.entries {
                        if let Some(emitter) = device.emitter.take() {
                            device.player.stop(emitter, &mut completions, &mut events);
                            events.write(SoundEvent(Request::Retire { emitter }));
                        }
                    }
                }
            },
        )
        .map_err(|error| error.to_string())
}

/// Await actual cancelled metadata work before purging its declaration marker.
pub(super) fn drained(world: &mut World) -> Result<bool, String> {
    world
        .run_system_cached(
            |mut owner: ResMut<Audiographs>, mut devices: Option<ResMut<Devices>>| {
                let mut complete = owner.player.poll_jobs();
                if let Some(devices) = devices.as_deref_mut() {
                    for device in &mut devices.entries {
                        complete &= device.player.poll_jobs();
                    }
                }
                complete
            },
        )
        .map_err(|error| error.to_string())
}

/// Windowless acquisition through the actual map bank and recording owners.
/// No playback, readiness, completion event or resource lifetime is simulated.
pub fn inspect_audiograph_resources(base: &std::path::Path, args: &[String]) -> Result<(), String> {
    use bevy::ecs::system::RunSystemOnce;
    let _session = session::Session::begin().map_err(str::to_owned)?;
    let catalog = Arc::new(Catalog::open(base).map_err(|error| error.to_string())?);
    let map = args
        .first()
        .cloned()
        .unwrap_or_else(|| "maps/campaign/edge_of_the_world/eotw_p_lowchaos.entities".into());
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, bevy::asset::AssetPlugin::default()));
    super::initialize_resources(&mut app);
    let source = catalog.clone();
    let selected = map.clone();
    app.world_mut()
        .run_system_once(
            move |mut registry: ResMut<WwiseRegistry>,
                  mut mix: ResMut<AudioMix>,
                  mut engine: ResMut<bevy_backend::Engine>,
                  mut outputs: ResMut<Assets<bevy_backend::OutputAudio>>,
                  commands: Commands| {
                let mut commands = crate::app::setup_commands::SetupCommands::Direct(commands);
                super::prepare_banks(
                    &mut registry,
                    &mut mix,
                    &mut engine,
                    &mut *outputs,
                    &mut commands,
                    &source,
                    &selected,
                    PlaybackBackend::Absent,
                );
            },
        )
        .map_err(|error| error.to_string())?;
    let events = app
        .world()
        .resource::<WwiseRegistry>()
        .library
        .event_ids()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let sources = crate::assets::inventory_item::PickableSourceLease::for_catalog(&catalog)?;
    let mut recordings = 0;
    let mut admitted = 0;
    let mut lines = 0;
    for path in catalog
        .paths_containing("/audiographs/")
        .into_iter()
        .filter(|path| {
            path.starts_with("generated/decls/inventoryitem/") && path.ends_with(".decl")
        })
    {
        let name = path
            .strip_prefix("generated/decls/inventoryitem/")
            .unwrap()
            .strip_suffix(".decl")
            .unwrap();
        let declaration = sources
            .0
            .lock()
            .map_err(|_| "item owner poisoned")?
            .acquire(&catalog, name)?;
        let Some(card) = declaration.audiograph_card.as_deref() else {
            continue;
        };
        let recording = crate::assets::speech::recording(
            &catalog,
            card,
            crate::app::player::character_name(crate::app::pause_menu::debug_character())
                == "corvo",
        )?;
        let available = recording
            .events
            .iter()
            .filter(|event| events.contains(event))
            .count();
        recordings += 1;
        if available != 0 {
            admitted += 1;
            lines += available;
            println!(
                "AUDIOGRAPH_RESOURCE path={name} card={card} lines={} available={available}",
                recording.events.len()
            );
            if available != recording.events.len() {
                return Err(format!("audiograph {name} has incomplete bank events"));
            }
        }
    }
    println!(
        "AUDIOGRAPH_RESOURCES map={map} recordings={recordings} admitted={admitted} lines={lines}"
    );
    drop(sources);
    super::begin_level(app.world_mut());
    drop(app);
    catalog.release_level_resources();
    if admitted == 0 {
        return Err("no audiograph speech event matched the installed map banks".into());
    }
    Ok(())
}
