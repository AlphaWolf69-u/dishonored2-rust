//! 0607f0 item-declaration leases shared by world reading and Journal opens.
use super::*;
use std::sync::{Arc, Mutex, Weak};
#[derive(Debug)]
pub struct PickableItem {
    pub path: Arc<str>,
    pub readable: Option<PickableReadable>,
    pub modifiers: ItemModifiers,
    item_type: Option<String>,
    audio: Option<(String, String)>,
    pickup_image: Option<String>,
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
#[derive(Default)]
pub struct PickableSources {
    catalog_identity: Option<u64>,
    items: HashMap<Arc<str>, Weak<PickableItem>>,
}
#[derive(Clone)]
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub struct PickableSourceLease(pub Arc<Mutex<PickableSources>>);
static SESSIONS: std::sync::LazyLock<Mutex<HashMap<u64, Weak<Mutex<PickableSources>>>>> = std::sync::LazyLock::new(Default::default);
impl PickableSourceLease {
    pub(crate) fn for_catalog(catalog: &Catalog) -> Result<Self, String> {
        let mut sessions = SESSIONS.lock().map_err(|_| "item source sessions are poisoned")?;
        let id = catalog.identity();
        if let Some(owner) = sessions.get(&id).and_then(Weak::upgrade) { return Ok(Self(owner)); }
        sessions.retain(|_, owner| owner.strong_count() != 0);
        let owner = Arc::new(Mutex::new(PickableSources { catalog_identity: Some(id), ..Default::default() }));
        sessions.insert(id, Arc::downgrade(&owner));
        Ok(Self(owner))
    }
}
impl PickableSources {
    pub fn acquire(&mut self, catalog: &Catalog, path: &str) -> Result<Arc<PickableItem>, String> {
        if self.catalog_identity.is_some_and(|id| id != catalog.identity()) { return Err("item source belongs to another catalog".into()); }
        self.catalog_identity = Some(catalog.identity());
        // Acquisition is off-thread, outside the frame loop. Retire dead
        // weak identities here; admitted inventory entries keep their leases.
        self.retire_unused();
        let key: Arc<str> = Arc::from(path.replace('\\', "/").to_ascii_lowercase());
        if !catalog.has_overlays() {
            if let Some(item) = self.items.get(&key).and_then(Weak::upgrade) { return Ok(item); }
        }
        let fields = item_fields(catalog, &key)?;
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
            pickup_image: atom(&fields, "m_pickupLogImage").filter(|name| !name.is_empty() && *name != "NULL").map(str::to_owned) });
        if !catalog.has_overlays() { self.items.insert(key, Arc::downgrade(&item)); }
        Ok(item)
    }
    pub fn retire_unused(&mut self) { self.items.retain(|_, item| item.strong_count() != 0); }
}
