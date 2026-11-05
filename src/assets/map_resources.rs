//! The resources a map loads before play (`ResourceManager.cpp`).
//!
//! `FUN_1400661e0` runs for `init`, `common` and the map, in that order, and
//! loads every entry of `generated/buildGame/<name>.mapResources` by type and
//! name (`thunk_FUN_1400607f0`); an image's LoadResourceInternal
//! (`1402ea140 -> 1402e9ff0`) uploads its base mip chain then. The file holds
//! a big-endian header (the entry count at +8) and 28-byte entries whose
//! first big-endian word is a resource id. Ids number the records of the
//! index of the archive that holds the `.mapResources` file's bytes; a localized decl
//! has one record per language under the same id.
use super::archive::index::{self, IndexEntry};
use super::archive::Catalog;
use std::collections::{HashMap, HashSet};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;

const HEADER: usize = 12;
const ENTRY: usize = 28;

/// One resource of a load list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub kind: String,
    pub path: String,
}

pub(crate) fn descriptor_bytes(list: &Vec<Resource>) -> usize {
    list.capacity() * std::mem::size_of::<Resource>()
        + list.iter().map(|resource| resource.kind.capacity() + resource.path.capacity()).sum::<usize>()
}

/// The ids of a `.mapResources` file, in load order.
pub fn ids(bytes: &[u8]) -> Result<Vec<u32>> {
    let count = bytes.get(8..HEADER).ok_or("map resources header is truncated")?;
    let count = u32::from_be_bytes(count.try_into().unwrap()) as usize;
    if bytes.len() != HEADER + count * ENTRY {
        return Err(format!("map resources hold {} bytes for {count} entries", bytes.len()));
    }
    Ok((0..count).map(|i| u32::from_be_bytes(bytes[HEADER + i * ENTRY..][..4].try_into().unwrap())).collect())
}

/// The load list of `name` (`init`, `common` or a map path such as
/// `campaign/edge_of_the_world/eotw_p_lowchaos`) for `language`.
pub fn load_list(base: &Path, catalog: &Catalog, name: &str, language: &str) -> Result<Vec<Resource>> {
    let manifest = format!("generated/buildgame/{}.mapresources", name.to_ascii_lowercase());
    // The manifest's record in the patched index names the archive holding
    // its bytes; the ids are that archive's index numbering (an unchanged
    // manifest keeps the base numbering, which the patch index renumbers).
    let (group, slot) = catalog.resource_container(&manifest)
        .ok_or_else(|| format!("{manifest} is not indexed"))?;
    let bytes = catalog.read(&manifest).map_err(|e| e.to_string())?.ok_or_else(|| format!("{manifest} is missing"))?;
    let requested = ids(&bytes)?;
    let wanted: HashSet<_> = requested.iter().copied().collect();
    let mut records: HashMap<u32, Vec<IndexEntry>> = HashMap::new();
    let path = index::slot_index_path(base, group, slot).map_err(|e| e.to_string())?;
    index::visit_file_ids(&path, group, &wanted, |r| records.entry(r.id).or_default().push(r)).map_err(|e| e.to_string())?;
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY manifest metadata: manifest={manifest} discovery_scans=0 requested_ids={} retained_ids={} retained_records={}",
            wanted.len(), records.len(), records.values().map(Vec::len).sum::<usize>());
    }
    let localized = format!("/localized/{language}/");
    let mut list = Vec::new();
    let mut missing = Vec::new();
    for id in requested {
        // The patched `common` list names one id (2789) its index lacks.
        let Some(entries) = records.get(&id) else {
            missing.push(id);
            continue;
        };
        let chosen: Vec<&IndexEntry> = if entries.iter().any(|e| e.path.contains("/localized/")) {
            entries.iter().filter(|e| e.path.contains(&localized)).collect()
        } else {
            entries.iter().collect()
        };
        list.extend(chosen.into_iter().map(|e| {
            let mut path = e.path.clone();
            path.make_ascii_lowercase();
            if path.contains('\\') { path = path.replace('\\', "/"); }
            Resource { kind: e.kind.clone(), path }
        }));
    }
    if !missing.is_empty() {
        eprintln!("{manifest}: ids without an index record: {missing:?}");
    }
    Ok(list)
}

/// The map's load lists in load order: `init`, `common`, then the map.
pub fn level_load_list(base: &Path, catalog: &Catalog, map: &str, language: &str) -> Result<Vec<Resource>> {
    let mut list = Vec::new();
    for name in ["init", "common", map] {
        list.extend(acquire_list(base, catalog, name, language)?.iter().cloned());
    }
    Ok(list)
}

/// Bootstrap manifests are shared for the session. A map's temporary request
/// table belongs to acquisition (1400661e0), not the registered resources.
pub fn acquire_list(base: &Path, catalog: &Catalog, name: &str, language: &str) -> Result<std::sync::Arc<Vec<Resource>>> {
    type Key = (u64, std::path::PathBuf, String, String);
    static LISTS: std::sync::Mutex<Option<HashMap<Key, std::sync::Arc<Vec<Resource>>>>> = std::sync::Mutex::new(None);
    let normalized = name.to_ascii_lowercase();
    // A loose manifest can change without changing the catalog snapshot.
    let manifest = format!("generated/buildgame/{normalized}.mapresources");
    if catalog.has_overlay_file(&manifest) {
        return Ok(std::sync::Arc::new(load_list(base, catalog, name, language)?));
    }
    let key = (catalog.identity(), base.to_owned(), normalized, language.to_ascii_lowercase());
    let mut lists = LISTS.lock().unwrap();
    let lists = lists.get_or_insert_with(Default::default);
    // Existing callers keep their Arc; the owner no longer pins another catalog.
    lists.retain(|(identity, _, _, _), _| *identity == catalog.identity());
    if let Some(list) = lists.get(&key) { return Ok(list.clone()); }
    let list = std::sync::Arc::new(load_list(base, catalog, name, language)?);
    if matches!(name, "init" | "common")
        || std::env::var("DH2_DEBUG_RETAIN_LEVEL_MANIFESTS").as_deref() == Ok("1") {
        lists.insert(key, list.clone());
    }
    Ok(list)
}

/// Three immutable request lists in authored load order. Common and init
/// descriptors remain owned by their session caches; this view owns only the
/// section leases and the level list, not a second copy of their strings.
pub struct LevelList {
    sections: [std::sync::Arc<Vec<Resource>>; 3],
}
impl Default for LevelList {
    fn default() -> Self {
        let empty = std::sync::Arc::new(Vec::new());
        Self { sections: [empty.clone(), empty.clone(), empty] }
    }
}
impl LevelList {
    fn acquire(base: &Path, catalog: &Catalog, map: &str, language: &str) -> Result<Self> {
        Ok(Self { sections: [acquire_list(base, catalog, "init", language)?,
            acquire_list(base, catalog, "common", language)?,
            acquire_list(base, catalog, map, language)?] })
    }
    pub fn iter(&self) -> impl Iterator<Item = &Resource> {
        self.sections[0].iter().chain(self.sections[1].iter()).chain(self.sections[2].iter())
    }
    pub fn len(&self) -> usize { self.sections.iter().map(|section| section.len()).sum() }
    pub fn is_empty(&self) -> bool { self.sections.iter().all(|section| section.is_empty()) }
    /// Counts section ownership; callers must account for shared Arc leases.
    pub(crate) fn logical_descriptor_bytes(&self) -> usize {
        self.sections.iter().map(|section| descriptor_bytes(section)).sum()
    }
}
impl std::ops::Index<usize> for LevelList {
    type Output = Resource;
    fn index(&self, index: usize) -> &Self::Output {
        let init = self.sections[0].len();
        if index < init { return &self.sections[0][index]; }
        let index = index - init;
        let common = self.sections[1].len();
        if index < common { &self.sections[1][index] }
        else { &self.sections[2][index - common] }
    }
}

/// The resource manager's already prepared map request list (1400661e0).
/// Share the immutable request descriptors between consumers; the registered
/// resource owners decide whether each payload is already resident.
pub fn acquire_level_list(base: &Path, catalog: &Catalog, map: &str, language: &str) -> Result<std::sync::Arc<LevelList>> {
    // A live loose manifest may change while another consumer owns an older
    // combined list. Preserve that old lease but do not reuse it for new reads.
    if catalog.has_overlays() && ["init", "common", map].iter().any(|name|
        catalog.has_overlay_file(&format!("generated/buildgame/{}.mapresources", name.to_ascii_lowercase()))) {
        return Ok(std::sync::Arc::new(LevelList::acquire(base, catalog, map, language)?));
    }
    type Key = (u64, std::path::PathBuf, String, String);
    // 1400661e0 releases its assembled request table after acquisition. Source
    // manifests above remain reusable metadata; a combined view belongs only
    // to its active consumers, not every map visited during this session.
    type Cached = (std::sync::Weak<LevelList>, Option<std::sync::Arc<LevelList>>);
    static LISTS: std::sync::Mutex<Option<HashMap<Key, Cached>>> = std::sync::Mutex::new(None);
    let key = (catalog.identity(), base.to_owned(), map.to_ascii_lowercase(), language.to_ascii_lowercase());
    let mut lists = LISTS.lock().unwrap();
    let lists = lists.get_or_insert_with(Default::default);
    lists.retain(|(identity, _, _, _), (list, _)| *identity == catalog.identity() && list.strong_count() != 0);
    if let Some(list) = lists.get(&key).and_then(|(list, _)| list.upgrade()) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("LOAD manifest reuse: {map} identity={:p}", std::sync::Arc::as_ptr(&list));
        }
        return Ok(list);
    }
    let list = std::sync::Arc::new(LevelList::acquire(base, catalog, map, language)?);
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("LOAD manifest acquire: {map} identity={:p} entries={}", std::sync::Arc::as_ptr(&list), list.len());
    }
    let retained = (std::env::var("DH2_DEBUG_RETAIN_LEVEL_MANIFESTS").as_deref() == Ok("1")).then(|| list.clone());
    lists.insert(key, (std::sync::Arc::downgrade(&list), retained));
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        let bytes = lists.values().filter_map(|(list, _)| list.upgrade()).map(|list| list.logical_descriptor_bytes()).sum::<usize>();
        eprintln!("MEMORY combined manifests: lists={} logical_section_bytes={bytes}", lists.len());
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed lists resolve completely, and the map's list holds its
    /// navmesh and entities (its world, tome, cfg and sound propagation have
    /// later ids: the map loader reads them, not the list).
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_lists_resolve() {
        let base = crate::assets::installation::game_base().unwrap();
        let language = crate::assets::lang::read_text_language(&base).unwrap();
        let catalog = Catalog::open(&base).unwrap();
        let map = load_list(&base, &catalog, "campaign/edge_of_the_world/eotw_p_lowchaos", language).unwrap();
        for suffix in ["eotw_p_lowchaos.bnavmesh", "eotw_p_lowchaos.entities"] {
            assert!(map.iter().any(|r| r.path.ends_with(suffix)), "{suffix}");
        }
        let all = level_load_list(&base, &catalog, "campaign/edge_of_the_world/eotw_p_lowchaos", language).unwrap();
        let images = all.iter().filter(|r| r.kind == "image").count();
        println!("{} resources, {images} images", all.len());
        assert!(all.iter().all(|r| !r.path.contains("/localized/") || r.path.contains(&format!("/localized/{language}/"))));
    }
}
