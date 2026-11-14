//! Canonical midnightScene identities and lazy immutable typed views.
//! Constructor193a10 owns declaration data, separate from5329c0's copied
//! keyframe playback and533460's graph action. Registry phases follow
//!0607f0/05feb0/0661e0/060b60/060180, with demand decoding as a host policy.
use crate::{archive::Catalog, assets::{entities::{midnight::MidnightScene,
    scene_motion::{MotionClip, AnimatedMotionClip}}, resource_registry::Registry}};
use std::{fmt, ops::Deref, sync::{Arc, Mutex, Weak}};

#[derive(Clone, Debug)]
pub(crate) enum Error { Absent(String), Source(String), Projection(String), Cancelled }
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent(path) => write!(f, "{path} is absent"),
            Self::Source(error) | Self::Projection(error) => f.write_str(error),
            Self::Cancelled => f.write_str("Midnight scene acquisition cancelled"),
        }
    }
}

/// Playback dereferences the typed payload directly; it does not acquire a
/// lock or consult the registry. The identity lease protects shared history.
pub struct SceneView<T> { data: Arc<T>, definition: Arc<Definition> }
impl<T> Deref for SceneView<T> { type Target = T; fn deref(&self) -> &T { &self.data } }
impl<T: fmt::Debug> fmt::Debug for SceneView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { fmt::Debug::fmt(self.data.as_ref(), f) }
}
impl<T: Clone> SceneView<T> {
    pub(crate) fn duplicate(&self) -> Arc<Self> {
        Arc::new(Self { data: Arc::new((*self.data).clone()), definition: Arc::clone(&self.definition) })
    }
}
pub type MotionScene = SceneView<MotionClip>;
pub type AnimatedScene = SceneView<AnimatedMotionClip>;
pub type PlayerScene = SceneView<MidnightScene>;

struct ProjectionState<T> { data: Option<Result<Arc<T>, Error>>, view: Option<Weak<SceneView<T>>> }
impl<T> Default for ProjectionState<T> {
    fn default() -> Self { Self { data: None, view: None } }
}
struct Projection<T>(Mutex<ProjectionState<T>>);
impl<T> Default for Projection<T> { fn default() -> Self { Self(Mutex::new(Default::default())) } }

pub(crate) struct Definition {
    path: Box<str>,
    motion: Projection<MotionClip>,
    animated: Projection<AnimatedMotionClip>,
    player: Projection<MidnightScene>,
}
impl Definition {
    fn new(path: &str) -> Self {
        Self { path: path.into(), motion: Default::default(), animated: Default::default(), player: Default::default() }
    }
}
fn active(cancelled: Option<&dyn Fn() -> bool>) -> Result<(), Error> {
    if cancelled.is_some_and(|closed| closed()) { Err(Error::Cancelled) } else { Ok(()) }
}
fn path(name: &str) -> String {
    let name = name.replace('\\', "/").to_ascii_lowercase();
    if name.starts_with("generated/decls/midnightscene/") { name }
    else { format!("generated/decls/midnightscene/{name}.decl") }
}
fn owner(catalog: &Catalog) -> &Mutex<Registry<Definition>> {
    catalog.midnight_scenes.get_or_init(Default::default)
}
fn projected<T>(definition: &Arc<Definition>, catalog: &Catalog, cancelled: Option<&dyn Fn() -> bool>,
    select: fn(&Definition) -> &Projection<T>, parse: fn(&str) -> Result<T, String>, view_name: &str)
    -> Result<Arc<SceneView<T>>, Error> {
    active(cancelled)?;
    let mut projection = select(definition).0.lock().map_err(|_| Error::Source("Midnight projection owner is poisoned".into()))?;
    active(cancelled)?;
    if let Some(view) = projection.view.as_ref().and_then(Weak::upgrade) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MIDNIGHT_VIEW_ACQUIRE path={} view={view_name} decoded=false identity={:p} payload={:p}",
                definition.path, Arc::as_ptr(definition), Arc::as_ptr(&view.data));
        }
        return Ok(view);
    }
    let mut decoded = false;
    let mut pending = None;
    if projection.data.is_none() {
        let _tag = crate::platform::allocation_tags::tag("Midnight scene declarations");
        let loaded = (|| {
            let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
            let read_start = trace.then(crate::loading::thread_cycles).flatten();
            let bytes = catalog.read_cancellable(&definition.path, cancelled)
                .map_err(|error| Error::Source(error.to_string()))?
                .ok_or_else(|| Error::Absent(definition.path.to_string()))?;
            active(cancelled)?;
            let read_cycles = read_start.zip(if trace { crate::loading::thread_cycles() } else { None }).map(|(a, b)| b.saturating_sub(a));
            decoded = true;
            let parse_start = trace.then(crate::loading::thread_cycles).flatten();
            let result = parse(&crate::entities::declaration_text(&bytes))
                .map(Arc::new).map_err(Error::Projection);
            active(cancelled)?;
            if trace {
                let parse_cycles = parse_start.zip(crate::loading::thread_cycles()).map(|(a, b)| b.saturating_sub(a));
                eprintln!("MIDNIGHT_VIEW_DECODE path={} view={view_name} source_bytes={} source_signature={} read_cycles={read_cycles:?} parse_cycles={parse_cycles:?} success={}",
                    definition.path, bytes.len(), blake3::hash(&bytes), result.is_ok());
            }
            result
        })();
        // Cancellation is withdrawal, not an immutable parse failure.
        active(cancelled)?;
        pending = Some(loaded);
    }
    let data = match pending.as_ref().or(projection.data.as_ref()).expect("initialized projection") {
        Ok(data) => Arc::clone(data),
        Err(error) => {
            let error = error.clone();
            active(cancelled)?;
            if pending.is_some() { projection.data = pending; }
            return Err(error);
        }
    };
    let view = Arc::new(SceneView { data, definition: Arc::clone(definition) });
    active(cancelled)?;
    if pending.is_some() { projection.data = pending; }
    projection.view = Some(Arc::downgrade(&view));
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MIDNIGHT_VIEW_ACQUIRE path={} view={view_name} decoded={decoded} identity={:p} payload={:p}",
            definition.path, Arc::as_ptr(definition), Arc::as_ptr(&view.data));
    }
    Ok(view)
}
fn acquire<T>(catalog: &Catalog, name: &str, cancelled: Option<&dyn Fn() -> bool>,
    select: fn(&Definition) -> &Projection<T>, parse: fn(&str) -> Result<T, String>, view_name: &str)
    -> Result<Arc<SceneView<T>>, Error> {
    active(cancelled)?;
    let path = path(name);
    let shared = !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_MIDNIGHT_RESOURCES").as_deref() != Ok("1");
    let mut prepared = None;
    let mut load = |_| {
        let definition = Arc::new(Definition::new(&path));
        prepared = Some(projected(&definition, catalog, cancelled, select, parse, view_name));
        Ok(definition)
    };
    let mut registry = owner(catalog).lock().map_err(|_| Error::Source("Midnight resource registry is poisoned".into()))?;
    let definition = if shared {
        registry.acquire_when(&path, 0, &mut load, || active(cancelled).is_ok())
    } else {
        registry.acquire_with_sharing(&path, 0, false, &mut load)
    }.map_err(Error::Source)?;
    drop(registry);
    active(cancelled)?;
    prepared.unwrap_or_else(|| projected(&definition, catalog, cancelled, select, parse, view_name))
}
pub(crate) fn acquire_motion(catalog: &Catalog, name: &str, cancelled: Option<&dyn Fn() -> bool>) -> Result<Arc<MotionScene>, Error> {
    acquire(catalog, name, cancelled, |definition| &definition.motion, crate::entities::scene_motion::parse_clip, "motion")
}
pub(crate) fn acquire_animated(catalog: &Catalog, name: &str, cancelled: Option<&dyn Fn() -> bool>) -> Result<Arc<AnimatedScene>, Error> {
    acquire(catalog, name, cancelled, |definition| &definition.animated, crate::entities::scene_motion::parse_animated_clip, "animated")
}
pub(crate) fn acquire_player(catalog: &Catalog, name: &str, cancelled: Option<&dyn Fn() -> bool>) -> Result<Arc<PlayerScene>, Error> {
    acquire(catalog, name, cancelled, |definition| &definition.player, crate::entities::midnight::parse, "player")
}
pub(crate) fn register_preload(catalog: &Catalog, name: &str, marker: u8, persistent: bool) -> Result<(), String> {
    owner(catalog).lock().map_err(|_| "Midnight resource registry is poisoned")?
        .register(&path(name), marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.midnight_scenes.get() {
        owner.lock().map_err(|_| "Midnight resource registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.midnight_scenes.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "midnightScene", marker);
        owner.report(catalog.identity(), "midnightScene", "scope-released", retained_bytes);
        report_views(&owner, catalog.identity(), "scope-released");
    }
}
pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.midnight_scenes.get() {
        let owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.report(catalog.identity(), "midnightScene", "catalog-retiring", retained_bytes);
        report_views(&owner, catalog.identity(), "catalog-retiring");
    }
}
fn report_views(owner: &Registry<Definition>, catalog: u64, phase: &str) {
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
    fn status<T>(value: &Projection<T>) -> [usize; 3] {
        let value = value.0.lock().unwrap_or_else(|error| error.into_inner());
        [usize::from(matches!(value.data, Some(Ok(_)))),
            usize::from(matches!(value.data, Some(Err(_)))),
            value.view.as_ref().map_or(0, Weak::strong_count)]
    }
    let mut views = [0; 3];
    let mut failures = [0; 3];
    let mut leases = 0;
    for definition in owner.live_values() {
        for (index, state) in [status(&definition.motion), status(&definition.animated), status(&definition.player)].into_iter().enumerate() {
            views[index] += state[0];
            failures[index] += state[1];
            leases += state[2];
        }
    }
    eprintln!("MIDNIGHT_OWNER_STORAGE catalog={catalog} phase={phase} headers={} motion_views={} animated_views={} player_views={} projection_failures={failures:?} view_leases={leases} retained_source_bytes=0",
        owner.len(), views[0], views[1], views[2]);
}
fn retained_bytes(definition: &Definition) -> usize {
    definition.path.len()
        + projection_bytes(&definition.motion, motion_bytes)
        + projection_bytes(&definition.animated, animated_bytes)
        + projection_bytes(&definition.player, player_bytes)
}
fn projection_bytes<T>(projection: &Projection<T>, nested: impl Fn(&T) -> usize) -> usize {
    let projection = projection.0.lock().unwrap_or_else(|error| error.into_inner());
    usize::from(projection.view.is_some()) * (2 * std::mem::size_of::<usize>() + std::mem::size_of::<SceneView<T>>())
        + match &projection.data {
        None => 0,
        Some(Err(Error::Absent(error) | Error::Source(error) | Error::Projection(error))) => error.capacity(),
        Some(Err(Error::Cancelled)) => 0,
        Some(Ok(data)) => 2 * std::mem::size_of::<usize>() + std::mem::size_of::<T>() + nested(data),
    }
}
fn vector_bytes<T>(values: &Vec<T>) -> usize { values.capacity() * std::mem::size_of::<T>() }
fn motion_bytes(clip: &MotionClip) -> usize {
    vector_bytes(&clip.groups) + clip.groups.iter().map(|group| group.name.capacity() + vector_bytes(&group.curve.keys)).sum::<usize>()
}
fn key_bytes(key: &crate::entities::midnight::AnimKey) -> usize {
    key.anim.capacity() + match &key.marker_values {
        Ok(markers) => vector_bytes(markers) + markers.iter().map(|marker| marker.name.capacity()).sum::<usize>(),
        Err(error) => error.capacity(),
    } + key.curve.as_ref().map_or(0, |curve| vector_bytes(&curve.times) + vector_bytes(&curve.values))
}
fn animated_bytes(clip: &AnimatedMotionClip) -> usize {
    motion_bytes(&clip.motion)
        + clip.animations.capacity() * std::mem::size_of::<(u32, Vec<crate::entities::midnight::AnimKey>)>()
        + clip.animations.values().map(|keys| vector_bytes(keys) + keys.iter().map(key_bytes).sum::<usize>()).sum::<usize>()
        + clip.motion_groups.capacity() * std::mem::size_of::<u32>()
}
fn player_bytes(scene: &MidnightScene) -> usize {
    vector_bytes(&scene.groups) + scene.groups.iter().map(|(_, name)| name.capacity()).sum::<usize>()
        + vector_bytes(&scene.nodes) + scene.nodes.iter().map(|node| node.group.capacity() + vector_bytes(&node.next)
            + node.next.iter().map(|(name, _)| name.capacity()).sum::<usize>()).sum::<usize>()
        + vector_bytes(&scene.scenes) + scene.scenes.iter().map(|scene| vector_bytes(&scene.groups) + scene.groups.iter().map(|group|
            vector_bytes(&group.specific_models) + group.specific_models.iter().map(String::capacity).sum::<usize>()
            + vector_bytes(&group.anim_keys) + group.anim_keys.iter().map(key_bytes).sum::<usize>()
            + vector_bytes(&group.events) + group.events.iter().map(|event| event.event.capacity()).sum::<usize>()
            + vector_bytes(&group.sounds) + group.sounds.iter().map(|sound| sound.event.capacity()).sum::<usize>()).sum::<usize>()).sum::<usize>()
}


/// Observe the actual declaration owner against an installed source, without
/// advancing gameplay, launching a window, or loading rig-bound resources.
pub(crate) fn observe_scopes(base: &std::path::Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = Catalog::open(base)?;
    let path = path(name);
    register_preload(&catalog, &path, 0, false)?;
    promote_bootstrap(&catalog)?;
    let motion = acquire_motion(&catalog, &path, None);
    let animated = acquire_animated(&catalog, &path, None);
    let player = acquire_player(&catalog, &path, None);
    let motion_again = acquire_motion(&catalog, &path, None);
    let animated_again = acquire_animated(&catalog, &path, None);
    let player_again = acquire_player(&catalog, &path, None);
    fn signature<T: fmt::Debug>(view: &Result<Arc<SceneView<T>>, Error>) -> String {
        match view {
            Ok(view) => blake3::hash(format!("{view:?}").as_bytes()).to_hex().to_string(),
            Err(error) => format!("{error:?}"),
        }
    }
    fn same<T>(a: &Result<Arc<SceneView<T>>, Error>, b: &Result<Arc<SceneView<T>>, Error>) -> Option<bool> {
        Some(Arc::ptr_eq(a.as_ref().ok()?, b.as_ref().ok()?))
    }
    eprintln!("MIDNIGHT_SCOPE path={path} stage=acquired motion={} animated={} player={} motion_shared={:?} animated_shared={:?} player_shared={:?}",
        signature(&motion), signature(&animated), signature(&player), same(&motion, &motion_again), same(&animated, &animated_again), same(&player, &player_again));
    let mut generations = Vec::new();
    if let Ok(view) = &motion { generations.push(Arc::downgrade(&view.definition)); }
    if let Ok(view) = &animated { generations.push(Arc::downgrade(&view.definition)); }
    if let Ok(view) = &player { generations.push(Arc::downgrade(&view.definition)); }
    drop((motion_again, animated_again, player_again));
    // An explicit persistent-map request overrides this bootstrap header.
    register_preload(&catalog, &path, 0, true)?;
    release_marker(&catalog, 0);
    release_marker(&catalog, 1);
    eprintln!("MIDNIGHT_SCOPE path={path} stage=purged-with-views live_generations={} history_owned={}",
        generations.iter().filter(|value| value.strong_count() != 0).count(),
        owner(&catalog).lock().map_err(|_| "Midnight registry is poisoned")?.owns_read_history(&path));
    drop(motion);
    release_marker(&catalog, 1);
    eprintln!("MIDNIGHT_SCOPE path={path} stage=motion-dropped live_generations={}",
        generations.iter().filter(|value| value.strong_count() != 0).count());
    drop((animated, player));
    release_marker(&catalog, 1);
    eprintln!("MIDNIGHT_SCOPE path={path} stage=all-dropped live_generations={} history_owned={}",
        generations.iter().filter(|value| value.strong_count() != 0).count(),
        owner(&catalog).lock().map_err(|_| "Midnight registry is poisoned")?.owns_read_history(&path));
    Ok(())
}

/// Withdraw immediately after the production archive reader credits its real
/// source read, then retry through the same owner. No synthetic resource data.
pub(crate) fn observe_read_cancellation(base: &std::path::Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let catalog = Catalog::open(base)?;
    let path = path(name);
    let kind = catalog.kind(&path).ok_or("Midnight source is absent")?;
    let read = || crate::loading::catalog_read_recorded(catalog.identity(), kind, &path);
    let withdrawn = acquire_player(&catalog, &path, Some(&read));
    eprintln!("MIDNIGHT_READ_CANCELLATION path={path} stage=withdrawn cancelled={} read={} history_owned={}",
        matches!(&withdrawn, Err(Error::Cancelled)), read(),
        owner(&catalog).lock().map_err(|_| "Midnight registry is poisoned")?.owns_read_history(&path));
    drop(withdrawn);
    release_marker(&catalog, 0);
    eprintln!("MIDNIGHT_READ_CANCELLATION path={path} stage=purged read={}", read());
    let retry = acquire_player(&catalog, &path, None);
    eprintln!("MIDNIGHT_READ_CANCELLATION path={path} stage=retry success={} error={:?}", retry.is_ok(), retry.as_ref().err());
    drop(retry);
    let motion = acquire_motion(&catalog, &path, None);
    let animated = acquire_animated(&catalog, &path, None);
    eprintln!("MIDNIGHT_READ_CANCELLATION path={path} stage=other-views motion={} animated={}", motion.is_ok(), animated.is_ok());
    drop((motion, animated));
    release_marker(&catalog, 0);
    Ok(())
}
