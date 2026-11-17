//! Level-scoped acquisition owner for effect dependencies.
//!
//! Setup, screen effects and world effects all use this same cache. Runtime
//! requests borrow it only on the IO worker; they never decode assets on the
//! gameplay frame. Dropping the level releases the owner's reference.
use bevy::{prelude::Resource, tasks::{IoTaskPool, Task}};
use std::{collections::HashMap, sync::{Arc, Mutex, Weak, atomic::{AtomicBool, Ordering}}};

#[derive(Resource, Clone)]
pub(super) struct ParticleResourceOwner(
    pub(super) Arc<Mutex<crate::rendering::particles::definition::ParticleResources>>,
);

/// Request-scoped world-material PMODEL preparation. The shared material owner
/// publishes its ordinary scene assets/proxies and acknowledges a generation
/// only after they are ready; weak source leases permit stopped tails to finish.
#[derive(Resource, Clone)]
pub(super) struct DynamicWorldMaterials {
    pub generation: u64,
    pub retirement: u64,
    pub sources: Arc<Sources>,
    /// Per-request sources are also retained by the render-side FX entity and
    /// its instance until every particle, sound and decal tail has retired.
    pub leases: Arc<Vec<Weak<Sources>>>,
    pub gpu_completed: Arc<std::sync::atomic::AtomicU64>,
    pub completed: Arc<std::sync::atomic::AtomicU64>,
    pub failed: Arc<std::sync::atomic::AtomicU64>,
    /// Ordered world-material dependencies, retained for live proxies and tails.
    pub images: HashMap<String, Vec<(crate::rendering::bevy_backend::surface_gpu::ImageRole, Arc<crate::rendering::particles::image_resources::ParticleImage>)>>,
    pub extra_images: Vec<Arc<crate::rendering::particles::image_resources::ParticleImage>>,
    pub image_revision: u64,
    pub images_ready: u64,
}
impl Default for DynamicWorldMaterials {
    fn default() -> Self {
        Self { generation: 0, retirement: 0, sources: Arc::default(), leases: Arc::default(), gpu_completed: Arc::default(),
            completed: Arc::default(), failed: Arc::default(), images: HashMap::new(), extra_images: Vec::new(),
            image_revision: 0, images_ready: 0 }
    }
}

/// Level-scoped request for the ordinary projected-decal owner. The publisher
/// acknowledges only after material variants, their images and receiver proxy
/// pipelines are usable. Existing effect tails keep their definitions alive.
#[derive(Resource, Clone, Default)]
pub(super) struct DynamicDecalMaterials {
    pub generation: u64,
    pub retirement: u64,
    pub sources: Arc<Sources>,
    pub leases: Arc<Vec<Weak<Sources>>>,
    pub prepared_generation: Option<u64>,
    pub prepared_revision: u64,
    pub prepared: Arc<Vec<super::runtime_materials::PreparedDecal>>,
    pub gpu_completed: Arc<std::sync::atomic::AtomicU64>,
    pub completed: Arc<std::sync::atomic::AtomicU64>,
    pub failed: Arc<std::sync::atomic::AtomicU64>,
}

impl ParticleResourceOwner {
    pub(super) fn new(resources: crate::rendering::particles::definition::ParticleResources) -> Self {
        Self(Arc::new(Mutex::new(resources)))
    }
}

type Sources = crate::rendering::particles::world_sources::WorldFxSources;
type Acquisition = Task<Result<Arc<Sources>, String>>;
type Lifetime = crate::rendering::bevy_backend::world_effects::DynamicEffectLifetime;

struct PendingSource {
    task: Mutex<Acquisition>,
    cancelled: Arc<AtomicBool>,
}

struct Wanted {
    context: Option<crate::rendering::bevy_backend::world_effects::EntityEffectContext>,
    path: String,
    revision: u64,
    retirement: Arc<Lifetime>,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) enum EffectOwner {
    Usable(usize),
    Player(String),
    Entity(Arc<str>),
    Reservation(Arc<str>, String),
}

#[derive(Clone)]
pub(super) struct EntityPlacement {
    pub target: crate::gameplay::event_graph::EntityReference,
    pub tag: String,
    pub local: crate::gameplay::reference_frame::ReferenceFrame,
    pub attached: bool,
}
#[derive(Clone)]
pub(super) struct EffectTicket { lifetime: Weak<Lifetime>, path: Arc<str> }

///Borrow actual request/source/publication state only when F8 asks for it.
pub(super) fn debug_ticket(world:&bevy::prelude::World,ticket:&EffectTicket)->String {
    let lifetime=ticket.lifetime.upgrade();
    let Some(owner)=world.get_resource::<UsableEffects>() else {return format!("path={} lifetime={} owner=absent",ticket.path,lifetime.is_some());};
    let wanted=lifetime.as_ref().and_then(|lifetime|owner.wanted.iter().find(|(_,wanted)|Arc::ptr_eq(&wanted.retirement,lifetime)));
    let ready=wanted.and_then(|(key,_)|owner.ready.get(key));
    let errors=ready.into_iter().flat_map(|source|source.effects.values()).filter_map(|effect|effect.as_ref().err()).collect::<Vec<_>>();
    format!("path={} lifetime={} completed={} wanted={} loading={} source_ready={} source_errors={errors:?} revision={} published={:?} active_publication={:?}",
        ticket.path,lifetime.is_some(),effect_completed(ticket),wanted.is_some(),owner.loading.contains_key(ticket.path.as_ref()),ready.is_some(),owner.revision,owner.published_revision,owner.published_active_revision)
}

pub(super) enum EffectRequest {
    StartPlayer { owner: String, path: String },
    StopPlayer { owner: String },
    StartEntity { owner: Arc<str>, path: String, placement: EntityPlacement, initial_world: Option<crate::gameplay::reference_frame::ReferenceFrame>, retirement: Arc<Lifetime>, context: Option<crate::rendering::bevy_backend::world_effects::EntityEffectContext> },
    UpdateEntity { owner: Arc<str>, placement: EntityPlacement },
    UpdateBeam { owner: Arc<str>, origin: [f32; 3], endpoint: [f32; 3] },
    StopEntity { owner: Arc<str> },
    Reserve { owner: Arc<str>, paths: Vec<String> },
    ReleaseReservation { owner: Arc<str> },
}

/// Level-owned requests from gameplay. An owner identifies one active effect;
/// its simulation and mutable parameters never live in the shared definition.
#[derive(Resource, Default)]
pub(super) struct EffectRequests(pub(super) std::collections::VecDeque<EffectRequest>);

impl EffectRequests {
    /// 774d90/776490 write the FX origin and target position independently;
    /// the constructor's identity axes remain unchanged.
    pub(super) fn start_beam(&mut self, owner: Arc<str>, path: &str, origin: bevy_math::Vec3, endpoint: bevy_math::Vec3) -> EffectTicket {
        let frame = crate::gameplay::reference_frame::ReferenceFrame { position: origin.to_array(), ..Default::default() };
        let ticket = self.start_world(owner.clone(), path, frame);
        self.update_beam(owner, origin, endpoint); ticket
    }
    pub(super) fn update_beam(&mut self, owner: Arc<str>, origin: bevy_math::Vec3, endpoint: bevy_math::Vec3) {
        self.0.push_back(EffectRequest::UpdateBeam { owner, origin: origin.to_array(), endpoint: endpoint.to_array() });
    }
    /// Acquire immutable dependencies only. This never creates an FX actor
    /// or submits a Start to the renderer.
    pub(super) fn reserve(&mut self, owner: Arc<str>, paths: Vec<String>) {
        self.0.push_back(EffectRequest::Reserve { owner, paths });
    }
    pub(super) fn release_reservation(&mut self, owner: Arc<str>) {
        self.0.push_back(EffectRequest::ReleaseReservation { owner });
    }
    pub(super) fn start_entity(&mut self, owner: Arc<str>, path: &str, placement: EntityPlacement) -> EffectTicket {
        self.start_entity_mode(owner, path, placement, None, false)
    }
    pub(super) fn start_entity_once(&mut self, owner: Arc<str>, path: &str, placement: EntityPlacement) -> EffectTicket {
        self.start_entity_mode(owner, path, placement, None, true)
    }
    pub(super) fn start_world(&mut self, owner: Arc<str>, path: &str, frame: crate::gameplay::reference_frame::ReferenceFrame) -> EffectTicket {
        self.start_world_mode(owner, path, frame, false)
    }
    pub(super) fn start_world_once(&mut self, owner: Arc<str>, path: &str, frame: crate::gameplay::reference_frame::ReferenceFrame) -> EffectTicket {
        self.start_world_mode(owner, path, frame, true)
    }
    fn start_world_mode(&mut self, owner: Arc<str>, path: &str, frame: crate::gameplay::reference_frame::ReferenceFrame, once: bool) -> EffectTicket {
        let placement = EntityPlacement { target: crate::gameplay::event_graph::EntityReference::Invalid, tag: String::new(),
            local: crate::gameplay::reference_frame::ReferenceFrame::default(), attached: false };
        self.start_entity_mode(owner, path, placement, Some(frame), once)
    }
    fn start_entity_mode(&mut self, owner: Arc<str>, path: &str, placement: EntityPlacement, initial_world: Option<crate::gameplay::reference_frame::ReferenceFrame>, once: bool) -> EffectTicket {
        let retirement = Arc::new(Lifetime::new(once));
        let ticket = EffectTicket { lifetime: Arc::downgrade(&retirement), path: Arc::from(path) };
        self.0.push_back(EffectRequest::StartEntity { owner, path: path.to_owned(), placement, initial_world, retirement, context: None });
        ticket
    }
    ///7ee610: retain source owner/culprit and inherited velocity on the same
    ///FX request/instance, without creating another acquisition or actor owner.
    pub(super) fn start_context_once(&mut self, owner: Arc<str>, path: &str, placement: EntityPlacement,
        initial_world: crate::gameplay::reference_frame::ReferenceFrame, context: crate::rendering::bevy_backend::world_effects::EntityEffectContext) -> EffectTicket {
        let retirement = Arc::new(Lifetime::new(true));
        let ticket = EffectTicket { lifetime: Arc::downgrade(&retirement), path: Arc::from(path) };
        self.0.push_back(EffectRequest::StartEntity { owner, path: path.to_owned(), placement, initial_world: Some(initial_world), retirement, context: Some(context) });
        ticket
    }
    pub(super) fn update_entity(&mut self, owner: Arc<str>, placement: EntityPlacement) {
        self.0.push_back(EffectRequest::UpdateEntity { owner, placement });
    }
    pub(super) fn stop_entity(&mut self, owner: Arc<str>) { self.0.push_back(EffectRequest::StopEntity { owner }); }
    pub(super) fn start_player(&mut self, owner: &str, path: &str) {
        self.0.push_back(EffectRequest::StartPlayer { owner: owner.to_owned(), path: path.to_owned() });
    }
    pub(super) fn stop_player(&mut self, owner: &str) {
        self.0.push_back(EffectRequest::StopPlayer { owner: owner.to_owned() });
    }
}

/// Acknowledges actual retirement, including the render snapshot and all
/// instance particle/sound tails. A cancelled acquisition must also return
/// before the gameplay entity can release its source owner.
pub(super) fn effect_completed(ticket: &EffectTicket) -> bool {
    ticket.lifetime.upgrade().is_none_or(|lifetime| lifetime.completed.load(Ordering::Acquire))
}

pub(super) fn entity_retired(world: &bevy::prelude::World, ticket: &EffectTicket) -> bool {
    if ticket.lifetime.strong_count() != 0 { return false; }
    world.get_resource::<UsableEffects>().is_none_or(|owner| !owner.loading.contains_key(ticket.path.as_ref()) && owner.retiring.is_empty())
}

/// Readiness includes actual source acquisition and the existing material,
/// image and audio acknowledgements, without requiring an active actor.
pub(super) fn reservation_ready(world: &bevy::prelude::World, key: &str) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<UsableEffects>() else { return Ok(false) };
    let Some(paths) = owner.reservations.get(key) else { return Ok(false) };
    //A registered empty dependency set owns no source, material, image, sound
    //or gameplay work. Unrelated world FX preparation cannot delay it.
    if paths.is_empty() { return Ok(true); }
    if paths.iter().any(|path| !owner.wanted.keys().any(|wanted| matches!(wanted, EffectOwner::Reservation(group, dependency) if group.as_ref() == key && dependency == path))) {
        return Err(format!("FX dependency reservation {key} failed acquisition or material publication"));
    }
    Ok(owner.reservations_ready.contains(key))
}

///The same actual playing publication used by WorldFX, after its source,
///descriptor, material, image and sound admission. No diagnostic snapshot or
///extra resource acquisition is needed by persistent PlayerFX startup.
pub(super) fn player_published(world: &bevy::prelude::World, name: &str, path: &str) -> Result<bool, String> {
    let Some(owner) = world.get_resource::<UsableEffects>() else { return Ok(false) };
    let Some((key, wanted)) = owner.wanted.iter().find(|(key, _)|
        matches!(key, EffectOwner::Player(player) if player == name)) else {
        let queued = world.get_resource::<EffectRequests>().is_some_and(|requests|
            requests.0.iter().any(|request| matches!(request,
                EffectRequest::StartPlayer { owner, path: requested } if owner == name && requested == path)));
        return if queued { Ok(false) } else { Err(format!("Player FX {name} lost its accepted request for {path}")) };
    };
    if wanted.path != path { return Err(format!("Player FX {name} changed its requested source")); }
    if !owner.ready.contains_key(key) { return Ok(false); }
    Ok(world.get_resource::<crate::rendering::bevy_backend::world_effects::DynamicEffects>()
        .is_some_and(|active| active.0.iter().any(|entry|
            entry.owner.strip_prefix("player:") == Some(name)
                && entry.effect == path && entry.revision == wanted.revision)))
}

///A merged render snapshot keeps one effect per name. Gameplay preparation
///must also visit the actual acquired bundles whose equal-named sub-effects
///were shadowed there: their entity/explosion receipts belong to those leases.
///Called only when that immutable snapshot changes, never on a steady frame.
pub(super) fn gameplay_sources(world:&bevy::prelude::World)->Vec<Arc<Sources>> {
    let Some(owner)=world.get_resource::<UsableEffects>()else{return Vec::new();};
    let mut seen=std::collections::HashSet::new();
    owner.ready.values().filter(|source|seen.insert(Arc::as_ptr(source)))
        .cloned().collect()
}

///The existing opt-in loading report borrows all actual admission witnesses;
///it neither publishes readiness nor keeps resource leases after returning.
pub(super) fn debug_reservation(world:&bevy::prelude::World,key:&str)->String {
    use std::fmt::Write;
    use crate::rendering::bevy_backend::{world_effects::DynamicMaterialPrewarm,
        particle_images::SceneParticleImages,world_sounds::WorldSounds};
    let Some(owner)=world.get_resource::<UsableEffects>()else{return format!("FX[{key}] owner=absent");};
    let mut output=format!("FX[{key}] registered={} ready={:?} revision={} published={:?} active={:?} loading={} retiring={}",
        owner.reservations.contains_key(key),reservation_ready(world,key),owner.revision,
        owner.published_revision,owner.published_active_revision,owner.loading.len(),owner.retiring.len());
    if let Some(prewarm)=world.get_resource::<DynamicMaterialPrewarm>() {
        let _=write!(output," particle_gen={}/{}/{}",prewarm.generation,
            prewarm.completed.load(Ordering::Acquire),prewarm.failed.load(Ordering::Acquire));
    }
    if let Some(materials)=world.get_resource::<DynamicWorldMaterials>() {
        let _=write!(output," world_gen={}/{}/{}/{} images={}/{}",materials.generation,
            materials.completed.load(Ordering::Acquire),materials.gpu_completed.load(Ordering::Acquire),
            materials.failed.load(Ordering::Acquire),materials.images_ready,materials.image_revision);
    }
    if let Some(decals)=world.get_resource::<DynamicDecalMaterials>() {
        let _=write!(output," decal_gen={}/{}/{}/{}",decals.generation,
            decals.completed.load(Ordering::Acquire),decals.gpu_completed.load(Ordering::Acquire),
            decals.failed.load(Ordering::Acquire));
    }
    let audio=world.get_resource::<WorldSounds>().map(|sounds|sounds.lock());
    let images=world.get_resource::<SceneParticleImages>();
    let _=write!(output," audio_bridge={:?}",audio.as_ref().map(|audio|audio.ready));
    for path in owner.reservations.get(key).into_iter().flatten() {
        let source=owner.ready.get(&EffectOwner::Reservation(Arc::from(key),path.clone()));
        let _=write!(output,"\n      path={path} source={} loading={} ",source.is_some(),owner.loading.contains_key(path));
        if let Some(source)=source {
            let missing_images=source.particle_materials().flat_map(|material|material.images.values())
                .filter(|image|image.scene_owned&&!images.is_some_and(|images|images.handles.contains_key(&image.path)))
                .map(|image|image.path.as_ref()).collect::<Vec<_>>();
            let mut missing_sounds=Vec::new();
            let mut errors=Vec::new();
            for (name,effect) in &source.effects {
                match effect {
                    Err(error)=>errors.push(format!("{name}:{error}")),
                    Ok(effect)=>for event in effect.groups.iter().flat_map(|group|&group.sounds)
                        .flat_map(|sound|[&sound.start,&sound.stop]).flatten() {
                        match audio.as_ref().and_then(|audio|audio.events.get(&event.name)) {
                            Some(Ok(_))=>{},Some(Err(error))=>errors.push(format!("{}:{error}",event.name)),
                            None=>missing_sounds.push(event.name.as_str()),
                        }
                    },
                }
            }
            let _=write!(output,"gameplay={} missing_images={missing_images:?} missing_sounds={missing_sounds:?} errors={errors:?}",source.gameplay_ready());
            for (name,effect) in &source.effects {
                let Ok(effect)=effect else{continue;};
                for (group_index,group) in effect.groups.iter().enumerate() {
                    for entry in &group.entity_defs {
                        let _=write!(output," entity[{name}:{group_index}:{}]={}/{}",entry.index,
                            entry.prepared.load(Ordering::Acquire),entry.declarations.iter().flatten()
                                .map(|definition|definition.entity.as_str()).collect::<Vec<_>>().join(","));
                    }
                    for entry in &group.explosions {
                        let _=write!(output," explosion[{name}:{group_index}:{}]={}",entry.index,
                            entry.prepared.load(Ordering::Acquire));
                    }
                }
            }
        }
    }
    output
}

/// Whole-level close bypasses the ordinary scene-publication gate. This
/// removes desired work only: real render entities, instances and audio
/// objects retain their leases until their own shutdown acknowledges them.
/// Call after gameplay producers begin retirement and before FxSimulation
/// shutdown; retire_level_acquisitions continues draining cancelled workers.
pub(super) fn begin_retirement(world: &mut bevy::prelude::World) -> Result<(), String> {
    if let Some(mut requests) = world.get_resource_mut::<EffectRequests>() { requests.0.clear(); }
    if let Some(mut owner) = world.get_resource_mut::<UsableEffects>() {
        owner.close_acquisitions();
        let changed = !owner.wanted.is_empty() || !owner.ready.is_empty() || !owner.reservations.is_empty();
        owner.wanted.clear(); owner.ready.clear();
        owner.reservations.clear(); owner.reservations_ready.clear();
        owner.placements.clear(); owner.entity_placements.clear(); owner.world_placements.clear(); owner.beam_targets.clear();
        owner.completed.clear(); owner.cache.clear(); owner.base = None;
        owner.cancel_unwanted_acquisitions();
        if changed {
            owner.revision = owner.revision.wrapping_add(1);
            owner.retirement = owner.retirement.wrapping_add(1);
        }
        // Poll once without blocking. No completed acquisition is admitted
        // after close; its result drops through the existing worker owner.
        owner.retire_acquisitions();
    }
    if let Some(mut active) = world.get_resource_mut::<crate::rendering::bevy_backend::world_effects::DynamicEffects>() {
        if !active.0.is_empty() { active.0 = Arc::default(); }
    }
    Ok(())
}

/// Desired effect and its prepared immutable dependencies belong to the
/// usable component, not to its animation node or the global program cache.
#[derive(Resource, Default)]
pub(super) struct UsableEffects {
    cancelled: Arc<AtomicBool>,
    wanted: HashMap<EffectOwner, Wanted>,
    reservations: HashMap<Arc<str>, Vec<String>>,
    reservations_ready: std::collections::HashSet<Arc<str>>,
    pub(super) placements: HashMap<EffectOwner, super::interactions::EffectSpawn>,
    entity_placements: HashMap<EffectOwner, EntityPlacement>,
    world_placements: HashMap<EffectOwner, crate::gameplay::reference_frame::ReferenceFrame>,
    beam_targets: HashMap<EffectOwner, crate::gameplay::particles::beam::BeamTarget>,
    transform_revision: u64,
    published_transform_revision: u64,
    pub(super) ready: HashMap<EffectOwner, Arc<Sources>>,
    cache: HashMap<String, Weak<Sources>>,
    loading: HashMap<String, PendingSource>,
    retiring: Vec<Mutex<Acquisition>>,
    completed: Vec<(String, Result<Arc<Sources>, String>)>,
    pub(super) revision: u64,
    next_revision: u64,
    retirement: u64,
    published_revision: Option<u64>,
    published_active_revision: Option<u64>,
    base: Option<Arc<Sources>>,
    seen_retired: u64,
    cleanup_dirty: bool,
    cleanup: Option<Mutex<Task<()>>>,
}

impl Drop for UsableEffects {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl UsableEffects {
    fn retire_completed(&mut self) {
        let completed: Vec<_> = self.wanted.iter().filter_map(|(key, wanted)|
            (wanted.retirement.once && wanted.retirement.completed.load(Ordering::Acquire)).then_some(key.clone())).collect();
        if completed.is_empty() { return; }
        for key in completed {
            self.wanted.remove(&key); self.entity_placements.remove(&key); self.world_placements.remove(&key); self.beam_targets.remove(&key);
            if self.ready.remove(&key).is_some() { self.retirement = self.retirement.wrapping_add(1); }
        }
        self.revision = self.revision.wrapping_add(1); self.cleanup_dirty = true;
        self.cancel_unwanted_acquisitions();
    }
    fn release_reservation(&mut self, owner: &Arc<str>) {
        if let Some(paths) = self.reservations.remove(owner) {
            for path in paths {
                let key = EffectOwner::Reservation(owner.clone(), path);
                self.wanted.remove(&key);
                if self.ready.remove(&key).is_some() { self.retirement = self.retirement.wrapping_add(1); }
            }
            self.reservations_ready.remove(owner);
            self.revision = self.revision.wrapping_add(1);
            self.cleanup_dirty = true;
        }
    }
    /// Stop desired publication; already-running particle/sound tails retain
    /// their own immutable source leases until the shared FX owner finishes.
    pub(super) fn stop_usable(&mut self, index: usize) {
        let actor = EffectOwner::Usable(index);
        self.wanted.remove(&actor);
        if self.ready.remove(&actor).is_some() { self.retirement = self.retirement.wrapping_add(1); }
        self.placements.remove(&actor);
        self.revision = self.revision.wrapping_add(1);
        self.cleanup_dirty = true;
    }
    pub(super) fn cancellation_token(&self) -> Arc<AtomicBool> { self.cancelled.clone() }

    pub(super) fn close_acquisitions(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(super) fn retire_acquisitions(&mut self) -> bool {
        let still_running = |task: &mut Mutex<Acquisition>| {
            let task = task.get_mut().unwrap_or_else(|error| error.into_inner());
            bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)).is_none()
        };
        self.loading.retain(|_, pending| still_running(&mut pending.task));
        self.retiring.retain_mut(still_running);
        if let Some(cleanup) = &mut self.cleanup {
            let cleanup = cleanup.get_mut().unwrap_or_else(|error| error.into_inner());
            if bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(cleanup)).is_some() {
                self.cleanup = None;
            }
        }
        self.loading.is_empty() && self.retiring.is_empty() && self.cleanup.is_none()
    }

    fn request(
        &mut self,
        actor: EffectOwner,
        path: String,
        catalog: Arc<crate::archive::Catalog>,
        programs: &super::ProgramCacheSource,
        resources: &ParticleResourceOwner,
        world: &Arc<Sources>,
    ) {
        self.next_revision = self.next_revision.wrapping_add(1);
        self.wanted.insert(actor.clone(), Wanted { context: None, path: path.clone(), revision: self.next_revision, retirement: Arc::new(Lifetime::new(false)) });
        self.revision = self.revision.wrapping_add(1);
        let cached = self.cache.get(&path).and_then(Weak::upgrade);
        if self.ready.remove(&actor).is_some() {
            self.retirement = self.retirement.wrapping_add(1);
            self.cleanup_dirty = true;
        }
        if let Some(source) = cached {
            if crate::rendering::bevy_backend::world_effects::trace_lifecycle() { eprintln!("FX_LIFECYCLE source=weak effect={path}"); }
            self.ready.insert(actor, source);
            return;
        }
        if world.effects.get(&path).is_some_and(Result::is_ok) {
            if crate::rendering::bevy_backend::world_effects::trace_lifecycle() { eprintln!("FX_LIFECYCLE source=world effect={path}"); }
            match world.subset(&path) {
                Ok(source) => {
                    let source = Arc::new(source);
                    self.cache.insert(path, Arc::downgrade(&source));
                    self.ready.insert(actor, source);
                    return;
                }
                Err(error) => { bevy::log::warn!("Usable effect {path} dependencies unavailable: {error}"); }
            }
        }
        if self.loading.contains_key(&path) { return; }
        let name = path.clone();
        let level_closed = self.cancelled.clone();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        let programs = programs.0.clone();
        let resources = resources.0.clone();
        let task = IoTaskPool::get().spawn(async move {
            let started = crate::rendering::bevy_backend::world_effects::trace_lifecycle().then(std::time::Instant::now);
            let cancelled = || level_closed.load(Ordering::Acquire) || stop.load(Ordering::Acquire);
            if cancelled() { return Err("effect acquisition cancelled".into()); }
            let mut programs = programs.lock().unwrap_or_else(|error| error.into_inner());
            let programs = programs.as_mut().map_err(|error| error.clone())?;
            if cancelled() { return Err("effect acquisition cancelled".into()); }
            let mut resources = resources.lock().unwrap_or_else(|error| error.into_inner());
            let sources = Sources::load_named_cancellable(vec![name.clone()], &catalog, programs, &mut resources, cancelled)?;
            sources.effects.get(&name).ok_or_else(|| format!("effect {name} was not acquired"))?
                .as_ref().map_err(Clone::clone)?;
            if let Some(started) = started {
                eprintln!("FX_LIFECYCLE source=io effect={name} definitions={} acquire_ms={:.3}",
                    sources.effects.len(), started.elapsed().as_secs_f64() * 1000.0);
            }
            Ok(Arc::new(sources))
        });
        self.loading.insert(path, PendingSource { task: Mutex::new(task), cancelled });
    }

    fn cancel_unwanted_acquisitions(&mut self) {
        let wanted = &self.wanted;
        let before = self.retiring.len();
        for (_, pending) in self.loading.extract_if(|path, _| !wanted.values().any(|request| &request.path == path)) {
            pending.cancelled.store(true, Ordering::Release);
            self.retiring.push(pending.task);
        }
        if self.retiring.len() != before && crate::rendering::bevy_backend::world_effects::trace_lifecycle() {
            eprintln!("FX_LIFECYCLE source_acquisitions_cancelled={}", self.retiring.len() - before);
        }
    }

    fn poll(&mut self) {
        let retiring = self.retiring.len();
        self.retiring.retain_mut(|task| {
            let task = task.get_mut().unwrap_or_else(|error| error.into_inner());
            bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)).is_none()
        });
        // The cancelled decoder and its output have dropped. A prune can now
        // retire their dependencies even if a previous prune already ran.
        self.cleanup_dirty |= self.retiring.len() != retiring;
        if self.loading.is_empty() { return; }
        self.completed.clear();
        let completed = &mut self.completed;
        self.loading.retain(|name, pending| {
            let task = pending.task.get_mut().unwrap_or_else(|error| error.into_inner());
            if let Some(result) = bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)) {
                completed.push((name.clone(), result));
                false
            } else { true }
        });
        for (name, result) in self.completed.drain(..) {
            match result {
                Ok(sources) => {
                    self.revision = self.revision.wrapping_add(1);
                    self.cache.insert(name.clone(), Arc::downgrade(&sources));
                    let mut used = false;
                    for (actor, _) in self.wanted.iter().filter(|(_, wanted)| wanted.path == name) {
                        self.ready.insert(actor.clone(), sources.clone());
                        used = true;
                    }
                    if !used { self.cleanup_dirty = true; }
                }
                Err(error) => {
                    bevy::log::warn!("Usable effect {name} unavailable: {error}");
                    let failed: Vec<_> = self.wanted.iter().filter_map(|(owner, wanted)|
                        (wanted.path == name).then_some(owner.clone())).collect();
                    for owner in failed {
                        self.wanted.remove(&owner);
                        self.ready.remove(&owner);
                        self.placements.remove(&owner);
                        self.entity_placements.remove(&owner);
                        self.world_placements.remove(&owner);
                        self.beam_targets.remove(&owner);
                    }
                    self.revision = self.revision.wrapping_add(1);
                    self.cleanup_dirty = true;
                }
            }
        }
    }
}

/// Called from the gameplay schedule after interaction state changes. This
/// consumes FX commands separately from the animation node, while retaining
/// pending work until its background acquisition has finished.
pub(super) fn prepare_requests(
    owner: Option<bevy::prelude::ResMut<UsableEffects>>,
    interactions: Option<bevy::prelude::ResMut<super::interactions::Interactions>>,
    programs: Option<bevy::prelude::Res<super::ProgramCacheSource>>,
    resources: Option<bevy::prelude::Res<ParticleResourceOwner>>,
    world: Option<bevy::prelude::Res<crate::rendering::bevy_backend::world_effects::WorldEffectSources>>,
    requests: Option<bevy::prelude::ResMut<EffectRequests>>,
    scene: bevy::prelude::Res<super::SceneData>,
    prewarm: bevy::prelude::Res<crate::rendering::bevy_backend::world_effects::DynamicMaterialPrewarm>,
) {
    let (Some(mut owner), Some(programs), Some(resources), Some(world), Some(catalog)) =
        (owner, programs, resources, world, scene.catalog.as_ref()) else { return };
    if owner.cancelled.load(Ordering::Acquire) {
        if let Some(mut requests) = requests { requests.0.clear(); }
        return;
    }
    if !owner.loading.is_empty() || !owner.retiring.is_empty() { owner.poll(); }
    owner.retire_completed();
    let retired = prewarm.retired_signal.load(Ordering::Acquire);
    if retired != owner.seen_retired {
        owner.seen_retired = retired;
        owner.cleanup_dirty = true;
    }
    if owner.cleanup.is_some() {
        let task = owner.cleanup.as_mut().unwrap().get_mut().unwrap_or_else(|error| error.into_inner());
        if bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)).is_some() { owner.cleanup = None; }
    }
    if owner.cleanup_dirty && owner.cleanup.is_none() {
        owner.cleanup_dirty = false;
        owner.cache.retain(|_, source| source.strong_count() != 0);
        let resources = resources.0.clone();
        let cancelled = owner.cancelled.clone();
        owner.cleanup = Some(Mutex::new(IoTaskPool::get().spawn(async move {
            if cancelled.load(Ordering::Acquire) { return; }
            let started = crate::rendering::bevy_backend::world_effects::trace_lifecycle().then(std::time::Instant::now);
            let mut resources = resources.lock().unwrap_or_else(|error| error.into_inner());
            if !cancelled.load(Ordering::Acquire) { resources.retire_unused(); }
            if let Some(started) = started { eprintln!("FX_LIFECYCLE acquisition_cache_retire_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0); }
        })));
    }
    let previous_revision = owner.revision;
    if let Some(mut interactions) = interactions.filter(|interactions| !interactions.pending.is_empty()) {
        for change in &mut interactions.pending {
            let actor = EffectOwner::Usable(change.actor);
            if change.stop_effect {
                owner.stop_usable(change.actor);
                change.stop_effect = false;
            }
            if let Some(spawn) = change.start_effect.take() {
                owner.placements.insert(actor.clone(), spawn.clone());
                owner.request(actor, spawn.path, catalog.clone(), &programs, &resources, &world.0);
            }
        }
        interactions.pending.retain(|change| change.animation.is_some() || change.stop_effect || change.start_effect.is_some());
    }
    if let Some(mut requests) = requests.filter(|requests| !requests.0.is_empty()) {
        for request in requests.0.drain(..) {
            match request {
                EffectRequest::StartPlayer { owner: key, path } => owner.request(
                    EffectOwner::Player(key), path, catalog.clone(), &programs, &resources, &world.0),
                EffectRequest::StopPlayer { owner: key } => {
                    let key = EffectOwner::Player(key);
                    owner.wanted.remove(&key);
                    if owner.ready.remove(&key).is_some() { owner.retirement = owner.retirement.wrapping_add(1); }
                    owner.revision = owner.revision.wrapping_add(1);
                    owner.cleanup_dirty = true;
                }
                EffectRequest::StartEntity { owner: key, path, placement, initial_world, retirement, context } => {
                    let key = EffectOwner::Entity(key);
                    owner.beam_targets.remove(&key);
                    if let Some(frame) = initial_world { owner.world_placements.insert(key.clone(), frame); }
                    else { owner.world_placements.remove(&key); }
                    owner.entity_placements.insert(key.clone(), placement);
                    owner.request(key.clone(), path, catalog.clone(), &programs, &resources, &world.0);
                    if let Some(wanted) = owner.wanted.get_mut(&key) { wanted.retirement = retirement; wanted.context = context; }
                }
                EffectRequest::UpdateEntity { owner: key, placement } => {
                    let key = EffectOwner::Entity(key);
                    let had_world = owner.world_placements.remove(&key).is_some();
                    owner.beam_targets.remove(&key);
                    if owner.wanted.contains_key(&key) && (had_world || owner.entity_placements.get(&key).is_none_or(|old|
                        old.target != placement.target || old.tag != placement.tag || old.local.position != placement.local.position
                            || old.local.basis != placement.local.basis || old.attached != placement.attached)) {
                        owner.entity_placements.insert(key, placement);
                        owner.transform_revision = owner.transform_revision.wrapping_add(1);
                    }
                }
                EffectRequest::UpdateBeam { owner: key, origin, endpoint } => {
                    let key = EffectOwner::Entity(key);
                    if !owner.wanted.contains_key(&key) { continue; }
                    let changed = owner.world_placements.get(&key).is_none_or(|frame| frame.position != origin)
                        || owner.beam_targets.get(&key).is_none_or(|target| target.position != endpoint);
                    owner.world_placements.insert(key.clone(), crate::gameplay::reference_frame::ReferenceFrame {position:origin,..Default::default()});
                    owner.beam_targets.insert(key, crate::gameplay::particles::beam::BeamTarget {position:endpoint,..Default::default()});
                    if changed { owner.transform_revision = owner.transform_revision.wrapping_add(1); }
                }
                EffectRequest::StopEntity { owner: key } => {
                    let key = EffectOwner::Entity(key);
                    owner.wanted.remove(&key); owner.entity_placements.remove(&key); owner.world_placements.remove(&key); owner.beam_targets.remove(&key);
                    if owner.ready.remove(&key).is_some() { owner.retirement = owner.retirement.wrapping_add(1); }
                    owner.revision = owner.revision.wrapping_add(1); owner.cleanup_dirty = true;
                }
                EffectRequest::Reserve { owner: key, mut paths } => {
                    paths.retain(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL"));
                    paths.sort_unstable(); paths.dedup();
                    if owner.reservations.get(&key) == Some(&paths) { continue; }
                    owner.release_reservation(&key);
                    for path in &paths {
                        owner.request(EffectOwner::Reservation(key.clone(), path.clone()), path.clone(),
                            catalog.clone(), &programs, &resources, &world.0);
                    }
                    if paths.is_empty() { owner.reservations_ready.insert(key.clone()); }
                    owner.reservations.insert(key, paths);
                    owner.revision = owner.revision.wrapping_add(1);
                }
                EffectRequest::ReleaseReservation { owner: key } => owner.release_reservation(&key),
            }
        }
    }
    // Keep a queued acquisition if another actor still needs it, including
    // a stop and restart of the same path in this command batch.
    if owner.revision != previous_revision && !owner.loading.is_empty() {
        owner.cancel_unwanted_acquisitions();
    }
}

/// Observe installed source leases through the same request and publication
/// systems as gameplay. No simulation, player input, audio, or draw runs.
pub(super) fn inspect_source_requests(world: &mut bevy::prelude::World, effect: &str) -> Result<(), String> {
    use bevy::ecs::system::RunSystemOnce;
    use crate::rendering::bevy_backend::{particle_images::SceneParticleImages,
        world_effects::{DynamicEffects, DynamicMaterialPrewarm, WorldEffectSources}, world_sounds::WorldSounds};

    world.init_resource::<EffectRequests>();
    world.init_resource::<DynamicMaterialPrewarm>();
    world.init_resource::<DynamicEffects>();
    world.init_resource::<SceneParticleImages>();
    world.init_resource::<WorldSounds>();
    let first_key = "resource-observer-a";
    let second_key = "resource-observer-b";
    let first = EffectOwner::Player(first_key.into());
    let second = EffectOwner::Player(second_key.into());
    let sources = world.get_resource::<WorldEffectSources>().ok_or("scene did not publish world FX sources")?;
    if sources.0.effects.contains_key(effect) {
        return Err(format!("{effect} is already in the scene's FX sources"));
    }
    let advance = |world: &mut bevy::prelude::World| -> Result<(), String> {
        world.run_system_once(prepare_requests).map_err(|error| format!("FX requests: {error:?}"))?;
        world.run_system_once(publish).map_err(|error| format!("FX publication: {error:?}"))
    };
    let drain = |world: &mut bevy::prelude::World| -> Result<(), String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            advance(world)?;
            let owner = world.resource::<UsableEffects>();
            if owner.loading.is_empty() && owner.retiring.is_empty() && owner.cleanup.is_none() && !owner.cleanup_dirty {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline { return Err("FX acquisition did not settle".into()); }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };

    {
        let mut requests = world.resource_mut::<EffectRequests>();
        requests.start_player(first_key, effect);
        requests.stop_player(first_key);
    }
    advance(world)?;
    let retiring = world.resource::<UsableEffects>().retiring.len();
    drain(world)?;
    let owner = world.resource::<UsableEffects>();
    eprintln!("RESOURCE_FX_REQUEST phase=cancelled admitted={retiring} ready={} retiring={} cached={}",
        owner.ready.len(), owner.retiring.len(), owner.cache.len());

    world.resource_mut::<EffectRequests>().start_player(first_key, effect);
    drain(world)?;
    let source = world.resource::<UsableEffects>().ready.get(&first).map(Arc::downgrade)
        .ok_or_else(|| format!("{effect} did not acquire a source"))?;
    world.resource_mut::<EffectRequests>().start_player(second_key, effect);
    drain(world)?;
    let owner = world.resource::<UsableEffects>();
    let shared = owner.ready.get(&second).is_some_and(|second| Weak::ptr_eq(&source, &Arc::downgrade(second)));
    eprintln!("RESOURCE_FX_REQUEST phase=overlap ready={} shared={shared} cached={}", owner.ready.len(), owner.cache.len());
    {
        let mut requests = world.resource_mut::<EffectRequests>();
        requests.stop_player(first_key);
        requests.stop_player(second_key);
    }
    drain(world)?;
    let owner = world.resource::<UsableEffects>();
    eprintln!("RESOURCE_FX_REQUEST phase=released ready={} cached={} source_leases={}",
        owner.ready.len(), owner.cache.len(), source.strong_count());

    world.resource_mut::<EffectRequests>().start_player(first_key, effect);
    advance(world)?;
    let admitted = world.resource::<UsableEffects>().loading.len();
    let texture = super::texture_streaming::observe_level_acquisition(world);
    let readable_request = {
        let mut request = None;
        if let (Some(interactions), Some(physics), Some(live)) = (
            world.get_resource::<super::interactions::Interactions>(),
            world.get_resource::<super::physics_scene::PhysicsScene>(),
            world.get_resource::<crate::gameplay::entity_lifetime::LiveEntities>(),
        ) {
            for (index, readable) in interactions.readable.iter().enumerate() {
                if let Some((identity, item)) = physics.readable_item(&readable.entity, live)? {
                    request = Some((index, identity, item));
                    break;
                }
            }
        }
        request
    };
    let readable = if let Some((index, identity, item)) = readable_request {
        world.resource_mut::<super::interactions::Interactions>().request_readable(index, identity, item);
        true
    } else { false };
    let rig = super::usable_animation::observe_level_acquisition(world);
    super::level_lifetime::close_level_acquisitions(world);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !super::level_lifetime::retire_level_acquisitions(world) {
        if std::time::Instant::now() >= deadline { return Err("outgoing FX worker did not retire".into()); }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let owner = world.resource::<UsableEffects>();
    eprintln!("RESOURCE_FX_REQUEST phase=map_retirement admitted={admitted} readable={readable} rig={rig} texture={texture} loading={} retiring={} ready={} cached={}",
        owner.loading.len(), owner.retiring.len(), owner.ready.len(), owner.cache.len());
    Ok(())
}

/// Publish shared immutable sources first so the existing audio and image
/// owners can prepare them. Only publish a start after those dependencies and
/// the render-side material prewarm acknowledge the same request generation.
pub(super) fn publish(
    owner: Option<bevy::prelude::ResMut<UsableEffects>>,
    interactions: Option<bevy::prelude::Res<super::interactions::Interactions>>,
    mut world: bevy::prelude::ResMut<crate::rendering::bevy_backend::world_effects::WorldEffectSources>,
    mut prewarm: bevy::prelude::ResMut<crate::rendering::bevy_backend::world_effects::DynamicMaterialPrewarm>,
    mut world_materials: bevy::prelude::ResMut<DynamicWorldMaterials>,
    mut decal_materials: bevy::prelude::ResMut<DynamicDecalMaterials>,
    mut active: bevy::prelude::ResMut<crate::rendering::bevy_backend::world_effects::DynamicEffects>,
    images: bevy::prelude::Res<crate::rendering::bevy_backend::particle_images::SceneParticleImages>,
    sounds: bevy::prelude::Res<crate::rendering::bevy_backend::world_sounds::WorldSounds>,
) {
    use crate::gameplay::{event_graph::EntityReference, reference_frame::ReferenceFrame};
    use crate::rendering::bevy_backend::world_effects::DynamicEffect;
    let Some(mut owner) = owner else { return };
    if owner.cancelled.load(Ordering::Acquire) { return; }
    let key = |owner: &EffectOwner| match owner {
        EffectOwner::Usable(index) => format!("usable:{index}"),
        EffectOwner::Player(name) => format!("player:{name}"),
        EffectOwner::Entity(name) => format!("entity:{name}"),
        EffectOwner::Reservation(name, path) => format!("reservation:{name}:{path}"),
    };
    if prewarm.generation != 0 && (prewarm.failed.load(Ordering::Acquire) == prewarm.generation
        || world_materials.failed.load(Ordering::Acquire) == prewarm.generation
        || decal_materials.failed.load(Ordering::Acquire) == prewarm.generation)
        && owner.published_revision == Some(owner.revision) {
        let failed: Vec<_> = owner.wanted.iter().filter_map(|(id, wanted)| {
            (!active.0.iter().any(|entry| entry.owner == key(id) && entry.revision == wanted.revision)).then_some(id.clone())
        }).collect();
        if !failed.is_empty() {
            for id in failed {
                owner.wanted.remove(&id);
                owner.ready.remove(&id);
                owner.placements.remove(&id);
                owner.entity_placements.remove(&id);
                owner.world_placements.remove(&id);
                owner.beam_targets.remove(&id);
            }
            owner.revision = owner.revision.wrapping_add(1);
            owner.retirement = owner.retirement.wrapping_add(1);
            owner.cleanup_dirty = true;
            owner.cancel_unwanted_acquisitions();
        }
    }
    if owner.base.is_some() && owner.published_revision == Some(owner.revision)
        && owner.published_active_revision == Some(owner.revision)
        && owner.published_transform_revision == owner.transform_revision { return; }
    if owner.base.is_none() {
        owner.base = Some(world.0.clone());
        owner.published_revision = Some(0);
        owner.published_active_revision = Some(0);
    }
    let base = owner.base.as_ref().unwrap().clone();
    if owner.published_revision != Some(owner.revision) {
        let combined = Sources::with_additions(&base, owner.ready.values().map(Arc::as_ref));
        let dynamic = Sources::with_additions(&Sources::default(), owner.ready.values()
            .filter(|source| !Arc::ptr_eq(source, &base)).map(Arc::as_ref));
        let needs_materials = dynamic.particle_materials().next().is_some();
        let needs_world_materials = !dynamic.world_meshes().is_empty();
        let needs_decals = !dynamic.decal_materials().is_empty();
        world.0 = Arc::new(combined);
        let dynamic = Arc::new(dynamic);
        let leases = Arc::new(owner.ready.values().map(Arc::downgrade).collect::<Vec<_>>());
        prewarm.sources = dynamic.clone();
        prewarm.generation = owner.revision;
        prewarm.retirement = owner.retirement;
        if !needs_materials { prewarm.completed.store(prewarm.generation, Ordering::Release); }
        world_materials.sources = dynamic.clone();
        world_materials.leases = leases.clone();
        world_materials.generation = owner.revision;
        world_materials.retirement = owner.retirement;
        if !needs_world_materials { world_materials.completed.store(world_materials.generation, Ordering::Release); }
        decal_materials.sources = dynamic;
        decal_materials.leases = leases;
        decal_materials.generation = owner.revision;
        decal_materials.retirement = owner.retirement;
        if !needs_decals { decal_materials.completed.store(decal_materials.generation, Ordering::Release); }
        owner.published_revision = Some(owner.revision);
    }
    if owner.published_active_revision == Some(owner.revision)
        && owner.published_transform_revision == owner.transform_revision { return; }
    let mut entries: Vec<DynamicEffect> = active.0.iter().filter(|entry| {
        owner.wanted.iter().any(|(id, wanted)| !matches!(id, EffectOwner::Reservation(_, _))
            && wanted.revision == entry.revision && key(id) == entry.owner)
    }).cloned().collect();
    let prewarmed = (prewarm.generation == 0 || prewarm.completed.load(Ordering::Acquire) == prewarm.generation)
        && (world_materials.generation == 0 || world_materials.completed.load(Ordering::Acquire) == world_materials.generation)
        && (decal_materials.generation == 0 || decal_materials.completed.load(Ordering::Acquire) == decal_materials.generation);
    if prewarmed {
        let audio = sounds.lock();
        let source_prepared = |sources: &Arc<Sources>| {
            if !sources.gameplay_ready() { return false; }
            if !Arc::ptr_eq(sources, &base) && !sources.particle_materials().flat_map(|material| material.images.values())
                .filter(|image| image.scene_owned).all(|image| images.handles.contains_key(&image.path)) { return false; }
            let mut events = sources.effects.values().filter_map(|effect| effect.as_ref().ok())
                .flat_map(|effect| &effect.groups).flat_map(|group| &group.sounds)
                .flat_map(|sound| [&sound.start, &sound.stop]).flatten().peekable();
            let has_sound = events.peek().is_some();
            (!has_sound || audio.ready) && events.all(|event| audio.events.contains_key(&event.name))
        };
        // Reservations participate in the same source/material/audio pipeline
        // but never enter DynamicEffects or create a playing instance.
        {
            let UsableEffects { reservations, reservations_ready, ready, .. } = &mut *owner;
            for (group, paths) in reservations {
                let dependencies = || ready.iter().filter(|(key, _)| matches!(key, EffectOwner::Reservation(owner, _) if owner == group));
                if dependencies().count() == paths.len() && dependencies().all(|(_, source)| source_prepared(source)) {
                    reservations_ready.insert(group.clone());
                } else { reservations_ready.remove(group); }
            }
        }
        for (id, wanted) in &owner.wanted {
            if matches!(id, EffectOwner::Reservation(_, _)) { continue; }
            let name = key(id);
            if entries.iter().any(|entry| entry.owner == name && entry.revision == wanted.revision) { continue; }
            let Some(sources) = owner.ready.get(id) else { continue };
            if !source_prepared(sources) { continue; }
            let (target, tag, local, attached) = match id {
                EffectOwner::Player(_) => (EntityReference::Player, String::new(), ReferenceFrame::default(), true),
                EffectOwner::Entity(_) => {
                    let Some(placement) = owner.entity_placements.get(id) else { continue };
                    (placement.target.clone(), placement.tag.clone(), placement.local, placement.attached)
                }
                EffectOwner::Usable(index) => {
                    let Some(actor) = interactions.as_ref().and_then(|interactions| interactions.actors.get(*index)) else { continue };
                    let Some(placement) = owner.placements.get(id) else { continue };
                    (EntityReference::Named(actor.entity.clone()), String::new(), ReferenceFrame { position: placement.origin, basis: placement.axis }, placement.attached)
                }
                EffectOwner::Reservation(_, _) => continue,
            };
            entries.push(DynamicEffect { entity_context: wanted.context, owner: name, revision: wanted.revision, effect: wanted.path.clone(),
                target, tag, local, attached, initial_world: owner.world_placements.get(id).copied(), explicit_target: owner.beam_targets.get(id).copied(), sources: sources.clone(), retirement: wanted.retirement.clone() });
        }
    }
    for entry in &mut entries {
        if let Some((id, placement)) = owner.entity_placements.iter().find(|(id,_)| key(id) == entry.owner) {
            entry.target = placement.target.clone(); entry.tag.clone_from(&placement.tag); entry.local = placement.local; entry.attached = placement.attached;
            entry.initial_world = owner.world_placements.get(id).copied();
            entry.explicit_target = owner.beam_targets.get(id).copied();
        }
    }
    if entries.len() != active.0.len() || entries.iter().zip(active.0.iter()).any(|(left, right)| left.owner != right.owner || left.revision != right.revision
        || left.target != right.target || left.tag != right.tag || left.local.position != right.local.position || left.local.basis != right.local.basis || left.attached != right.attached
        || left.initial_world.map(|frame| (frame.position, frame.basis)) != right.initial_world.map(|frame| (frame.position, frame.basis))
        || left.explicit_target.map(|target| (target.position, target.basis)) != right.explicit_target.map(|target| (target.position, target.basis))) {
        active.0 = Arc::new(entries);
    }
    owner.published_transform_revision = owner.transform_revision;
    let playing_wanted = owner.wanted.keys().filter(|key| !matches!(key, EffectOwner::Reservation(_, _))).count();
    if (active.0.len() == playing_wanted && owner.reservations_ready.len() == owner.reservations.len())
        || (prewarm.generation != 0 && (prewarm.failed.load(Ordering::Acquire) == prewarm.generation
            || world_materials.failed.load(Ordering::Acquire) == prewarm.generation
            || decal_materials.failed.load(Ordering::Acquire) == prewarm.generation)) {
        owner.published_active_revision = Some(owner.revision);
    }
}


/// Read-only, per-request lifecycle diagnostic. No readiness or failure value
/// is manufactured: all tuples contain the existing generation atomics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AdmissionSnapshot {
    pub queued: bool,
    pub wanted: bool,
    pub loading: bool,
    pub ready: bool,
    pub active: bool,
    pub revision: u64,
    pub wanted_revision: Option<u64>,
    pub published_revision: Option<u64>,
    pub published_active_revision: Option<u64>,
    pub prewarm: (u64, u64, u64),
    /// generation, completed, failed, GPU completed, image revision, images ready
    pub world: (u64, u64, u64, u64, u64, u64),
    /// generation, completed, failed, GPU completed, prepared generation/revision/count
    pub decal: (u64, u64, u64, u64, Option<u64>, u64, usize),
    pub coordinator: super::runtime_materials::CoordinatorSnapshot,
    pub source_world_meshes: usize,
    pub source_decal_materials: usize,
    pub image_memberships: usize,
    pub missing_images: Vec<String>,
    pub sounds_ready: bool,
    pub sound_memberships: usize,
    pub missing_sounds: Vec<String>,
    pub prepared_errors: Vec<String>,
}
pub(super) fn observe_admission(world: &bevy::prelude::World, name: &str, path: &str)
    -> Result<AdmissionSnapshot, String> {
    use crate::rendering::bevy_backend::{world_effects::{DynamicEffects, DynamicMaterialPrewarm},
        particle_images::SceneParticleImages, world_sounds::WorldSounds};
    let owner = world.get_resource::<UsableEffects>().ok_or("FX admission observation has no request owner")?;
    let requests = world.get_resource::<EffectRequests>().ok_or("FX admission observation has no command queue")?;
    let prewarm = world.get_resource::<DynamicMaterialPrewarm>().ok_or("FX admission observation has no particle prewarm owner")?;
    let materials = world.get_resource::<DynamicWorldMaterials>().ok_or("FX admission observation has no world material owner")?;
    let decals = world.get_resource::<DynamicDecalMaterials>().ok_or("FX admission observation has no decal material owner")?;
    let images = world.get_resource::<SceneParticleImages>().ok_or("FX admission observation has no particle image owner")?;
    let active = world.get_resource::<DynamicEffects>().ok_or("FX admission observation has no active publication")?;
    let bridge = world.get_resource::<WorldSounds>().ok_or("FX admission observation has no FX sound owner")?.lock();
    let key = EffectOwner::Player(name.to_owned());
    let wanted = owner.wanted.get(&key);
    let source = owner.ready.get(&key);
    let mut missing_images = Vec::new();
    let mut missing_sounds = Vec::new();
    let mut prepared_errors = Vec::new();
    let mut image_memberships = 0;
    let mut source_world_meshes = 0;
    let mut source_decal_materials = 0;
    let mut sound_memberships = 0;
    if let Some(source) = source {
        source_world_meshes = source.world_meshes().len();
        source_decal_materials = source.decal_materials().len();
        if !source.effects.contains_key(path) { prepared_errors.push(format!("requested definition {path} absent from prepared source")); }
        for image in source.particle_materials().flat_map(|material| material.images.values()).filter(|image| image.scene_owned) {
            image_memberships += 1;
            if !images.handles.contains_key(&image.path) { missing_images.push(image.path.to_string()); }
        }
        for (name, definition) in &source.effects {
            match definition {
                Err(error) => prepared_errors.push(format!("{name}: {error}")),
                Ok(definition) => for event in definition.groups.iter().flat_map(|group| &group.sounds)
                    .flat_map(|sound| [&sound.start, &sound.stop]).flatten() {
                    sound_memberships += 1;
                    match bridge.events.get(&event.name) {
                        None => missing_sounds.push(event.name.clone()),
                        Some(Err(error)) => prepared_errors.push(format!("sound {}: {error}", event.name)),
                        Some(Ok(_)) => {},
                    }
                },
            }
        }
    }
    missing_images.sort(); missing_images.dedup();
    missing_sounds.sort(); missing_sounds.dedup();
    prepared_errors.sort(); prepared_errors.dedup();
    Ok(AdmissionSnapshot {
        queued: requests.0.iter().any(|request| matches!(request, EffectRequest::StartPlayer { owner, path: requested } if owner == name && requested == path)),
        wanted: wanted.is_some_and(|wanted| wanted.path == path),
        loading: owner.loading.contains_key(path), ready: source.is_some(),
        active: active.0.iter().any(|entry| entry.owner == format!("player:{name}") && entry.effect == path),
        revision: owner.revision, wanted_revision: wanted.map(|wanted| wanted.revision),
        published_revision: owner.published_revision, published_active_revision: owner.published_active_revision,
        prewarm: (prewarm.generation, prewarm.completed.load(Ordering::Acquire), prewarm.failed.load(Ordering::Acquire)),
        world: (materials.generation, materials.completed.load(Ordering::Acquire), materials.failed.load(Ordering::Acquire),
            materials.gpu_completed.load(Ordering::Acquire), materials.image_revision, materials.images_ready),
        decal: (decals.generation, decals.completed.load(Ordering::Acquire), decals.failed.load(Ordering::Acquire),
            decals.gpu_completed.load(Ordering::Acquire), decals.prepared_generation, decals.prepared_revision, decals.prepared.len()),
        coordinator: super::runtime_materials::observe_coordinator(world)?, source_world_meshes, source_decal_materials,
        image_memberships, missing_images, sounds_ready: bridge.ready, sound_memberships, missing_sounds, prepared_errors,
    })
}


/// Opt-in lifecycle observer only. Inspect the existing requested proxies and
/// borrow the real asset owners; do not acquire assets or alter visibility.
pub(super) fn publish_proxy_observation(world: &mut bevy::prelude::World) {
    use bevy::{asset::VisitAssetDependencies, prelude::*, render::sync_world::RenderEntity};
    use crate::rendering::bevy_backend::projected_decals::{DynamicProxyReadiness, ProxyMainObservation};
    let Some(request) = world.get_resource::<DynamicProxyReadiness>() else { return };
    let mut ids = request.decals.proxies.iter().chain(request.world_meshes.proxies.iter()).copied().collect::<Vec<_>>();
    ids.sort_unstable(); ids.dedup();
    let observations = ids.into_iter().map(|entity| {
        let mesh = world.get::<Mesh3d>(entity).map(|mesh| mesh.0.id());
        let material = world.get::<MeshMaterial3d<super::ProbeMaterial>>(entity).map(|material| material.0.id());
        let mesh_asset = mesh.and_then(|mesh| world.get_resource::<Assets<Mesh>>().and_then(|assets| assets.get(mesh)));
        let material_asset = material.and_then(|material| world.get_resource::<Assets<super::ProbeMaterial>>().and_then(|assets| assets.get(material)));
        let mut images = Vec::new();
        let mut buffers = Vec::new();
        let mut base_other_dependencies = Vec::new();
        if let Some(material) = material_asset {
            material.base.visit_dependencies(&mut |id| {
                match id.try_typed::<Image>() {
                    Ok(id) => images.push((format!("base:{}", images.len()), id,
                        world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(id).is_some()))),
                    Err(_) => base_other_dependencies.push(id),
                }
            });
            let extension = &material.extension;
            if let Some(handle) = &extension.fog_world { images.push(("fog_world".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.fog_canvas { images.push(("fog_canvas".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.fog_curve { images.push(("fog_curve".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.fog_depth { images.push(("fog_depth".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.reflection_brdf_lookup { images.push(("reflection_brdf_lookup".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.reflections { images.push(("reflections".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.local_shadow_map { images.push(("local_shadow_map".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.shadow_map { images.push(("shadow_map".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.screen_map { images.push(("screen_map".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.cloud_map { images.push(("cloud_map".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.diffuse0 { images.push(("diffuse0".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.diffuse1 { images.push(("diffuse1".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.diffuse2 { images.push(("diffuse2".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.bump0 { images.push(("bump0".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.bump1 { images.push(("bump1".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.bump2 { images.push(("bump2".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.gloss0 { images.push(("gloss0".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.gloss1 { images.push(("gloss1".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.gloss2 { images.push(("gloss2".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.metallic0 { images.push(("metallic0".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.metallic1 { images.push(("metallic1".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.metallic2 { images.push(("metallic2".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.mask0 { images.push(("mask0".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.mask1 { images.push(("mask1".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.mask2 { images.push(("mask2".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.specular0 { images.push(("specular0".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.specular1 { images.push(("specular1".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.specular2 { images.push(("specular2".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.detail_bump { images.push(("detail_bump".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.emissive_map { images.push(("emissive_map".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.emissive_mask { images.push(("emissive_mask".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.anisotropic_sampler_image { images.push(("anisotropic_sampler_image".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.lightmap { images.push(("lightmap".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.filling_diffuse { images.push(("filling_diffuse".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.filling_distortion1 { images.push(("filling_distortion1".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.filling_distortion2 { images.push(("filling_distortion2".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.filling_lookup { images.push(("filling_lookup".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.filling_normal { images.push(("filling_normal".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.filling_fresnel { images.push(("filling_fresnel".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.linear_clamp_image { images.push(("linear_clamp_image".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.cloud_opacity { images.push(("cloud_opacity".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.cloud_density { images.push(("cloud_density".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.cloud_back_density { images.push(("cloud_back_density".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.skin_fresnel_lookup { images.push(("skin_fresnel_lookup".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.diffuse_lookup { images.push(("diffuse_lookup".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.sss_mask { images.push(("sss_mask".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.fuzz_mask { images.push(("fuzz_mask".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            if let Some(handle) = &extension.translucency_mask { images.push(("translucency_mask".to_owned(), handle.id(), world.get_resource::<Assets<Image>>().is_some_and(|assets| assets.get(handle.id()).is_some()))); }
            buffers.push(("fog_objects", extension.fog_objects.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.fog_objects.id()).is_some())));
            buffers.push(("fog_parameters", extension.fog_parameters.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.fog_parameters.id()).is_some())));
            buffers.push(("reflection_planes", extension.reflection_planes.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.reflection_planes.id()).is_some())));
            buffers.push(("material_exposure", extension.material_exposure.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.material_exposure.id()).is_some())));
            buffers.push(("sh", extension.sh.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.sh.id()).is_some())));
            buffers.push(("vertex_probes", extension.vertex_probes.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.vertex_probes.id()).is_some())));
            buffers.push(("shadow", extension.shadow.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.shadow.id()).is_some())));
            buffers.push(("local_shadow", extension.local_shadow.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.local_shadow.id()).is_some())));
            buffers.push(("vegetation_slots", extension.vegetation_slots.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.vegetation_slots.id()).is_some())));
            buffers.push(("vegetation_instances", extension.vegetation_instances.id(), world.get_resource::<Assets<bevy::render::storage::ShaderBuffer>>().is_some_and(|assets| assets.get(extension.vegetation_instances.id()).is_some())));
        }
        let main = format!("entity_exists={} mesh={mesh:?} mesh_cpu={} mesh_usage={:?} material={material:?} material_cpu={} visibility={:?} inherited={:?} view_visible={:?} class={:?} layers={:?} no_frustum={} no_cpu_culling={} parent={:?} other_base_dependencies={base_other_dependencies:?}",
            world.get_entity(entity).is_ok(), mesh_asset.is_some(), mesh_asset.map(|mesh| mesh.asset_usage), material_asset.is_some(),
            world.get::<Visibility>(entity), world.get::<InheritedVisibility>(entity), world.get::<ViewVisibility>(entity),
            world.get::<bevy::camera::visibility::VisibilityClass>(entity).map(|class| &class.0),
            world.get::<bevy::camera::visibility::RenderLayers>(entity), world.get::<bevy::camera::visibility::NoFrustumCulling>(entity).is_some(),
            world.get::<bevy::camera::visibility::NoCpuCulling>(entity).is_some(), world.get::<ChildOf>(entity).map(|parent| parent.parent()));
        ProxyMainObservation { entity, render_entity: world.get::<RenderEntity>(entity).map(RenderEntity::id), main, images, buffers }
    }).collect::<Vec<_>>();
    let mut request = world.resource_mut::<DynamicProxyReadiness>();
    if request.diagnostics.as_ref() != &observations { request.diagnostics = Arc::new(observations); }
}
