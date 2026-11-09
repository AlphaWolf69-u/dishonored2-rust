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

#[derive(Default)]
pub struct PickableSources {
    catalog_identity: Option<u64>,
    items: HashMap<Arc<str>, Weak<PickableItem>>,
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
        if !catalog.has_overlays() {
            if let Some(item) = self.items.get(&key).and_then(Weak::upgrade) {
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
        self.items.insert(key, Arc::downgrade(&item));
        if tracing() { eprintln!("PICKABLE_ITEM path={} identity={:p} reused=false", item.path, Arc::as_ptr(&item)); }
        Ok(item)
    }
    pub(super) fn read_fields(&mut self, catalog: &Catalog, item: &str) -> Result<ItemFields, String> {
        if self.catalog_identity.is_some_and(|id| id != catalog.identity()) { return Err("item source belongs to another catalog".into()); }
        self.catalog_identity = Some(catalog.identity());
        let source = Arc::new(());
        let mut paths = Vec::new();
        let fields = entities::player::resolved_decl("inventoryitem", item, |path| {
            let bytes = catalog.read(path).map_err(|error| error.to_string())?;
            if bytes.is_some() { paths.push(Arc::from(path.replace('\\', "/").to_ascii_lowercase())); }
            Ok(bytes.map(|bytes| entities::declaration_text(&bytes).into_owned()))
        });
        self.reads.push(ReadGeneration { owner: Arc::downgrade(&source), paths });
        fields.map(|fields| ItemFields { fields, source })
    }
    pub fn retire_unused(&mut self) {
        self.items.retain(|_, item| item.strong_count() != 0);
        if self.reads.iter().all(|generation| generation.owner.strong_count() != 0) { return; }
        let mut live = HashSet::new();
        let mut retired = HashSet::new();
        self.reads.retain(|generation| {
            let keep = generation.owner.strong_count() != 0;
            let paths = if keep { &mut live } else { &mut retired };
            paths.extend(generation.paths.iter().cloned());
            keep
        });
        if let Some(catalog) = self.catalog_identity {
            for path in retired.difference(&live) { crate::loading::forget(catalog, "inventoryitem", path); }
        }
        if tracing() { eprintln!("PICKABLE_RETIRE live_generations={} live_paths={} retired_paths={}", self.reads.len(), live.len(), retired.difference(&live).count()); }
    }
    pub(crate) fn release_session(&mut self, catalog: &Catalog) {
        self.configurations.release_session(catalog);
        self.retire_unused();
    }
    pub(crate) fn release_level(&mut self, catalog: &Catalog) {
        self.retire_unused();
        self.configurations.release(catalog, super::configuration::Scope::Level);
    }
}
