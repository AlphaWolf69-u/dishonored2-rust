//! Inventory collection/loadout declaration identities and their item lists.
//! 613ae0/613b60 construct them, 011a20 loads their inherited edit data,
//! and the shared 613930 destructor releases the lists before 022c10.
use super::{Catalog, PickableSourceLease, atom};
use crate::entities::{self, Value};
use std::{collections::HashMap, sync::{Arc, Weak}};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Scope { Level = 0, PersistentHeap = 1, Permanent = 2 }

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { Collection, Loadout }
impl Kind {
    fn directory(self) -> &'static str {
        match self { Self::Collection => "inventoryitemcollection", Self::Loadout => "inventoryloadout" }
    }
}

#[derive(Clone)]
struct Item {
    slot: Arc<str>,
    declaration: Option<Arc<str>>,
    quantity: i32,
}

/// One immutable generation, including its actual inherited declaration.
pub(super) struct Definition {
    path: Arc<str>,
    all_registered: bool,
    pub(super) clear: bool,
    pub(super) allow_auto_equip: bool,
    items: Vec<Item>,
    first: HashMap<Arc<str>, usize>,
    _parent: Option<Arc<Definition>>,
}
impl Definition {
    pub(super) fn assignments(&self) -> impl Iterator<Item = (&str, i32)> {
        self.items.iter().filter_map(|item| item.declaration.as_deref().map(|name| (name, item.quantity)))
    }
    /// 6136e0: the first occurrence controls even a zero/negative exclusion.
    pub(super) fn maximum(&self, name: &str) -> u32 {
        let normalized;
        let name = if name.bytes().any(|b| b.is_ascii_uppercase() || b == b'\\') {
            normalized = name.replace('\\', "/").to_ascii_lowercase(); normalized.as_str()
        } else { name };
        self.first.get(name).map_or(u32::from(self.all_registered), |&index| self.items[index].quantity.max(0) as u32)
    }
}
impl Drop for Definition {
    fn drop(&mut self) {
        if super::sources::tracing() { eprintln!("INVENTORY_DECL_RELEASE path={} identity={:p}", self.path, self); }
    }
}

struct Record {
    scope: Scope,
    current: Weak<Definition>,
    previous: Vec<Weak<Definition>>,
    // Native declaration payload residency ends at the matching purge;
    // live backend consumers can retain their immutable generation longer.
    resident: Option<Arc<Definition>>,
}
impl Record {
    fn new(scope: Scope) -> Self { Self { scope, current: Weak::new(), previous: Vec::new(), resident: None } }
}
#[derive(Default)]
pub(super) struct Registry { records: HashMap<Arc<str>, Record> }

fn path(kind: Kind, name: &str) -> Result<Arc<str>, String> {
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    if normalized.is_empty() || normalized == "null" { return Err("unnamed inventory configuration".into()); }
    let prefix = format!("generated/decls/{}/", kind.directory());
    Ok(Arc::from(if normalized.starts_with(&prefix) { normalized }
        else { format!("{prefix}{normalized}.decl") }))
}
fn resource_kind(path: &str) -> Option<Kind> {
    if path.starts_with("generated/decls/inventoryitemcollection/") { Some(Kind::Collection) }
    else if path.starts_with("generated/decls/inventoryloadout/") { Some(Kind::Loadout) }
    else { None }
}
fn edit(fields: &[(String, Value)]) -> Result<&[(String, Value)], String> {
    match fields.iter().find(|(key, _)| key == "edit").map(|(_, value)| value) {
        None => Ok(fields), Some(Value::Scope(fields)) => Ok(fields),
        Some(_) => Err("inventory configuration edit requires a scope".into()),
    }
}
fn flag(fields: &[(String, Value)], name: &str, default: bool) -> Result<bool, String> {
    match atom(fields, name) {
        None => Ok(default), Some("true" | "1") => Ok(true), Some("false" | "0") => Ok(false),
        Some(value) => Err(format!("invalid {name}: {value}")),
    }
}

impl Registry {
    fn acquire(&mut self, catalog: &Catalog, kind: Kind, name: &str, context: Scope,
        active: &mut Vec<Arc<str>>) -> Result<Arc<Definition>, String> {
        let path = path(kind, name)?;
        if active.contains(&path) { return Err(format!("inventory configuration inheritance cycle at {path}")); }
        let _tag = cfg!(feature = "debug_allocations").then(|| crate::platform::allocation_tags::tag("inventory-configuration-owner"));
        let record = self.records.entry(path.clone()).or_insert_with(|| Record::new(context));
        let scope = record.scope;
        if !catalog.has_overlays() {
            if let Some(value) = record.current.upgrade() {
                record.resident = Some(value.clone());
                if super::sources::tracing() { eprintln!("INVENTORY_DECL_ACQUIRE path={path} identity={:p} reused=true source_bytes=0", Arc::as_ptr(&value)); }
                return Ok(value);
            }
        }
        active.push(path.clone());
        let result = (|| {
            let bytes = catalog.read(&path).map_err(|error| error.to_string())?.ok_or_else(|| format!("inventory configuration missing: {path}"))?;
            let source_bytes = bytes.len();
            let fields = entities::parse_declaration(&entities::declaration_text(&bytes)).map_err(|error| format!("{path}: {error}"))?;
            drop(bytes);
            let parent = atom(&fields, "inherit").filter(|name| !name.is_empty() && *name != "NULL")
                .map(|name| self.acquire(catalog, kind, name, scope, active)).transpose()?;
            let inherited = flag(&fields, "expandInheritance", true)?.then_some(parent.as_deref()).flatten();
            let mut items = inherited.map_or_else(Vec::new, |value| value.items.clone());
            let fields = edit(&fields)?;
            let all_registered = match atom(fields, "m_type") {
                None => inherited.is_some_and(|value| value.all_registered),
                Some("ICT_DEFAULT_NOITEM" | "0") => false,
                Some("ICT_DEFAULT_ONEITEM" | "1") => true,
                Some(value) if kind == Kind::Collection => return Err(format!("unknown collection type {value}")),
                Some(_) => false,
            };
            let clear = flag(fields, "m_clear", inherited.is_some_and(|value| value.clear))?;
            let allow_auto_equip = flag(fields, "m_allowAutoEquip", inherited.is_none_or(|value| value.allow_auto_equip))?;
            if let Some((_, value)) = fields.iter().find(|(key, _)| key == "m_items") {
                let Value::Scope(entries) = value else { return Err("inventory m_items requires a scope".into()) };
                if entries.is_empty() { items.clear(); }
                for (key, value) in entries {
                    if key == "item_begin" { flag(entries, key, true)?; continue; }
                    if let Some(slot) = key.strip_prefix("item_remove[").and_then(|key| key.strip_suffix(']')) {
                        items.retain(|entry| entry.slot.as_ref() != slot); continue;
                    }
                    let override_entry = key.starts_with("item_override[");
                    let slot = key.strip_prefix("item_add[").or_else(|| key.strip_prefix("item_override["))
                        .and_then(|key| key.strip_suffix(']')).ok_or_else(|| format!("unsupported inventory list entry {key}"))?;
                    let index = match items.iter().position(|entry| entry.slot.as_ref() == slot) {
                        Some(index) => index,
                        // 00c460 discards an unmatched ordinary override.
                        None if override_entry => continue,
                        None => { items.push(Item { slot: Arc::from(slot), declaration: None, quantity: 1 }); items.len() - 1 },
                    };
                    let Value::Scope(fields) = value else { return Err("inventory item requires a scope".into()) };
                    if fields.is_empty() { items[index].declaration = None; items[index].quantity = 1; }
                    let (reference, quantity) = if kind == Kind::Collection { ("m_item", "m_maxQuantity") } else { ("m_decl", "m_quantity") };
                    if let Some(name) = atom(fields, reference) {
                        items[index].declaration = (!name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
                            .then(|| Arc::from(name.replace('\\', "/").to_ascii_lowercase()));
                    }
                    if let Some(value) = atom(fields, quantity) { items[index].quantity = value.parse().map_err(|_| format!("invalid {quantity}: {value}"))?; }
                }
            }
            let mut first = HashMap::new();
            for (index, item) in items.iter().enumerate() { if let Some(name) = &item.declaration { first.entry(name.clone()).or_insert(index); } }
            let value = Arc::new(Definition { path: path.clone(), all_registered, clear, allow_auto_equip, items, first, _parent: parent });
            let record = self.records.get_mut(&path).expect("configuration header remains registered");
            if record.current.strong_count() != 0 { record.previous.push(record.current.clone()); }
            record.current = Arc::downgrade(&value);
            record.resident = Some(value.clone());
            record.previous.retain(|generation| generation.strong_count() != 0);
            if super::sources::tracing() { eprintln!("INVENTORY_DECL_ACQUIRE path={path} identity={:p} reused=false source_bytes={source_bytes}", Arc::as_ptr(&value)); }
            Ok(value)
        })();
        active.pop();
        result
    }
    pub(super) fn collection(&mut self, catalog: &Catalog, name: &str) -> Result<Arc<Definition>, String> {
        self.acquire(catalog, Kind::Collection, name, Scope::Level, &mut Vec::new())
    }
    pub(super) fn loadout(&mut self, catalog: &Catalog, name: &str) -> Result<Arc<Definition>, String> {
        self.acquire(catalog, Kind::Loadout, name, Scope::Level, &mut Vec::new())
    }
    pub(super) fn release_session(&mut self, catalog: &Catalog) {
        self.records.clear();
        if super::sources::tracing() { eprintln!("INVENTORY_DECL_SESSION_RELEASE catalog={}", catalog.identity()); }
    }
    pub(super) fn release(&mut self, catalog: &Catalog, scope: Scope) {
        // Release matching payloads together before testing parent/child leases.
        for record in self.records.values_mut().filter(|record| record.scope == scope) { record.resident = None; }
        self.records.retain(|path, record| {
            record.previous.retain(|generation| generation.strong_count() != 0);
            let keep = record.scope != scope || record.current.strong_count() != 0 || !record.previous.is_empty();
            if !keep { if let Some(kind) = catalog.kind(path) { crate::loading::forget(catalog.identity(), kind, path); } }
            keep
        });
        if super::sources::tracing() { eprintln!("INVENTORY_DECL_PURGE catalog={} marker={} retained={}", catalog.identity(), scope as u8, self.records.len()); }
    }
}

/// Called from the same filtered header pass as other declaration classes.
/// Neither inventory class occurs in 0661e0's 36 heap-1 promoted kinds.
pub(crate) fn register_header(catalog: &Catalog, resource: &str) -> Result<bool, String> {
    let normalized = resource.replace('\\', "/").to_ascii_lowercase();
    if resource_kind(&normalized).is_none() { return Ok(false); }
    let sources = PickableSourceLease::for_catalog(catalog)?;
    sources.0.lock().map_err(|_| "inventory declaration owner is poisoned")?.configurations.records
        .entry(Arc::from(normalized)).or_insert_with(|| Record::new(Scope::Level));
    Ok(true)
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    let sources = PickableSourceLease::for_catalog(catalog)?;
    for record in sources.0.lock().map_err(|_| "inventory declaration owner is poisoned")?.configurations.records.values_mut() { record.scope = Scope::Permanent; }
    Ok(())
}
pub(crate) fn release_persistent(catalog: &Catalog) {
    if let Some(sources) = catalog.pickable_sources.get() {
        sources.lock().unwrap_or_else(|error| error.into_inner()).configurations.release(catalog, Scope::PersistentHeap);
    }
}
