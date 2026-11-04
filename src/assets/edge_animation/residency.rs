//! Host resource identity and init/common lifetime for compressed clips.
//! 1400607f0/14005feb0 acquire; 140060180 purges map resources, separately
//! from 1400661e0's init/common acquisitions. 14024adf0 owns the sample blob.
use super::{AnimationData, EdgeAnimation};
use crate::assets::{archive::Catalog, map_resources::{self, Resource}};
use std::{collections::HashMap, path::PathBuf, sync::{Arc, Mutex, OnceLock, Weak}};

struct Entry {
    weak: Weak<AnimationData>,
    pinned: Option<EdgeAnimation>,
    signature: u32,
    source_bytes: usize,
}
struct Registry {
    base: PathBuf,
    catalog_identity: u64,
    lists: [Arc<Vec<Resource>>; 2],
    shared: Vec<(u8, u32)>,
    entries: HashMap<String, Entry>,
}
impl Registry {
    fn new(catalog: &Catalog) -> Result<Self, String> {
        let language = crate::assets::lang::read_text_language(catalog.base())?;
        let lists = [map_resources::acquire_list(catalog.base(), catalog, "init", language)?,
            map_resources::acquire_list(catalog.base(), catalog, "common", language)?];
        let mut shared: Vec<_> = lists.iter().enumerate().flat_map(|(list, entries)|
            entries.iter().enumerate().filter(|(_, entry)| entry.kind == "anim")
                .map(move |(index, _)| (list as u8, index as u32))).collect();
        shared.sort_unstable_by(|&(a, i), &(b, j)| lists[a as usize][i as usize].path.cmp(&lists[b as usize][j as usize].path));
        Ok(Self { base: catalog.base().to_owned(), catalog_identity: catalog.identity(), lists, shared, entries: Default::default() })
    }
    fn is_shared(&self, path: &str) -> bool {
        self.shared.binary_search_by(|&(list, index)|
            self.lists[list as usize][index as usize].path.as_str().cmp(path)).is_ok()
    }
}
static REGISTRY: Mutex<Option<Registry>> = Mutex::new(None);
fn separate() -> bool {
    static SEPARATE: OnceLock<bool> = OnceLock::new();
    *SEPARATE.get_or_init(|| std::env::var("DH2_DEBUG_SEPARATE_ANIMATION_RESOURCES").as_deref() == Ok("1"))
}
fn tracing() -> bool {
    static TRACE: OnceLock<bool> = OnceLock::new();
    *TRACE.get_or_init(|| std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1"))
}
fn record(path: &str, animation: &EdgeAnimation, signature: u32, source_bytes: usize, reused: bool) {
    if tracing() {
        eprintln!("MEMORY animation acquisition: path={path} reused={reused} identity={:p} source_bytes={source_bytes} retained_bytes={} frames={} rate={:?} flags={} properties={} signature={signature:08x}",
            Arc::as_ptr(&animation.0), animation.retained_bytes(), animation.frame_count,
            animation.frame_rate, animation.flags, animation.properties);
    }
}

/// The same resource may be validated against several rigs; validation stays
/// with each caller, while the serialized clip allocation remains immutable.
pub fn acquire(catalog: &Catalog, name: &str) -> Result<EdgeAnimation, String> {
    let path = name.replace('\\', "/").to_ascii_lowercase();
    if !separate() {
        let needs_owner = REGISTRY.lock().unwrap().as_ref().is_none_or(|owner| owner.base != catalog.base() || owner.catalog_identity != catalog.identity());
        if needs_owner {
            // No archive/declaration IO while holding the resource mutex.
            let owner = Registry::new(catalog)?;
            let mut current = REGISTRY.lock().unwrap();
            if current.as_ref().is_none_or(|owner| owner.base != catalog.base() || owner.catalog_identity != catalog.identity()) { *current = Some(owner); }
        }
        let owner = REGISTRY.lock().unwrap();
        if let Some(entry) = owner.as_ref().filter(|owner| owner.base == catalog.base() && owner.catalog_identity == catalog.identity()).and_then(|owner| owner.entries.get(&path)) {
            if let Some(data) = entry.weak.upgrade() {
                let animation = EdgeAnimation(data);
                record(&path, &animation, entry.signature, entry.source_bytes, true);
                return Ok(animation);
            }
        }
    }
    let bytes = catalog.read(&path).map_err(|error| error.to_string())?
        .ok_or_else(|| format!("{path} is absent"))?;
    let source_bytes = bytes.len();
    let signature = if tracing() { let mut crc = flate2::Crc::new(); crc.update(&bytes); crc.sum() } else { 0 };
    let animation = if separate() { EdgeAnimation::parse(&bytes) } else { EdgeAnimation::parse_owned(bytes) }
        .map_err(|error| format!("{path}: {error}"))?;
    if separate() {
        record(&path, &animation, signature, source_bytes, false);
        return Ok(animation);
    }
    let mut current = REGISTRY.lock().unwrap();
    let owner = current.as_mut().filter(|owner| owner.base == catalog.base() && owner.catalog_identity == catalog.identity())
        .ok_or("animation installation changed during acquisition")?;
    if let Some(entry) = owner.entries.get(&path) {
        if let Some(data) = entry.weak.upgrade() {
            let animation = EdgeAnimation(data);
            record(&path, &animation, entry.signature, entry.source_bytes, true);
            return Ok(animation);
        }
    }
    let pinned = owner.is_shared(&path).then(|| animation.clone());
    owner.entries.insert(path.clone(), Entry {weak: Arc::downgrade(&animation.0), pinned, signature, source_bytes});
    record(&path, &animation, signature, source_bytes, false);
    Ok(animation)
}

pub(crate) fn release_level() {
    let mut current = REGISTRY.lock().unwrap();
    let Some(owner) = current.as_mut() else { return; };
    let before = owner.entries.len();
    let mut forgotten = 0;
    let mut forgotten_bytes = 0;
    owner.entries.retain(|path, entry| {
        let keep = entry.pinned.is_some() || entry.weak.strong_count() != 0;
        if !keep {
            if let Some(bytes) = crate::loading::forget("anim", path) {
                forgotten += 1;
                forgotten_bytes += bytes;
            }
        }
        keep
    });
    if tracing() {
        eprintln!("MEMORY animation history release: entries={forgotten} path_capacity_bytes={forgotten_bytes}");
        eprintln!("MEMORY animation purge: removed={} retained_common={} retained_common_bytes={} live_map_leases={}",
            before - owner.entries.len(), owner.entries.values().filter(|entry| entry.pinned.is_some()).count(),
            owner.entries.values().filter_map(|entry| entry.pinned.as_ref()).map(EdgeAnimation::retained_bytes).sum::<usize>(),
            owner.entries.values().filter(|entry| entry.pinned.is_none() && entry.weak.strong_count() != 0).count());
    }
}
