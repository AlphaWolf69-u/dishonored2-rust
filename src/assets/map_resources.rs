//! The resources a map loads before play (`ResourceManager.cpp`).
//!
//! `FUN_1400661e0` runs for `init`, `common` and the map, in that order, and
//! selects entries of `generated/buildGame/<name>.mapResources` using their
//! layer masks, registers eligible headers, then loads them by type and name
//! (`thunk_FUN_1400607f0`); an image's LoadResourceInternal
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

type ListKey = (u64, std::path::PathBuf, String, String);
static SOURCE_LISTS: std::sync::Mutex<Option<HashMap<ListKey, std::sync::Arc<SourceList>>>> = std::sync::Mutex::new(None);
type CachedLevel = (std::sync::Weak<LevelList>, Option<std::sync::Arc<LevelList>>);
static LEVEL_LISTS: std::sync::Mutex<Option<HashMap<ListKey, CachedLevel>>> = std::sync::Mutex::new(None);

/// Session request metadata belongs to its archive namespace. Live loading
/// consumers retain their section leases independently of these cache entries.
pub(crate) fn retire_catalog(catalog: u64) -> usize {
    let sources = {
        let mut owner = SOURCE_LISTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(lists) = owner.as_mut() {
            let before = lists.len();
            lists.retain(|(identity, _, _, _), _| *identity != catalog);
            let removed = before - lists.len();
            if lists.is_empty() { *owner = None; }
            removed
        } else {
            0
        }
    };
    sources + retire_level_lists(catalog)
}

fn retire_level_lists(catalog: u64) -> usize {
    let mut owner = LEVEL_LISTS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(lists) = owner.as_mut() else { return 0; };
    let before = lists.len();
    lists.retain(|(identity, _, _, _), _| *identity != catalog);
    let removed = before - lists.len();
    if lists.is_empty() { *owner = None; }
    removed
}

const HEADER: usize = 12;
const ENTRY: usize = 28;

/// 0661e0 uses the same declaration-family admission for primary/reuse
/// records, masks, and HeaderSelection publication. Keep these stages in
/// agreement as each real class owner joins the existing preload pass.
fn preload_declaration_kind(kind: &str) -> bool {
    kind.eq_ignore_ascii_case("anim") || kind.eq_ignore_ascii_case("animset")
        || kind.eq_ignore_ascii_case("md6def") || kind.eq_ignore_ascii_case("inventoryLoadout")
        || kind.eq_ignore_ascii_case("inventoryItemCollection") || kind.eq_ignore_ascii_case("pblock")
        || kind.eq_ignore_ascii_case("inventoryitem")
}

/// One resource of a load list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub kind: String,
    pub path: String,
}

/// Compact references into the existing resource list, in container-header
/// order. Masks belong to the authored mapResources entries, not the archive
/// index; repeated requests for the same ID select the union of their layers.
#[derive(Clone)]
pub(crate) struct HeaderSelection {
    pub(crate) resource: u32,
    pub(crate) mask: [u8; 24],
    pub(crate) flags: u16,
}

struct SourceList {
    catalog_identity: u64,
    /// Mounted primary header table, not the archive storing manifest bytes.
    header_container: u16,
    resources: std::sync::Arc<Vec<Resource>>,
    headers: Vec<HeaderSelection>,
    reuse: std::sync::OnceLock<Result<Option<ReuseSelection>>>,
    reuse_path: Option<String>,
    language: Box<str>,
}

struct ReuseSelection {
    // Only paths absent from the primary section need their own retained value.
    additional: Vec<Resource>,
    headers: Vec<ReuseHeader>,
}

struct ReuseHeader {
    primary: bool,
    selection: HeaderSelection,
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
    let source = load_source_list(base, catalog, name, language)?;
    std::sync::Arc::try_unwrap(source.resources).map_err(|_| "new manifest resource list is shared".into())
}

fn load_source_list(base: &Path, catalog: &Catalog, name: &str, language: &str) -> Result<SourceList> {
    let manifest = format!("generated/buildgame/{}.mapresources", name.to_ascii_lowercase());
    let mut source = load_manifest(base, catalog, &manifest, language, false)?;
    if source.header_container != 0 {
        source.reuse_path = Some(format!("generated/buildgame/{}.maprscreusechunk0", name.to_ascii_lowercase()));
    }
    Ok(source)
}

fn load_reuse(base: &Path, catalog: &Catalog, source: &SourceList) -> Result<Option<ReuseSelection>> {
    if let Some(reuse_path) = source.reuse_path.as_deref().filter(|path| catalog.resource_container(path).is_some()) {
        let reuse = load_manifest(base, catalog, reuse_path, &source.language, true)?;
        return combine_reuse(source, reuse).map(Some);
    }
    Ok(None)
}

fn combine_reuse(source: &SourceList, reuse: SourceList) -> Result<ReuseSelection> {
        let primary_paths: HashMap<(&str, &str), u32> = source.resources.iter().enumerate()
            .map(|(index, resource)| ((resource.kind.as_str(), resource.path.as_str()), index as u32)).collect();
        let mut additional = Vec::new();
        let mut additional_paths: HashMap<(&str, &str), u32> = HashMap::new();
        let mut headers = Vec::with_capacity(reuse.headers.len());
        for mut header in reuse.headers {
            let resource = reuse.resources.get(header.resource as usize).ok_or("reuse header identity is absent")?;
            let (is_primary, index) = if let Some(index) = primary_paths.get(&(resource.kind.as_str(), resource.path.as_str())) {
                (true, *index)
            } else if let Some(index) = additional_paths.get(&(resource.kind.as_str(), resource.path.as_str())) {
                (false, *index)
            } else {
                let index = u32::try_from(additional.len()).map_err(|_| "reuse resource list is too large")?;
                additional_paths.insert((resource.kind.as_str(), resource.path.as_str()), index);
                additional.push(resource.clone());
                (false, index)
            };
            header.resource = index;
            headers.push(ReuseHeader { primary: is_primary, selection: header });
        }
        drop(primary_paths);
    Ok(ReuseSelection { additional, headers })
}

fn load_manifest(base: &Path, catalog: &Catalog, manifest: &str, language: &str, headers_only: bool) -> Result<SourceList> {
    // The manifest's record in the patched index names the archive holding
    // its bytes; the ids are that archive's index numbering (an unchanged
    // manifest keeps the base numbering, which the patch index renumbers).
    let (group, slot) = catalog.resource_container(&manifest)
        .ok_or_else(|| format!("{manifest} is not indexed"))?;
    let bytes = catalog.read(&manifest).map_err(|e| e.to_string())?.ok_or_else(|| format!("{manifest} is missing"))?;
    let requested = ids(&bytes)?;
    let wanted: HashSet<_> = requested.iter().copied().collect();
    let mut records: HashMap<u32, Vec<(u32, IndexEntry)>> = HashMap::new();
    let path = index::slot_index_path(base, group, slot).map_err(|e| e.to_string())?;
    let mut ordinal = 0u32;
    index::visit_file_ids(&path, group, &wanted, |r| {
        if !headers_only || preload_declaration_kind(&r.kind) {
            records.entry(r.id).or_default().push((ordinal, r));
        }
        ordinal += 1;
    }).map_err(|e| e.to_string())?;
    materialize_manifest(catalog, manifest, language, headers_only, &bytes, &requested, &records)
}

/// Read each actual container index once for a batch of manifest selections.
/// The temporary table is shared only while the immutable section owners are
/// constructed; it is not a session-level resource or path cache.
fn load_manifest_batch(base: &Path, catalog: &Catalog, language: &str,
    manifests: &[(String, bool)]) -> Result<Vec<SourceList>> {
    struct Input { manifest: String, headers_only: bool, bytes: Vec<u8>, ids: Vec<u32>, key: (std::path::PathBuf, u8) }
    #[derive(Default)]
    struct Selection { ids: HashSet<u32>, full_ids: HashSet<u32>, records: HashMap<u32, Vec<(u32, IndexEntry)>> }
    let mut inputs = Vec::with_capacity(manifests.len());
    let mut selections: HashMap<(std::path::PathBuf, u8), Selection> = HashMap::new();
    for (manifest, headers_only) in manifests {
        let (group, slot) = catalog.resource_container(manifest).ok_or_else(|| format!("{manifest} is not indexed"))?;
        let bytes = catalog.read(manifest).map_err(|error| error.to_string())?
            .ok_or_else(|| format!("{manifest} is missing"))?;
        let ids = ids(&bytes)?;
        let key = (index::slot_index_path(base, group, slot).map_err(|error| error.to_string())?, group);
        let selection = selections.entry(key.clone()).or_default();
        selection.ids.extend(ids.iter().copied());
        if !*headers_only { selection.full_ids.extend(ids.iter().copied()); }
        inputs.push(Input { manifest: manifest.clone(), headers_only: *headers_only, bytes, ids, key });
    }
    for ((path, group), selection) in &mut selections {
        let mut ordinal = 0u32;
        let full_ids = &selection.full_ids;
        let records = &mut selection.records;
        index::visit_file_ids(path, *group, &selection.ids, |record| {
            if full_ids.contains(&record.id) || preload_declaration_kind(&record.kind) {
                records.entry(record.id).or_default().push((ordinal, record));
            }
            ordinal += 1;
        }).map_err(|error| error.to_string())?;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY grouped manifest index: path={} requested_ids={} full_ids={} selected_records={} retained_records={}",
                path.display(), selection.ids.len(), selection.full_ids.len(), ordinal,
                selection.records.values().map(Vec::len).sum::<usize>());
        }
    }
    inputs.into_iter().map(|input| {
        let selection = selections.get(&input.key).ok_or("manifest index selection is absent")?;
        materialize_manifest(catalog, &input.manifest, language, input.headers_only,
            &input.bytes, &input.ids, &selection.records)
    }).collect()
}

/// Materialize against a caller-owned index selection. Several manifests in
/// the same container can share this borrowed record table and one scan.
fn materialize_manifest(catalog: &Catalog, manifest: &str, language: &str, headers_only: bool,
    bytes: &[u8], requested: &[u32], records: &HashMap<u32, Vec<(u32, IndexEntry)>>) -> Result<SourceList> {
    let mut masks: HashMap<u32, [u8; 24]> = HashMap::new();
    for (index, id) in requested.iter().copied().enumerate() {
        if !records.get(&id).is_some_and(|entries| entries.iter().any(|(_, entry)|
            preload_declaration_kind(&entry.kind))) { continue; }
        let mask = &bytes[HEADER + index * ENTRY + 4..HEADER + (index + 1) * ENTRY];
        let combined = masks.entry(id).or_insert([0; 24]);
        for (destination, source) in combined.iter_mut().zip(mask) { *destination |= *source; }
    }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY manifest metadata: manifest={manifest} discovery_scans=0 requested_entries={} index_selection_ids={} index_selection_records={}",
            requested.len(), records.len(), records.values().map(Vec::len).sum::<usize>());
    }
    let localized = format!("/localized/{language}/");
    let mut list = Vec::new();
    let mut headers = Vec::new();
    let mut missing = Vec::new();
    for &id in requested {
        // The patched `common` list names one id (2789) its index lacks.
        let Some(entries) = records.get(&id) else {
            if !headers_only { missing.push(id); }
            continue;
        };
        let eligible: Vec<&(u32, IndexEntry)> = entries.iter().filter(|(_, entry)| !headers_only
            || preload_declaration_kind(&entry.kind)).collect();
        let chosen: Vec<&(u32, IndexEntry)> = if eligible.iter().any(|(_, e)| e.path.contains("/localized/")) {
            eligible.into_iter().filter(|(_, e)| e.path.contains(&localized)).collect()
        } else {
            eligible
        };
        for (ordinal, e) in chosen {
            let mut path = e.path.clone();
            path.make_ascii_lowercase();
            if path.contains('\\') { path = path.replace('\\', "/"); }
            if preload_declaration_kind(&e.kind) {
                let resource = u32::try_from(list.len()).map_err(|_| "manifest resource list is too large")?;
                headers.push((*ordinal, HeaderSelection { resource, mask: masks[&id], flags: e.flags }));
            }
            list.push(Resource { kind: e.kind.clone(), path });
        }
    }
    if !missing.is_empty() {
        eprintln!("{manifest}: ids without an index record: {missing:?}");
    }
    headers.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    headers.dedup_by_key(|(ordinal, _)| *ordinal);
    let headers: Vec<HeaderSelection> = headers.into_iter().map(|(_, header)| header).collect();
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY manifest header selection: manifest={manifest} headers={} capacity={} retained_bytes={}",
            headers.len(), headers.capacity(), headers.capacity() * std::mem::size_of::<HeaderSelection>());
    }
    let header_container = catalog.resource_header_container(manifest)?
        .ok_or_else(|| format!("manifest has no primary header container: {manifest}"))?;
    Ok(SourceList { catalog_identity: catalog.identity(), header_container, resources: std::sync::Arc::new(list),
        headers, reuse: std::sync::OnceLock::new(),
        reuse_path: None, language: language.into() })
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
    Ok(acquire_source_list(base, catalog, name, language)?.resources.clone())
}

fn acquire_source_list(base: &Path, catalog: &Catalog, name: &str, language: &str) -> Result<std::sync::Arc<SourceList>> {
    let normalized = name.to_ascii_lowercase();
    // A loose manifest can change without changing the catalog snapshot.
    let manifest = format!("generated/buildgame/{normalized}.mapresources");
    let reuse_manifest = format!("generated/buildgame/{normalized}.maprscreusechunk0");
    if catalog.has_overlay_file(&manifest) || catalog.has_overlay_file(&reuse_manifest) {
        return Ok(std::sync::Arc::new(load_source_list(base, catalog, name, language)?));
    }
    let key = (catalog.identity(), base.to_owned(), normalized, language.to_ascii_lowercase());
    let mut lists = SOURCE_LISTS.lock().unwrap();
    let lists = lists.get_or_insert_with(Default::default);
    // Existing callers keep their Arc; the owner no longer pins another catalog.
    lists.retain(|(identity, _, _, _), _| *identity == catalog.identity());
    if let Some(list) = lists.get(&key) { return Ok(list.clone()); }
    let list = std::sync::Arc::new(load_source_list(base, catalog, name, language)?);
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
    sections: [std::sync::Arc<SourceList>; 3],
}
impl Default for LevelList {
    fn default() -> Self {
        let empty = std::sync::Arc::new(SourceList { catalog_identity: 0, header_container: 0, resources: std::sync::Arc::new(Vec::new()), headers: Vec::new(),
            reuse: std::sync::OnceLock::new(), reuse_path: None, language: "".into() });
        Self { sections: [empty.clone(), empty.clone(), empty] }
    }
}
impl LevelList {
    /// 0661e0 registers init/common before the map; 040490 then promotes
    /// existing bootstrap identities through 060b60. Host: payloads retain
    /// demand loading instead of eagerly decoding unused animation resources.
    /// Installed lists have no named/default layers and only the global bit.
    pub(crate) fn register_preload_headers(&self, catalog: &Catalog, map: &str,
        persistent_heap: bool) -> Result<usize> {
        let context = crate::edge_animation::CreationContext::Level;
        let bootstrap = catalog.preload_bootstrap.get_or_init(|| {
            let mut count = 0;
            for section in 0..2 {
                count += self.register_declaration_headers(catalog, section, &[0; 24], context, false)?;
            }
            crate::edge_animation::residency::promote_bootstrap_records(catalog)?;
            crate::assets::animation_rig::animset::promote_bootstrap_records(catalog)?;
            crate::assets::animation_rig::declaration::promote_bootstrap_records(catalog)?;
            crate::assets::parm_block::promote_bootstrap(catalog)?;
            crate::assets::fx::particle_resources::promote_bootstrap(catalog)?;
            crate::assets::inventory_item::promote_bootstrap(catalog)?;
            crate::assets::inventory_item::configuration::promote_bootstrap(catalog)?;
            Ok(count)
        }).as_ref().map_err(Clone::clone)?;
        let mut previous = catalog.preload_map.lock()
            .map_err(|_| "animation preload map owner is poisoned")?;
        // 049f30 records exact map-name equality; 04a6c0 additionally checks
        // com_usePersistentHeap. 062bd0 then gives its inverse to 0410f0,
        // which purges marker1 for a different map or disabled persistence.
        if !persistent_heap || previous.as_deref() != Some(map) {
            crate::assets::parm_block::release_marker(catalog, 1);
            crate::assets::fx::particle_resources::release_marker(catalog, 1);
            crate::assets::inventory_item::configuration::release_persistent(catalog);
            crate::edge_animation::release_marker(catalog, crate::edge_animation::PurgeMarker::PersistentHeap);
            crate::assets::animation_rig::animset::release_scoped(catalog, crate::edge_animation::PurgeMarker::PersistentHeap);
            crate::assets::animation_rig::declaration::release_marker(catalog,
                crate::assets::animation_rig::declaration::PurgeMarker::PersistentHeap);
        }
        let map_count = self.register_declaration_headers(catalog, 2, &[0; 24], context, persistent_heap)?;
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
            eprintln!("MEMORY animation header registration: bootstrap={} map={map_count} persistent_heap={persistent_heap}", bootstrap);
        }
        // Store only a successfully registered map. No per-frame comparison.
        *previous = Some(map.into());
        Ok(map_count)
    }

    /// Native declaration registration and parse run separately for each
    /// mounted manifest, rather than on a flattened multi-manifest view.
    pub(crate) fn sections(&self) -> impl Iterator<Item = &[Resource]> {
        self.sections.iter().map(|section| section.resources.as_slice())
    }
    /// Each section is still processed at its own source load phase. Header
    /// selections refer to that section's retained resource identities.
    pub(crate) fn selected_headers(&self, section: usize) -> Option<(&[Resource], &[HeaderSelection])> {
        self.sections.get(section).map(|source| (source.resources.as_slice(), source.headers.as_slice()))
    }
    /// The caller owns the actual 0661e0 section phase and layer selection.
    /// Register only selected, non-streamed headers, in container order;
    /// creation never reads the referenced animation or declaration payload.
    pub(crate) fn register_declaration_headers(&self, catalog: &Catalog, section: usize,
        layers: &[u8; 24], context: crate::edge_animation::CreationContext,
        promote_preload_kinds: bool) -> Result<usize> {
        let source = self.sections.get(section).ok_or("manifest section is absent")?;
        if source.catalog_identity != catalog.identity() { return Err("manifest catalog identity differs".into()); }
        let reuse = if source.header_container != 0 {
            source.reuse.get_or_init(|| load_reuse(catalog.base(), catalog, source))
                .as_ref().map_err(Clone::clone)?.as_ref()
        } else { None };
        let mut layers = *layers;
        // 064920 always includes the global layer after applying the caller's
        // selected layers, including when the supplied filter is empty.
        layers[23] |= 0x80;
        let mut registered = 0;
        let mut register = |resource: &Resource, header: &HeaderSelection| -> Result<()> {
            if !header.mask.iter().zip(&layers).any(|(mask, selected)| mask & selected != 0)
                || header.flags & 0x20 != 0 { return Ok(()); }
            if [".sbcm", ".tbcm", ".bwm"].iter().any(|suffix| resource.path.contains(suffix)) { return Ok(()); }
            if resource.kind.eq_ignore_ascii_case("inventoryLoadout")
                || resource.kind.eq_ignore_ascii_case("inventoryItemCollection") {
                // Gate by the existing kind before normalizing a path: other
                // resource families make no inventory allocation or lookup.
                if !crate::assets::inventory_item::configuration::register_header(catalog, &resource.path)? {
                    return Err(format!("invalid inventory declaration resource {}", resource.path));
                }
                // Neither class belongs to 0661e0's heap-1 promotion list.
            } else if resource.kind.eq_ignore_ascii_case("anim") {
                crate::edge_animation::register_lazy_preload(catalog, &resource.path, context, promote_preload_kinds)?;
            } else if resource.kind.eq_ignore_ascii_case("animset") {
                crate::assets::animation_rig::animset::register_lazy_preload(catalog, &resource.path,
                    context, promote_preload_kinds)?;
            } else if resource.kind.eq_ignore_ascii_case("particledt")
                || resource.kind.eq_ignore_ascii_case("staticParticleModel") {
                crate::assets::fx::particle_resources::register_preload(catalog, &resource.kind,
                    &resource.path, context as u8, promote_preload_kinds)?;
            } else if resource.kind.eq_ignore_ascii_case("pblock") {
                crate::assets::parm_block::register_preload(catalog, &resource.path, context, promote_preload_kinds)?;
            } else if resource.kind.eq_ignore_ascii_case("inventoryitem") {
                crate::assets::inventory_item::register_preload(catalog, &resource.path, context)?;
            } else {
                // MD6 is not in 0661e0's list of heap-1 promoted kinds.
                let context = match context {
                    crate::edge_animation::CreationContext::Level => crate::assets::animation_rig::declaration::LoadContext::Level,
                    crate::edge_animation::CreationContext::PersistentHeap => crate::assets::animation_rig::declaration::LoadContext::PersistentHeap,
                };
                crate::assets::animation_rig::declaration::register(catalog, &resource.path, context)?;
            }
            registered += 1;
            Ok(())
        };
        // The source checks container zero's reuse chunk before the primary
        // container and performs the eager phase after both header passes.
        if let Some(reuse) = reuse {
            for header in &reuse.headers {
                let resources = if header.primary { source.resources.as_slice() } else { reuse.additional.as_slice() };
                let resource = resources.get(header.selection.resource as usize).ok_or("reuse header identity is absent")?;
                register(resource, &header.selection)?;
            }
        }
        for header in &source.headers {
            let resource = source.resources.get(header.resource as usize).ok_or("manifest header identity is absent")?;
            register(resource, header)?;
        }
        Ok(registered)
    }
    fn acquire(base: &Path, catalog: &Catalog, map: &str, language: &str) -> Result<Self> {
        enum BatchItem { Primary(usize), Reuse(usize) }
        if let Some(lists) = SOURCE_LISTS.lock().unwrap().as_mut() {
            lists.retain(|(identity, _, _, _), _| *identity == catalog.identity());
        }
        let names = ["init", "common", map];
        let mut sections: Vec<Option<std::sync::Arc<SourceList>>> = vec![None; names.len()];
        let mut manifests = Vec::new();
        let mut items = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let normalized = name.to_ascii_lowercase();
            let primary = format!("generated/buildgame/{normalized}.mapresources");
            let reuse = format!("generated/buildgame/{normalized}.maprscreusechunk0");
            let header_container = catalog.resource_header_container(&primary)?
                .ok_or_else(|| format!("manifest has no primary header container: {primary}"))?;
            let overlay = catalog.has_overlay_file(&primary) || catalog.has_overlay_file(&reuse);
            let key = (catalog.identity(), base.to_owned(), normalized, language.to_ascii_lowercase());
            if !overlay {
                sections[index] = SOURCE_LISTS.lock().unwrap().as_ref().and_then(|lists| lists.get(&key)).cloned();
            }
            if sections[index].is_none() {
                manifests.push((primary, false));
                items.push(BatchItem::Primary(index));
            }
            let already_prepared = sections[index].as_ref().is_some_and(|source| source.reuse.get().is_some());
            if header_container != 0 && !already_prepared && catalog.resource_container(&reuse).is_some() {
                manifests.push((reuse, true));
                items.push(BatchItem::Reuse(index));
            }
        }
        let loaded = load_manifest_batch(base, catalog, language, &manifests)?;
        for (item, mut source) in items.into_iter().zip(loaded) {
            match item {
                BatchItem::Primary(index) => {
                    if source.header_container != 0 {
                        source.reuse_path = Some(format!("generated/buildgame/{}.maprscreusechunk0", names[index].to_ascii_lowercase()));
                    }
                    sections[index] = Some(std::sync::Arc::new(source));
                }
                BatchItem::Reuse(index) => {
                    let primary = sections[index].as_ref().ok_or("reuse section precedes its primary")?;
                    let reuse = combine_reuse(primary, source)?;
                    // Another reader can have prepared the same retained section.
                    // Its owner remains authoritative if it wins this race.
                    let _ = primary.reuse.set(Ok(Some(reuse)));
                }
            }
        }
        let mut complete = Vec::with_capacity(names.len());
        for (index, source) in sections.into_iter().enumerate() {
            let source = source.ok_or("manifest section was not acquired")?;
            let _ = source.reuse.set(Ok(None));
            if matches!(names[index], "init" | "common") {
                let normalized = names[index].to_ascii_lowercase();
                let primary = format!("generated/buildgame/{normalized}.mapresources");
                let reuse = format!("generated/buildgame/{normalized}.maprscreusechunk0");
                if !catalog.has_overlay_file(&primary) && !catalog.has_overlay_file(&reuse) {
                    let key = (catalog.identity(), base.to_owned(), normalized, language.to_ascii_lowercase());
                    SOURCE_LISTS.lock().unwrap().get_or_insert_with(Default::default).insert(key, source.clone());
                }
            }
            complete.push(source);
        }
        let sections = complete.try_into().map_err(|_| "manifest section count differs")?;
        Ok(Self { sections })
    }
    pub fn iter(&self) -> impl Iterator<Item = &Resource> {
        self.sections[0].resources.iter().chain(self.sections[1].resources.iter()).chain(self.sections[2].resources.iter())
    }
    pub fn len(&self) -> usize { self.sections.iter().map(|section| section.resources.len()).sum() }
    pub fn is_empty(&self) -> bool { self.sections.iter().all(|section| section.resources.is_empty()) }
    /// Counts section ownership; callers must account for shared Arc leases.
    pub(crate) fn logical_descriptor_bytes(&self) -> usize {
        self.sections.iter().map(|section| descriptor_bytes(&section.resources)
            + section.headers.capacity() * std::mem::size_of::<HeaderSelection>()
            + section.reuse_path.as_ref().map_or(0, String::capacity) + section.language.len()
            + section.reuse.get().and_then(|result| result.as_ref().ok()).and_then(Option::as_ref)
                .map_or(0, |reuse| descriptor_bytes(&reuse.additional)
                    + reuse.headers.capacity() * std::mem::size_of::<ReuseHeader>())).sum()
    }
}
impl std::ops::Index<usize> for LevelList {
    type Output = Resource;
    fn index(&self, index: usize) -> &Self::Output {
        let init = self.sections[0].resources.len();
        if index < init { return &self.sections[0].resources[index]; }
        let index = index - init;
        let common = self.sections[1].resources.len();
        if index < common { &self.sections[1].resources[index] }
        else { &self.sections[2].resources[index - common] }
    }
}

/// The resource manager's already prepared map request list (1400661e0).
/// Share the immutable request descriptors between consumers; the registered
/// resource owners decide whether each payload is already resident.
pub fn acquire_level_list(base: &Path, catalog: &Catalog, map: &str, language: &str) -> Result<std::sync::Arc<LevelList>> {
    // A live loose manifest may change while another consumer owns an older
    // combined list. Preserve that old lease but do not reuse it for new reads.
    if catalog.has_overlays() && ["init", "common", map].iter().any(|name|
        catalog.has_overlay_file(&format!("generated/buildgame/{}.mapresources", name.to_ascii_lowercase()))
            || catalog.has_overlay_file(&format!("generated/buildgame/{}.maprscreusechunk0", name.to_ascii_lowercase()))) {
        return Ok(std::sync::Arc::new(LevelList::acquire(base, catalog, map, language)?));
    }
    // 1400661e0 releases its assembled request table after acquisition. Source
    // manifests above remain reusable metadata; a combined view belongs only
    // to its active consumers, not every map visited during this session.
    let key = (catalog.identity(), base.to_owned(), map.to_ascii_lowercase(), language.to_ascii_lowercase());
    let mut lists = LEVEL_LISTS.lock().unwrap();
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
