//! 0607f0 item-declaration leases shared by world reading and Journal opens.
use super::*;
use std::{collections::{HashMap, HashSet}, ops::Deref, sync::{Arc, Mutex, Weak}};
#[derive(Debug)]
pub struct PickableItem {
    pub path: Arc<str>,
    pub readable: Option<PickableReadable>,
    pub modifiers: ItemModifiers,
    item_type: Option<String>,
    audio: Option<(String, String)>,
    pickup_image: Option<String>,
    // The same read-generation lease covers every inherited inventory path.
    _source: Arc<()>,
}
#[derive(Debug)]
pub enum PickableReadable {
    Note(Readable),
    Other(Readable, PickableOtherKind),
    Map(crate::assets::interactive_map::Map),
}
#[derive(Clone, Copy, Debug)]
pub enum PickableOtherKind { Map, Audiograph, Item }
impl PickableItem {
    pub fn item_type(&self) -> &str { self.item_type.as_deref().unwrap_or("0") }
    pub fn map(&self) -> Result<Option<&crate::assets::interactive_map::Map>, &str> {
        Ok(match &self.readable { Some(PickableReadable::Map(map)) => Some(map), _ => None })
    }
    pub fn audiograph_text(&self) -> Option<(&str, &str, Option<&str>)> {
        let (name, text) = self.audio.as_ref()?;
        Some((name, text, self.pickup_image.as_deref()))
    }
}
/// An ephemeral generic projection keeps its source reads alive without
/// retaining generic declaration trees in every typed item.
pub struct ItemFields {
    fields: Vec<(String, crate::entities::Value)>,
    source: Arc<()>,
}
impl Deref for ItemFields {
    type Target = [(String, crate::entities::Value)];
    fn deref(&self) -> &Self::Target { &self.fields }
}
struct ReadGeneration {
    owner: Weak<()>,
    paths: Vec<Arc<str>>,
}

struct ItemRecord {
    current: Weak<PickableItem>,
    previous: Vec<Weak<PickableItem>>,
    resident: Option<Arc<PickableItem>>,
    /// Immutable source/parse failure retained by the existing header.
    failure: Option<Arc<str>>,
    // None retains the existing host policy where no allocator heap was supplied.
    marker: Option<u8>,
    retired: bool,
}
impl Default for ItemRecord {
    fn default() -> Self {
        Self { current: Weak::new(), previous: Vec::new(), resident: None, failure: None, marker: None, retired: false }
    }
}

#[derive(Default)]
pub struct PickableSources {
    catalog_identity: Option<u64>,
    items: HashMap<Arc<str>, ItemRecord>,
    reads: Vec<ReadGeneration>,
    pub(super) configurations: super::configuration::Registry,
}
#[derive(Clone)]
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub struct PickableSourceLease(pub Arc<Mutex<PickableSources>>);
impl PickableSourceLease {
    pub(crate) fn for_catalog(catalog: &Catalog) -> Result<Self, String> {
        let owner = catalog.pickable_sources.get_or_init(|| Arc::new(Mutex::new(
            PickableSources { catalog_identity: Some(catalog.identity()), ..Default::default() }
        ))).clone();
        if tracing() { eprintln!("PICKABLE_OWNER catalog={} identity={:p}", catalog.identity(), Arc::as_ptr(&owner)); }
        Ok(Self(owner))
    }
}

/// 0661e0 registers this kind on its real creation heap. Inventory items are
/// absent from the 36-kind persistent-promotion table.
pub(crate) fn register_preload(catalog: &Catalog, path: &str,
    context: crate::edge_animation::CreationContext) -> Result<(), String> {
    let name = path.strip_prefix("generated/decls/inventoryitem/")
        .and_then(|value| value.strip_suffix(".decl"))
        .ok_or("invalid inventory item manifest resource path")?;
    let owner = PickableSourceLease::for_catalog(catalog)?;
    let mut owner = owner.0.lock().map_err(|_| "inventory item owner is poisoned")?;
    let existed = owner.items.contains_key(name);
    let entry = owner.items.entry(Arc::from(name)).or_default();
    if !existed || entry.retired {
        entry.marker = Some(context as u8);
        entry.retired = false;
        entry.resident = entry.current.upgrade();
    }
    if tracing() { eprintln!("PICKABLE_HEADER catalog={} path={name} marker={:?} reused={existed}", catalog.identity(), entry.marker); }
    Ok(())
}

pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.pickable_sources.get() {
        let mut owner = owner.lock().map_err(|_| "inventory item owner is poisoned")?;
        for entry in owner.items.values_mut() {
            entry.marker = Some(2);
            entry.retired = false;
            if entry.resident.is_none() { entry.resident = entry.current.upgrade(); }
        }
        if tracing() { eprintln!("PICKABLE_BOOTSTRAP catalog={} headers={}", catalog.identity(), owner.items.len()); }
    }
    Ok(())
}
pub(super) fn tracing() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var_os("DH2_DEBUG_PICKABLE_RESOURCES").is_some())
}
impl PickableSources {
    pub fn acquire(&mut self, catalog: &Catalog, path: &str) -> Result<Arc<PickableItem>, String> {
        if self.catalog_identity.is_some_and(|id| id != catalog.identity()) { return Err("item source belongs to another catalog".into()); }
        self.catalog_identity = Some(catalog.identity());
        // Retire generations at the existing level/session purge boundary,
        // not by scanning all live declarations on every acquisition.
        let key: Arc<str> = Arc::from(path.replace('\\', "/").to_ascii_lowercase());
        {
            let entry = self.items.entry(key.clone()).or_default();
            if entry.retired { entry.retired = false; entry.marker = None; entry.resident = entry.current.upgrade(); }
        }
        if !catalog.has_overlays() {
            if let Some(item) = self.items.get(&key).and_then(|entry| entry.current.upgrade()) {
                self.items.get_mut(&key).ok_or("inventory item record disappeared")?.resident = Some(item.clone());
                if tracing() { eprintln!("PICKABLE_ITEM path={key} identity={:p} reused=true", Arc::as_ptr(&item)); }
                return Ok(item);
            }
        }
        let fields = self.read_fields(catalog, &key)?;
        let readable = if let Some(note) = readable_fields(&fields)? { Some(PickableReadable::Note(note)) }
        else if let Some(map) = crate::assets::interactive_map::resolve_fields(catalog, &key, &fields)? { Some(PickableReadable::Map(map)) }
        else { other_readable_fields(&fields)?.map(|(readable, kind)| PickableReadable::Other(readable, match kind {
            "TYPE_INTERACTIVE_MAP" | "24" => PickableOtherKind::Map,
            "TYPE_AUDIOGRAPH" => PickableOtherKind::Audiograph,
            _ => PickableOtherKind::Item,
        })) };
        let audio = (atom(&fields, "m_type") == Some("TYPE_AUDIOGRAPH")).then(||
            (atom(&fields, "m_name").unwrap_or("").to_owned(), atom(&fields, "m_description").unwrap_or("").to_owned()));
        let item = Arc::new(PickableItem { path: key.clone(), readable, modifiers: modifiers_fields(&key, &fields)?,
            item_type: atom(&fields, "m_type").map(str::to_owned), audio,
            pickup_image: atom(&fields, "m_pickupLogImage").filter(|name| !name.is_empty() && *name != "NULL").map(str::to_owned), _source: fields.source.clone() });
        // Overlay requests bypass reuse, but their read generations remain
        // accounted until every old/new consumer releases its actual lease.
        let entry = self.items.get_mut(&key).ok_or("inventory item record disappeared")?;
        let previous = std::mem::replace(&mut entry.current, Arc::downgrade(&item));
        entry.resident = Some(item.clone());
        entry.previous.retain(|generation| generation.strong_count() != 0);
        if previous.strong_count() != 0 { entry.previous.push(previous); }
        if tracing() { eprintln!("PICKABLE_ITEM path={} identity={:p} reused=false", item.path, Arc::as_ptr(&item)); }
        Ok(item)
    }
    pub(super) fn read_fields(&mut self, catalog: &Catalog, item: &str) -> Result<ItemFields, String> {
        if self.catalog_identity.is_some_and(|id| id != catalog.identity()) { return Err("item source belongs to another catalog".into()); }
        self.catalog_identity = Some(catalog.identity());
        let key: Arc<str> = Arc::from(item.replace('\\', "/").to_ascii_lowercase());
        {
            let entry = self.items.entry(key.clone()).or_default();
            if entry.retired { entry.retired = false; entry.marker = None; entry.resident = entry.current.upgrade(); }
            if !catalog.has_overlays() {
                if let Some(error) = &entry.failure {
                    if tracing() { eprintln!("PICKABLE_FAILURE path={key} reused=true"); }
                    return Err(error.to_string());
                }
            }
        }
        let source = Arc::new(());
        let mut paths = Vec::new();
        let fields = entities::player::resolved_decl("inventoryitem", item, |path| {
            let normalized = path.replace('\\', "/").to_ascii_lowercase();
            if let Some(name) = normalized.strip_prefix("generated/decls/inventoryitem/")
                .and_then(|value| value.strip_suffix(".decl")) {
                let entry = self.items.entry(Arc::from(name)).or_default();
                if entry.retired { entry.retired = false; entry.marker = None; entry.resident = entry.current.upgrade(); }
            }
            let bytes = catalog.read(path).map_err(|error| error.to_string())?;
            if bytes.is_some() { paths.push(Arc::from(normalized)); }
            Ok(bytes.map(|bytes| entities::declaration_text(&bytes).into_owned()))
        });
        self.reads.push(ReadGeneration { owner: Arc::downgrade(&source), paths });
        match fields {
            Ok(fields) => {
                self.items.get_mut(&key).ok_or("inventory item record disappeared")?.failure = None;
                Ok(ItemFields { fields, source })
            }
            Err(error) => {
                if !catalog.has_overlays() {
                    self.items.get_mut(&key).ok_or("inventory item record disappeared")?.failure = Some(Arc::from(error.as_str()));
                }
                if tracing() { eprintln!("PICKABLE_FAILURE path={key} reused=false"); }
                Err(error)
            }
        }
    }
    pub fn retire_unused(&mut self) {
        let mut live = HashSet::new();
        let mut retired = HashSet::new();
        self.reads.retain(|generation| {
            let keep = generation.owner.strong_count() != 0;
            let paths = if keep { &mut live } else { &mut retired };
            paths.extend(generation.paths.iter().cloned());
            keep
        });
        let live_items: HashSet<&str> = live.iter().filter_map(|path: &Arc<str>|
            path.strip_prefix("generated/decls/inventoryitem/").and_then(|value| value.strip_suffix(".decl"))).collect();
        self.items.retain(|name, entry| {
            entry.previous.retain(|generation| generation.strong_count() != 0);
            !entry.retired || entry.current.strong_count() != 0 || !entry.previous.is_empty()
                || live_items.contains(name.as_ref())
        });
        if let Some(catalog) = self.catalog_identity {
            for path in retired.difference(&live) { crate::loading::forget(catalog, "inventoryitem", path); }
        }
        if tracing() { eprintln!("PICKABLE_RETIRE live_generations={} live_paths={} retired_paths={}", self.reads.len(), live.len(), retired.difference(&live).count()); }
    }
    pub(crate) fn release_session(&mut self, catalog: &Catalog) {
        let before = self.items.len();
        for entry in self.items.values_mut() { entry.retired = true; entry.resident = None; entry.failure = None; }
        self.configurations.release_session(catalog);
        self.retire_unused();
        if tracing() { eprintln!("PICKABLE_SESSION_RELEASE catalog={} before={before} removed={} retained_live={}", catalog.identity(), before - self.items.len(), self.items.len()); }
    }
    pub(crate) fn release_level(&mut self, catalog: &Catalog) {
        let before = self.items.len();
        let mut matched = 0usize;
        let mut resident = 0usize;
        for entry in self.items.values_mut() {
            if !entry.retired && (entry.marker.is_none() || entry.marker == Some(0)) {
                matched += 1;
                resident += usize::from(entry.resident.is_some());
                entry.retired = true;
                entry.resident = None;
                entry.failure = None;
            }
        }
        self.retire_unused();
        self.configurations.release(catalog, super::configuration::Scope::Level);
        if tracing() {
            let permanent = self.items.values().filter(|entry| entry.marker == Some(2) && !entry.retired).count();
            let retired_live = self.items.values().filter(|entry| entry.retired).count();
            eprintln!("PICKABLE_PURGE catalog={} marker=level before={before} matched={matched} resident_released={resident} removed={} retained={} permanent={permanent} retired_live={retired_live}", catalog.identity(), before - self.items.len(), self.items.len());
        }
    }
}
