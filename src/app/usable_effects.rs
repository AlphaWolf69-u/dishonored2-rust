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
    pub prepared_generation: u64,
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

struct Wanted {
    path: String,
    revision: u64,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) enum EffectOwner {
    Usable(usize),
    Player(String),
}

pub(super) enum EffectRequest {
    StartPlayer { owner: String, path: String },
    StopPlayer { owner: String },
}

/// Level-owned requests from gameplay. An owner identifies one active effect;
/// its simulation and mutable parameters never live in the shared definition.
#[derive(Resource, Default)]
pub(super) struct EffectRequests(pub(super) std::collections::VecDeque<EffectRequest>);

impl EffectRequests {
    pub(super) fn start_player(&mut self, owner: &str, path: &str) {
        self.0.push_back(EffectRequest::StartPlayer { owner: owner.to_owned(), path: path.to_owned() });
    }
    pub(super) fn stop_player(&mut self, owner: &str) {
        self.0.push_back(EffectRequest::StopPlayer { owner: owner.to_owned() });
    }
}

/// Desired effect and its prepared immutable dependencies belong to the
/// usable component, not to its animation node or the global program cache.
#[derive(Resource, Default)]
pub(super) struct UsableEffects {
    cancelled: Arc<AtomicBool>,
    wanted: HashMap<EffectOwner, Wanted>,
    pub(super) placements: HashMap<EffectOwner, super::interactions::EffectSpawn>,
    pub(super) ready: HashMap<EffectOwner, Arc<Sources>>,
    cache: HashMap<String, Weak<Sources>>,
    loading: HashMap<String, Mutex<Acquisition>>,
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
    pub(super) fn cancellation_token(&self) -> Arc<AtomicBool> { self.cancelled.clone() }

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
        self.wanted.insert(actor.clone(), Wanted { path: path.clone(), revision: self.next_revision });
        self.revision = self.revision.wrapping_add(1);
        if self.ready.remove(&actor).is_some() {
            self.retirement = self.retirement.wrapping_add(1);
            self.cleanup_dirty = true;
        }
        if world.effects.get(&path).is_some_and(Result::is_ok) {
            if crate::rendering::bevy_backend::world_effects::trace_lifecycle() { eprintln!("FX_LIFECYCLE source=world effect={path}"); }
            match world.subset(&path) {
                Ok(source) => { self.ready.insert(actor, Arc::new(source)); return; }
                Err(error) => { bevy::log::warn!("Usable effect {path} dependencies unavailable: {error}"); }
            }
        }
        if let Some(source) = self.cache.get(&path).and_then(Weak::upgrade) {
            if crate::rendering::bevy_backend::world_effects::trace_lifecycle() { eprintln!("FX_LIFECYCLE source=weak effect={path}"); }
            self.ready.insert(actor, source);
            return;
        }
        if self.loading.contains_key(&path) { return; }
        let name = path.clone();
        let cancelled = self.cancelled.clone();
        let programs = programs.0.clone();
        let resources = resources.0.clone();
        self.loading.insert(path, Mutex::new(IoTaskPool::get().spawn(async move {
            let started = crate::rendering::bevy_backend::world_effects::trace_lifecycle().then(std::time::Instant::now);
            if cancelled.load(Ordering::Acquire) { return Err("level unloaded".into()); }
            let mut programs = programs.lock().unwrap_or_else(|error| error.into_inner());
            let programs = programs.as_mut().map_err(Clone::clone)?;
            let mut resources = resources.lock().unwrap_or_else(|error| error.into_inner());
            if cancelled.load(Ordering::Acquire) { return Err("level unloaded".into()); }
            let sources = Sources::load_named_shared(vec![name.clone()], &catalog, programs, &mut resources);
            sources.effects.get(&name).ok_or_else(|| format!("effect {name} was not acquired"))?
                .as_ref().map_err(Clone::clone)?;
            if cancelled.load(Ordering::Acquire) { return Err("level unloaded".into()); }
            if let Some(started) = started {
                eprintln!("FX_LIFECYCLE source=io effect={name} definitions={} acquire_ms={:.3}",
                    sources.effects.len(), started.elapsed().as_secs_f64() * 1000.0);
            }
            Ok(Arc::new(sources))
        })));
    }

    fn poll(&mut self) {
        if self.loading.is_empty() { return; }
        self.completed.clear();
        let completed = &mut self.completed;
        self.loading.retain(|name, task| {
            let task = task.get_mut().unwrap_or_else(|error| error.into_inner());
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
                    }
                    self.revision = self.revision.wrapping_add(1);
                    self.cleanup_dirty = true;
                }
            }
        }
        self.cache.retain(|_, sources| sources.strong_count() != 0);
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
    if !owner.loading.is_empty() { owner.poll(); }
    let retired = prewarm.retired_signal.load(Ordering::Acquire);
    if retired != owner.seen_retired {
        owner.seen_retired = retired;
        owner.cleanup_dirty = true;
        owner.cache.retain(|_, source| source.strong_count() != 0);
    }
    if owner.cleanup.is_some() {
        let task = owner.cleanup.as_mut().unwrap().get_mut().unwrap_or_else(|error| error.into_inner());
        if bevy::tasks::block_on(bevy::tasks::futures_lite::future::poll_once(task)).is_some() { owner.cleanup = None; }
    }
    if owner.cleanup_dirty && owner.cleanup.is_none() {
        owner.cleanup_dirty = false;
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
    if let Some(mut interactions) = interactions.filter(|interactions| !interactions.pending.is_empty()) {
        for change in &mut interactions.pending {
            let actor = EffectOwner::Usable(change.actor);
            if change.stop_effect {
                owner.wanted.remove(&actor);
                if owner.ready.remove(&actor).is_some() { owner.retirement = owner.retirement.wrapping_add(1); }
                owner.placements.remove(&actor);
                owner.revision = owner.revision.wrapping_add(1);
                owner.cleanup_dirty = true;
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
            }
        }
    }
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
    let key = |owner: &EffectOwner| match owner {
        EffectOwner::Usable(index) => format!("usable:{index}"),
        EffectOwner::Player(name) => format!("player:{name}"),
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
            }
            owner.revision = owner.revision.wrapping_add(1);
            owner.retirement = owner.retirement.wrapping_add(1);
            owner.cleanup_dirty = true;
        }
    }
    if owner.base.is_some() && owner.published_revision == Some(owner.revision)
        && owner.published_active_revision == Some(owner.revision) { return; }
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
        let leases = Arc::new(owner.ready.values().map(Arc::downgrade).collect());
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
    if owner.published_active_revision == Some(owner.revision) { return; }
    let mut entries: Vec<DynamicEffect> = active.0.iter().filter(|entry| {
        owner.wanted.iter().any(|(id, wanted)| key(id) == entry.owner && wanted.revision == entry.revision)
    }).cloned().collect();
    let prewarmed = (prewarm.generation == 0 || prewarm.completed.load(Ordering::Acquire) == prewarm.generation)
        && (world_materials.generation == 0 || world_materials.completed.load(Ordering::Acquire) == world_materials.generation)
        && (decal_materials.generation == 0 || decal_materials.completed.load(Ordering::Acquire) == decal_materials.generation);
    if prewarmed {
        let audio = sounds.lock();
        for (id, wanted) in &owner.wanted {
            let name = key(id);
            if entries.iter().any(|entry| entry.owner == name && entry.revision == wanted.revision) { continue; }
            let Some(sources) = owner.ready.get(id) else { continue };
            if !Arc::ptr_eq(sources, &base) && !sources.particle_materials().flat_map(|material| material.images.values())
                .filter(|image| image.scene_owned).all(|image| images.handles.contains_key(&image.path)) { continue; }
            let mut sound_events = sources.effects.values().filter_map(|effect| effect.as_ref().ok())
                .flat_map(|effect| &effect.groups).flat_map(|group| &group.sounds)
                .flat_map(|sound| [&sound.start, &sound.stop]).flatten().peekable();
            let has_sound = sound_events.peek().is_some();
            if (has_sound && !audio.ready) || !sound_events.all(|event| audio.events.contains_key(&event.name)) { continue; }
            let (target, local, attached) = match id {
                EffectOwner::Player(_) => (EntityReference::Player, ReferenceFrame::default(), true),
                EffectOwner::Usable(index) => {
                    let Some(actor) = interactions.as_ref().and_then(|interactions| interactions.actors.get(*index)) else { continue };
                    let Some(placement) = owner.placements.get(id) else { continue };
                    (EntityReference::Named(actor.entity.clone()), ReferenceFrame { position: placement.origin, basis: placement.axis }, placement.attached)
                }
            };
            entries.push(DynamicEffect { owner: name, revision: wanted.revision, effect: wanted.path.clone(),
                target, local, attached, sources: sources.clone() });
        }
    }
    if entries.len() != active.0.len() || entries.iter().zip(active.0.iter()).any(|(left, right)| left.owner != right.owner || left.revision != right.revision) {
        active.0 = Arc::new(entries);
    }
    if active.0.len() == owner.wanted.len()
        || (prewarm.generation != 0 && (prewarm.failed.load(Ordering::Acquire) == prewarm.generation
            || world_materials.failed.load(Ordering::Acquire) == prewarm.generation
            || decal_materials.failed.load(Ordering::Acquire) == prewarm.generation)) {
        owner.published_active_revision = Some(owner.revision);
    }
}
