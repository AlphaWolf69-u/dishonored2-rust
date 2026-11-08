//! Clip resource records and immutable playback snapshots.
//! 0607f0 creates/loads; 05feb0 only resolves an existing record.
//! 24adf0 owns the installed sample blob. The installed 05fde0 static
//! registration roster contains no animation-class references.
use super::{AnimationData, EdgeAnimation};
use crate::assets::{archive::Catalog, map_resources::{self, Resource}};
use std::{borrow::Cow, collections::HashMap, sync::{Arc, Mutex, OnceLock, Weak}};

#[derive(Default)]
struct Snapshot {
    weak: Weak<AnimationData>,
    pinned: Option<EdgeAnimation>,
    signature: u32,
    source_bytes: usize,
    retain_until_purge: bool,
    failure: Option<Arc<str>>,
    previous: Vec<Weak<AnimationData>>,
}

/// Header identity survives an unloaded/evicted snapshot. Holding this Arc
/// across acquisition also prevents a concurrent purge from replacing it.
struct Record {
    scope: ResourceScope,
    snapshot: Mutex<Snapshot>,
}
impl Record {
    fn new(scope: ResourceScope) -> Self {
        Self { scope, snapshot: Mutex::new(Snapshot::default()) }
    }
}

// Host: retain the existing common-list reuse policy separately from actual
// record admission. Membership alone never makes find_existing succeed.
struct SharedResources {
    lists: [Arc<Vec<Resource>>; 2],
    indices: Vec<(u8, u32)>,
}
impl SharedResources {
    fn new(catalog: &Catalog) -> Result<Self, String> {
        let language = crate::assets::lang::read_text_language(catalog.base())?;
        let lists = [map_resources::acquire_list(catalog.base(), catalog, "init", language)?,
            map_resources::acquire_list(catalog.base(), catalog, "common", language)?];
        let mutable = catalog.has_overlays() &&
            (catalog.has_overlay_file("generated/buildgame/init.mapresources")
                || catalog.has_overlay_file("generated/buildgame/common.mapresources"));
        let mut indices: Vec<_> = if mutable { Vec::new() } else {
            lists.iter().enumerate().flat_map(|(list, entries)|
                entries.iter().enumerate().filter(|(_, entry)| entry.kind == "anim")
                    .map(move |(index, _)| (list as u8, index as u32))).collect()
        };
        indices.sort_unstable_by(|&(a, i), &(b, j)|
            lists[a as usize][i as usize].path.cmp(&lists[b as usize][j as usize].path));
        Ok(Self { lists, indices })
    }
    fn contains(&self, path: &str) -> bool {
        self.indices.binary_search_by(|&(list, index)|
            self.lists[list as usize][index as usize].path.as_str().cmp(path)).is_ok()
    }
}

#[derive(Default)]
pub(crate) struct Registry {
    shared: Option<SharedResources>,
    records: HashMap<String, Arc<Record>>,
}

fn ensure_registry(catalog: &Catalog) -> Result<(), String> {
    let mut owner = catalog.animation_resources.lock().map_err(|_| "animation resource owner is poisoned")?;
    if owner.is_none() { *owner = Some(Registry::default()); }
    Ok(())
}

fn ensure_shared(catalog: &Catalog) -> Result<(), String> {
    ensure_registry(catalog)?;
    let needed = catalog.animation_resources.lock().map_err(|_| "animation resource owner is poisoned")?
        .as_ref().is_some_and(|owner| owner.shared.is_none());
    if needed {
        // Metadata IO stays outside the registry lock. Static-reference
        // registration and failed existing-record lookups do not read lists.
        let shared = SharedResources::new(catalog)?;
        let mut current = catalog.animation_resources.lock().map_err(|_| "animation resource owner is poisoned")?;
        let owner = current.as_mut().ok_or("animation resource owner is absent")?;
        if owner.shared.is_none() { owner.shared = Some(shared); }
    }
    Ok(())
}

pub(crate) fn retire_catalog(owner: Option<Registry>) -> usize {
    // Playback snapshots may outlive the retiring catalog safely.
    owner.as_ref().map_or(0, |owner| owner.records.len())
}

fn separate() -> bool {
    static SEPARATE: OnceLock<bool> = OnceLock::new();
    *SEPARATE.get_or_init(|| std::env::var("DH2_DEBUG_SEPARATE_ANIMATION_RESOURCES").as_deref() == Ok("1"))
}
fn tracing() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"))
}
fn normalize(name: &str) -> Cow<'_, str> {
    if name.bytes().all(|byte| byte != b'\\' && !byte.is_ascii_uppercase()) {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.replace('\\', "/").to_ascii_lowercase())
    }
}
fn report(path: &str, animation: &EdgeAnimation, signature: u32, source_bytes: usize, reused: bool) {
    if tracing() {
        eprintln!("MEMORY animation acquisition: path={path} reused={reused} identity={:p} source_bytes={source_bytes} retained_bytes={} frames={} rate={:?} flags={} properties={} signature={signature:08x}",
            Arc::as_ptr(&animation.0), animation.retained_bytes(), animation.frame_count,
            animation.frame_rate, animation.flags, animation.properties);
    }
}

fn existing(catalog: &Catalog, path: &str) -> Result<Option<Arc<Record>>, String> {
    let current = catalog.animation_resources.lock().map_err(|_| "animation resource owner is poisoned")?;
    Ok(current.as_ref().and_then(|owner| owner.records.get(path)).cloned())
}

/// A dictionary retains an actual resource record even when loading failed.
/// 191dc0 stores 05feb0's nonnull record after 257330's returning diagnostic;
/// failure is neither a missing alias nor permission to invent clip samples.
/// Playback borrows the immutable decoded snapshot, without locking a record.
#[derive(Clone)]
pub(crate) struct ResourceLease {
    record: Arc<Record>,
    value: Result<EdgeAnimation, Arc<str>>,
}
impl ResourceLease {
    pub(crate) fn decoded(&self) -> Result<&EdgeAnimation, &str> {
        self.value.as_ref().map_err(|error| error.as_ref())
    }
    pub(crate) fn identity(&self) -> usize { Arc::as_ptr(&self.record) as usize }
}
impl std::fmt::Debug for ResourceLease {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("AnimationResource").field("identity", &self.identity())
            .field("decoded", &self.value.is_ok()).field("error", &self.value.as_ref().err()).finish()
    }
}

fn lease(catalog: &Catalog, path: &str, record: Arc<Record>, duplicate: bool) -> Result<ResourceLease, String> {
    let value = match load(catalog, path, &record, duplicate, false) {
        Ok(animation) => Ok(animation),
        Err(LoadFailure::Record(error)) => Err(error),
        Err(LoadFailure::Owner(error)) => return Err(error),
    };
    Ok(ResourceLease { record, value })
}

/// 257330's creating acquisition followed by 05feb0's record lookup. Native
/// load diagnostics may return; the resource header still belongs to its list.
pub(crate) fn acquire_record(catalog: &Catalog, name: &str) -> Result<ResourceLease, String> {
    let path = normalize(name);
    let record = create(catalog, &path, None)?;
    lease(catalog, &path, record, separate())
}

/// Unlike a decoded-clip lookup, this preserves a registered failed record.
/// An absent record still returns None without metadata or payload IO.
pub(crate) fn find_record(catalog: &Catalog, name: &str) -> Result<Option<ResourceLease>, String> {
    let path = normalize(name);
    let Some(record) = existing(catalog, &path)? else { return Ok(None) };
    lease(catalog, &path, record, false).map(Some)
}

fn create(catalog: &Catalog, path: &str, scope: Option<ResourceScope>) -> Result<Arc<Record>, String> {
    ensure_registry(catalog)?;
    if let Some(record) = existing(catalog, path)? { return Ok(record); }
    let mutable = catalog.has_overlay_file(path);
    if scope.is_none() && !mutable { ensure_shared(catalog)?; }
    let mut current = catalog.animation_resources.lock().map_err(|_| "animation resource owner is poisoned")?;
    let owner = current.as_mut().ok_or("animation resource owner is absent")?;
    if let Some(record) = owner.records.get(path) { return Ok(record.clone()); }
    let scope = scope.unwrap_or_else(|| {
        if !mutable && owner.shared.as_ref().is_some_and(|shared| shared.contains(path)) {
            ResourceScope::Common
        } else { ResourceScope::Level }
    });
    let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("animation records"));
    let record = Arc::new(Record::new(scope));
    owner.records.insert(path.to_owned(), record.clone());
    if tracing() {
        eprintln!("MEMORY animation record registration: path={path} common={} entries={} record_bytes={} map_entry_bytes={}",
            scope == ResourceScope::Common, owner.records.len(),
            std::mem::size_of::<Record>() + 2 * std::mem::size_of::<usize>(),
            std::mem::size_of::<(String, Arc<Record>)>());
    }
    Ok(record)
}

/// 05feb0: absence is a normal result, not permission to create a resource.
/// A registered lazy header loads through the same owner as ordinary acquire.
pub(crate) fn find_existing(catalog: &Catalog, name: &str) -> Result<Option<EdgeAnimation>, String> {
    let path = normalize(name);
    let Some(record) = existing(catalog, &path)? else {
        if tracing() { eprintln!("MEMORY animation record lookup: path={path} registered=false"); }
        return Ok(None);
    };
    load(catalog, &path, &record, false, false).map(Some).map_err(LoadFailure::into_string)
}

/// 0607f0's creating acquisition. Validation against a rig remains caller-owned;
/// clock/pose state never enters a resource record.
pub fn acquire(catalog: &Catalog, name: &str) -> Result<EdgeAnimation, String> {
    acquire_scoped(catalog, name, None)
}
fn acquire_scoped(catalog: &Catalog, name: &str, scope: Option<ResourceScope>) -> Result<EdgeAnimation, String> {
    let path = normalize(name);
    let record = create(catalog, &path, scope)?;
    load(catalog, &path, &record, separate() && scope.is_none(), scope.is_some()).map_err(LoadFailure::into_string)
}

enum LoadFailure {
    Record(Arc<str>),
    Owner(String),
}
impl LoadFailure {
    fn into_string(self) -> String {
        match self { Self::Record(error) => error.to_string(), Self::Owner(error) => error }
    }
}

fn load(catalog: &Catalog, path: &str, record: &Arc<Record>, duplicate: bool, retain: bool) -> Result<EdgeAnimation, LoadFailure> {
    // Edge decoding has no foreign-resource callbacks. A per-record lock
    // coalesces its IO/parse without holding the whole class registry during IO;
    // playback still holds only its immutable AnimationData Arc.
    let mut snapshot = record.snapshot.lock().map_err(|_| LoadFailure::Owner("animation record is poisoned".into()))?;
    snapshot.retain_until_purge |= retain;
    let mutable = catalog.has_overlay_file(path);
    if !mutable && !duplicate {
        if let Some(data) = snapshot.weak.upgrade() {
            let animation = EdgeAnimation(data);
            if snapshot.retain_until_purge && snapshot.pinned.is_none() { snapshot.pinned = Some(animation.clone()); }
            report(path, &animation, snapshot.signature, snapshot.source_bytes, true);
            return Ok(animation);
        }
        if let Some(error) = &snapshot.failure { return Err(LoadFailure::Record(error.clone())); }
    }
    if mutable || duplicate {
        let old = std::mem::take(&mut snapshot.weak);
        snapshot.pinned = None;
        if old.strong_count() != 0 { snapshot.previous.push(old); }
    }
    snapshot.previous.retain(|generation| generation.strong_count() != 0);
    let decoded = (|| {
        let bytes = catalog.read(path).map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{path} is absent"))?;
        let source_bytes = bytes.len();
        let signature = if tracing() { let mut crc = flate2::Crc::new(); crc.update(&bytes); crc.sum() } else { 0 };
        let animation = if duplicate { EdgeAnimation::parse(&bytes) } else { EdgeAnimation::parse_owned(bytes) }
            .map_err(|error| format!("{path}: {error}"))?;
        Ok::<_, String>((animation, signature, source_bytes))
    })();
    let (animation, signature, source_bytes) = match decoded {
        Ok(decoded) => decoded,
        Err(error) => {
            let error: Arc<str> = Arc::from(error);
            snapshot.failure = Some(error.clone());
            return Err(LoadFailure::Record(error));
        }
    };
    snapshot.failure = None;
    snapshot.signature = signature;
    snapshot.source_bytes = source_bytes;
    snapshot.weak = Arc::downgrade(&animation.0);
    if snapshot.retain_until_purge || record.scope == ResourceScope::Common {
        snapshot.pinned = Some(animation.clone());
    }
    report(path, &animation, signature, source_bytes, false);
    Ok(animation)
}

/// Host session/map retirement domains. These do not encode idResource +24:
/// the original's ordinary/persistent/permanent marker has separate producers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResourceScope { Level, Common }

/// 257390: the resource owner, not an alias value, retains a forced clip.
/// A hash lookup replaces the former linear scan of two strong-lease vectors.
pub(crate) fn force_load(catalog: &Catalog, name: &str, scope: ResourceScope) -> Result<(), String> {
    acquire_scoped(catalog, name, Some(scope)).map(drop)
}

pub(crate) fn release_level(catalog: &Catalog) {
    let mut current = catalog.animation_resources.lock().unwrap_or_else(|error| error.into_inner());
    let Some(owner) = current.as_mut() else { return };
    let before = owner.records.len();
    let mut forgotten = 0;
    let mut forgotten_bytes = 0;
    let mut live_map = 0;
    owner.records.retain(|path, record| {
        let mut snapshot = record.snapshot.lock().unwrap_or_else(|error| error.into_inner());
        snapshot.previous.retain(|generation| generation.strong_count() != 0);
        if record.scope == ResourceScope::Common { return true; }
        snapshot.retain_until_purge = false;
        snapshot.pinned = None;
        let keep = Arc::strong_count(record) > 1 || snapshot.weak.strong_count() != 0 || !snapshot.previous.is_empty();
        if keep { live_map += 1; }
        else if let Some(bytes) = crate::loading::forget(catalog.identity(), "anim", path) {
            forgotten += 1;
            forgotten_bytes += bytes;
        }
        keep
    });
    if tracing() {
        let mut common = 0;
        let mut common_bytes = 0;
        for record in owner.records.values().filter(|record| record.scope == ResourceScope::Common) {
            common += 1;
            if let Some(animation) = &record.snapshot.lock().unwrap_or_else(|error| error.into_inner()).pinned {
                common_bytes += animation.retained_bytes();
            }
        }
        eprintln!("MEMORY animation history release: entries={forgotten} path_capacity_bytes={forgotten_bytes}");
        eprintln!("MEMORY animation purge: removed={} retained_common={common} retained_common_bytes={common_bytes} live_map_leases={live_map}",
            before - owner.records.len());
    }
}
