//! Session-owned cpntFaction declaration-class identities (05fc00/0610d0).
//! The real preload/declaration owner registers headers before parsing, preserving
//! registration order and sharing the same resolved declaration leases.
use std::{borrow::Borrow, collections::HashMap, sync::{Arc, Mutex, Weak}};
use crate::{assets::archive::Catalog, gameplay::faction_manager::Definition};
use super::options::decl::field;

/// One canonical declaration resource name. This is session-owned and never
/// points into PhysicsScene's level-owned class NamePool.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ResourceName(Arc<str>);
impl ResourceName {
    pub(crate) fn as_str(&self) -> &str { &self.0 }
    pub(crate) fn same_identity(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
impl Borrow<str> for ResourceName { fn borrow(&self) -> &str { &self.0 } }
fn resource_path(name: &str) -> String { format!("generated/decls/cpntfaction/{name}.decl") }
struct Entry { name: ResourceName, definition: Option<Weak<Definition>>, prepared: Option<Arc<Definition>>, resolving: bool }

pub(crate) enum Admission {
    /// Caller owns the new slot and finishes it after real source resolution.
    New,
    /// The preload pass registered a placeholder that has not been parsed.
    Pending,
    /// A recursive request reached a resource currently being parsed.
    Resolving,
    /// Reuse the exact existing declaration/ancestor lease; do not reread it.
    Ready(Arc<Definition>),
}

#[derive(Default)]
struct FactionRegistry {
    entries: Vec<Entry>,
    indices: HashMap<ResourceName, usize>,
    active_snapshot: Option<Weak<SnapshotPin>>,
    resources: crate::assets::resource_registry::Registry<Definition>,
    names: HashMap<String, Weak<str>>,
    sources: HashMap<ResourceName, Weak<crate::assets::entities::spawn::ComponentDeclarationLease>>,
}

/// One launch/session owner; clones are leases, not independent registries.
/// Its names and declaration leases survive individual PhysicsScene teardown.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
#[derive(Clone)]
pub(crate) struct SessionFactionRegistry {
    catalog: u64,
    shared: bool,
    inner: Arc<Mutex<FactionRegistry>>,
}

/// A map's faction manager retains this while native numeric indices are in
/// use. It carries no extra parsed source tree and shares each declaration.
struct SnapshotPin { count: usize }
pub(crate) struct Snapshot {
    definitions: Vec<Arc<Definition>>,
    _token: Arc<SnapshotPin>,
}
/// The matrix and its exact registry snapshot retire together; callers cannot
/// accidentally unpin numeric identities while keeping the matrix alive.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub(crate) struct ActiveFactionManager {
    manager: super::faction_manager::FactionManager,
    _snapshot: Snapshot,
}
impl ActiveFactionManager {
    pub(crate) fn manager(&self) -> &super::faction_manager::FactionManager { &self.manager }
    pub(crate) fn manager_mut(&mut self) -> &mut super::faction_manager::FactionManager { &mut self.manager }
}
impl Snapshot {
    pub(crate) fn definitions(&self) -> &[Arc<Definition>] { &self.definitions }
    pub(crate) fn into_manager(mut self) -> Result<ActiveFactionManager, String> {
        let manager = super::faction_manager::FactionManager::initialize(std::mem::take(&mut self.definitions))?;
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("FACTION_MATRIX_SEMANTICS rows={} signature={}", self._token.count, manager.semantic_signature());
        }
        Ok(ActiveFactionManager { manager, _snapshot: self })
    }
}
impl FactionRegistry {
    fn can_change_order(&self) -> Result<(), String> {
        if self.active_snapshot.as_ref().and_then(Weak::upgrade).is_some() {
            return Err("faction declaration registry is retained by a live matrix".into());
        }
        Ok(())
    }
    fn begin(&mut self, name: &str) -> Result<(ResourceName, Admission), String> { self.begin_scoped(name, 0) }
    fn begin_scoped(&mut self, name: &str, marker: u8) -> Result<(ResourceName, Admission), String> {
        let normalized = name.replace('\\', "/").to_ascii_lowercase();
        if normalized.is_empty() { return Err("unnamed faction declaration".into()); }
        if let Some(&index) = self.indices.get(normalized.as_str()) {
            let entry = &self.entries[index];
            let admission = match entry.definition.as_ref().and_then(Weak::upgrade) {
                Some(value) => Admission::Ready(value),
                None if entry.resolving => Admission::Resolving,
                None => Admission::Pending,
            };
            return Ok((entry.name.clone(), admission));
        }
        // Appending later registrations does not renumber any live row.
        self.entries.try_reserve(1).map_err(|e| format!("faction declaration list: {e}"))?;
        self.indices.try_reserve(1).map_err(|e| format!("faction declaration lookup: {e}"))?;
        let text = self.names.get(&normalized).and_then(Weak::upgrade).unwrap_or_else(|| {
            let text: Arc<str> = Arc::from(normalized.as_str());
            self.names.insert(normalized.clone(), Arc::downgrade(&text));
            text
        });
        let identity = ResourceName(text);
        self.resources.register(&resource_path(identity.as_str()), marker, false);
        let index = self.entries.len();
        self.entries.push(Entry { name: identity.clone(), definition: None, prepared: None, resolving: false });
        self.indices.insert(identity.clone(), index);
        Ok((identity, Admission::New))
    }
}
impl SessionFactionRegistry {
    pub(crate) fn acquire(catalog: &Catalog) -> Result<Self, String> {
        Ok(catalog.faction_registry.get_or_init(|| Self { catalog: catalog.identity(), shared: !catalog.has_overlay_declaration("generated/decls/cpntfaction/"),
            inner: Arc::new(Mutex::new(FactionRegistry::default())) }).clone())
    }
    /// Editable resource layers are a host feature. Reparse their generation
    /// at the real map-preparation boundary, after the outgoing matrix retires;
    /// names and registry order remain the same. Installed archives keep reuse.
    pub(crate) fn begin_preparation(&self, catalog: &Catalog) -> Result<(), String> {
        self.check_catalog(catalog)?;
        if !catalog.has_overlay_declaration("generated/decls/cpntfaction/") { return Ok(()); }
        let mut owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        owner.can_change_order()?;
        if owner.entries.iter().any(|entry| entry.resolving) { return Err("faction preparation overlaps active declaration resolution".into()); }
        for entry in &mut owner.entries { entry.definition = None; entry.prepared = None; }
        owner.sources.clear();
        Ok(())
    }

    /// Complete one source through the same class registry used by Initialize.
    /// The serial map preparation owns acquisition; gameplay callbacks only
    /// borrow these prepared leases and never read archives.
    pub(crate) fn acquire_definition(&self, catalog: &Catalog, name: &str) -> Result<Arc<Definition>, String> {
        use crate::assets::entities::{self, Value, event_graph::atom};
        let (identity, _) = self.begin(catalog, name)?;
        let marker = self.inner.lock().map_err(|_| "faction registry is poisoned")?.resources.producer_scope(&resource_path(identity.as_str()));
        let mut decoded = false;
        let result = self.resolve(catalog, name, |registry, identity| {
            decoded = true;
            let _tag = crate::platform::allocation_tags::tag("Faction declarations");
            let lease = registry.source_lease(catalog, identity.as_str(), marker, &mut Vec::new())?;
            let fields = lease.faction_fields()?;
            let parent = lease.parent().map(|value| registry.begin_dependency(catalog, value.source_name(), marker).map(|value| value.0)).transpose()?;
            fn relation(value: &str) -> Result<u32, String> {
                match value.trim_matches('"') { "ALLY" => Ok(0), "NEUTRAL" => Ok(1), "ENEMY" => Ok(2), "UNDEFINED" => Ok(3), value => Err(format!("unknown faction relationship {value}")) }
            }
            let default_relationship = match field(&fields, "m_defaultRelationship") {
                Some(Value::Atom(value)) => relation(value)?, None => 1, // 955430: NEUTRAL
                _ => return Err("faction default is not scalar".into()),
            };
            let propagate_overrides = match field(&fields, "m_propagateOverwrite") {
                Some(Value::Atom(value)) if matches!(value.as_str(), "true" | "1") => true,
                Some(Value::Atom(value)) if matches!(value.as_str(), "false" | "0") => false,
                None => false, // 955430 initializes +0x90 to zero.
                _ => return Err("invalid faction propagation flag".into()),
            };
            let mut relationships = Vec::new();
            if let Some(value) = field(&fields, "m_factionRelationship") {
                let Value::Scope(items) = value else { return Err("faction relationships are not a scope".into()); };
                for (key, value) in items {
                    if key == "item_begin" { continue; }
                    if !key.starts_with("item_add[") && !key.starts_with("item[") { return Err(format!("unsupported faction relationship entry {key}")); }
                    let Value::Scope(item) = value else { return Err("faction relationship entry is not a scope".into()); };
                    let target = atom(item, "m_faction").ok_or("faction relationship has no target")?;
                    let value = relation(atom(item, "m_relationship").ok_or("faction relationship has no value")?)?;
                    relationships.push((registry.begin_dependency(catalog, target, marker)?.0, value));
                }
            }
            Ok(Arc::new(Definition { faction_name: Arc::from(atom(&fields, "m_factionName").unwrap_or("")),
                lease, name: identity.clone(), parent, default_relationship, relationships, propagate_overrides }))
        });
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            if let Ok(value) = &result {
                eprintln!("FACTION_RESOURCE_ACQUIRE name={} marker={marker} shared={} decoded={decoded} definition={:p} source={:p}",
                    value.name.as_str(), self.shared, Arc::as_ptr(value), Arc::as_ptr(&value.lease));
            }
        }
        result
    }

    /// References parsed above may append entries. Drain precisely that same
    /// registered order before capturing the numeric matrix snapshot.
    pub(crate) fn acquire_all_registered(&self, catalog: &Catalog) -> Result<(), String> {
        while let Some(name) = self.next_pending()? { self.acquire_definition(catalog, name.as_str())?; }
        Ok(())
    }

    fn source_lease(&self, catalog: &Catalog, name: &str, inherited_marker: u8, active: &mut Vec<String>)
        -> Result<Arc<crate::assets::entities::spawn::ComponentDeclarationLease>, String> {
        use crate::assets::entities::{self, Value, event_graph::atom, spawn::ComponentDeclarationLease};
        let (identity, _) = self.begin_dependency(catalog, name, inherited_marker)?;
        let (source, marker) = {
            let owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
            (owner.sources.get(&identity).and_then(Weak::upgrade), owner.resources.producer_scope(&resource_path(identity.as_str())))
        };
        if let Some(source) = source {
            let mut parent = source.parent();
            while let Some(value) = parent {
                self.begin_dependency(catalog, value.source_name(), marker)?;
                parent = value.parent();
            }
            return Ok(source);
        }
        let path = resource_path(identity.as_str());
        if active.contains(&path) || active.len() > 16 { return Err(format!("faction declaration inheritance cycle or depth limit at {path}")); }
        active.push(path.clone());
        let result = (|| {
            let bytes = catalog.read(&path).map_err(|error| error.to_string())?
                .ok_or_else(|| format!("faction declaration missing: {path}"))?;
            let text: Arc<str> = Arc::from(entities::owned_declaration_text(bytes));
            let fields = entities::parse_declaration(&text).map_err(|error| format!("{path}: {error}"))?;
            let parent = match field(&fields, "inherit") {
                None => None,
                Some(Value::Atom(name)) if !name.trim_matches('"').is_empty() => Some(self.source_lease(catalog, name.trim_matches('"'), marker, active)?),
                Some(_) => return Err(format!("{path}: invalid faction inheritance reference")),
            };
            let class = atom(&fields, "class").unwrap_or("").to_owned();
            let source = Arc::new(entities::Definition { kind: "component".into(), class, id: name.to_owned(), fields });
            let lease = Arc::new(ComponentDeclarationLease::new(source, parent, Some(text)));
            let mut owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
            if let Some(existing) = owner.sources.get(&identity).and_then(Weak::upgrade) { return Ok(existing); }
            owner.sources.insert(identity, Arc::downgrade(&lease));
            Ok(lease)
        })();
        active.pop();
        result
    }

    fn begin_dependency(&self, catalog: &Catalog, name: &str, marker: u8) -> Result<(ResourceName, Admission), String> {
        self.check_catalog(catalog)?;
        let mut owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        let result = owner.begin_scoped(name, marker)?;
        owner.resources.inherit_scope(&resource_path(result.0.as_str()), marker);
        Ok(result)
    }

    fn pin_ready(&self, catalog: &Catalog, definition: &Arc<Definition>) -> Result<(), String> {
        self.check_catalog(catalog)?;
        let mut owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        let path = resource_path(definition.name.as_str());
        let marker = owner.resources.producer_scope(&path);
        if self.shared { owner.resources.acquire(&path, marker, |_| Ok(definition.clone()))?; }
        if marker != 2 { return Ok(()); }
        // Deferred bootstrap parses inherit that original creation phase,
        // including dependencies of a previously decoded declaration.
        let mut pending = vec![definition.clone()];
        let mut seen = std::collections::HashSet::new();
        while let Some(value) = pending.pop() {
            if !seen.insert(value.name.clone()) { continue; }
            for name in value.parent.iter().chain(value.relationships.iter().map(|entry| &entry.0)) {
                let (identity, admission) = owner.begin_scoped(name.as_str(), marker)?;
                owner.resources.inherit_scope(&resource_path(identity.as_str()), marker);
                if let Admission::Ready(value) = admission { pending.push(value); }
            }
        }
        Ok(())
    }

    fn check_catalog(&self, catalog: &Catalog) -> Result<(), String> {
        if self.catalog != catalog.identity() { return Err("faction registry belongs to another catalog session".into()); }
        Ok(())
    }
    /// 0607f0/05fc00 publish before parsing. Reentrant reference acquisition
    /// calls this same method and receives the same canonical ResourceName.
    pub(crate) fn begin(&self, catalog: &Catalog, name: &str) -> Result<(ResourceName, Admission), String> {
        self.check_catalog(catalog)?;
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).begin(name)
    }
    /// Resolve through the caller's existing declaration-source owner. The
    /// class entry exists before the callback reads or parses any source, and
    /// recursive parent acquisition uses this same registry. No mutex is held
    /// during the callback, which may register other declarations.
    pub(crate) fn resolve<F>(&self, catalog: &Catalog, name: &str, load: F)
        -> Result<Arc<Definition>, String>
    where F: FnOnce(&Self, &ResourceName) -> Result<Arc<Definition>, String> {
        let (identity, admission) = self.begin(catalog, name)?;
        match admission {
            Admission::Ready(definition) => {
                self.pin_ready(catalog, &definition)?;
                return Ok(definition);
            }
            Admission::Resolving => return Err(format!("faction declaration inheritance cycle at {}", identity.as_str())),
            Admission::New | Admission::Pending => {}
        }
        if !self.begin_resolution(&identity)? {
            return Err(format!("faction declaration is already resolving: {}", identity.as_str()));
        }
        let result = load(self, &identity).and_then(|definition| {
            if !definition.name.same_identity(&identity) {
                return Err("faction declaration source returned a different resource identity".into());
            }
            self.finish(definition.clone())?;
            self.pin_ready(catalog, &definition)?;
            Ok(definition)
        });
        if result.is_err() {
            let resolving = {
                let owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
                owner.indices.get(&identity).is_some_and(|&index| owner.entries[index].resolving)
            };
            if resolving { self.abandon_resolution(&identity)?; }
        }
        result
    }
    /// Pick the next registered resource in native array order. Resolution
    /// may append referenced declarations; the caller repeats until none are
    /// pending before the faction manager takes its numeric snapshot.
    pub(crate) fn next_pending(&self) -> Result<Option<ResourceName>, String> {
        let owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        if owner.entries.iter().any(|entry| entry.resolving) {
            return Err("cannot finish faction preload during recursive resolution".into());
        }
        Ok(owner.entries.iter().find(|entry| entry.definition.as_ref().and_then(Weak::upgrade).is_none()).map(|entry| entry.name.clone()))
    }
    /// Resolve only an already-parsed class entry. Component initialization
    /// must never turn a missing declaration into a manufactured faction.
    pub(crate) fn definition(&self, name: &str) -> Result<Arc<Definition>, String> {
        let owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        let normalized = name.replace('\\', "/").to_ascii_lowercase();
        let index = *owner.indices.get(normalized.as_str()).ok_or("faction declaration is not registered")?;
        owner.entries[index].definition.as_ref().and_then(Weak::upgrade).ok_or_else(|| "faction declaration is not parsed".into())
    }
    /// 5bd9a0 captures the actual class-array index during Initialize.
    pub(crate) fn index_of(&self, name: &ResourceName) -> Result<u8, String> {
        let owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        let index = *owner.indices.get(name).ok_or("faction declaration is not registered")?;
        if !owner.entries[index].name.same_identity(name) { return Err("faction identity belongs to another session".into()); }
        if index > i8::MAX as usize { return Err("faction index exceeds native signed byte".into()); }
        Ok(index as u8)
    }
    /// Transition an existing placeholder into the real source parse. A
    /// recursive encounter returns false and uses the already allocated name
    /// identity; it must not start a second parse or duplicate the lease.
    pub(crate) fn begin_resolution(&self, name: &ResourceName) -> Result<bool, String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let index = *owner.indices.get(name).ok_or("faction declaration was not registered")?;
        let entry = &mut owner.entries[index];
        if !entry.name.same_identity(name) { return Err("faction declaration belongs to another name namespace".into()); }
        if entry.definition.as_ref().and_then(Weak::upgrade).is_some() || entry.resolving { return Ok(false); }
        entry.resolving = true;
        Ok(true)
    }
    /// Failed source parse retains the native resource position and permits a
    /// later real retry; it never silently removes or reorders the entry.
    pub(crate) fn abandon_resolution(&self, name: &ResourceName) -> Result<(), String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let index = *owner.indices.get(name).ok_or("faction declaration was not registered")?;
        let entry = &mut owner.entries[index];
        if !entry.name.same_identity(name) { return Err("faction declaration belongs to another name namespace".into()); }
        if entry.definition.as_ref().and_then(Weak::upgrade).is_some() || !entry.resolving { return Err("faction declaration is not resolving".into()); }
        entry.resolving = false;
        Ok(())
    }

    /// Retains the exact existing source/ancestor lease. Failed resolution
    /// leaves its allocated slot pending, so later registrations keep order.
    pub(crate) fn finish(&self, definition: Arc<Definition>) -> Result<(), String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let index = *owner.indices.get(&definition.name).ok_or("faction declaration was not registered")?;
        let entry = &owner.entries[index];
        if !entry.name.same_identity(&definition.name) { return Err("faction declaration belongs to another name namespace".into()); }
        if let Some(existing) = entry.definition.as_ref().and_then(Weak::upgrade) {
            if !Arc::ptr_eq(&existing.lease, &definition.lease) { return Err("faction declaration identity changed without retirement".into()); }
            return Ok(());
        }
        if !entry.resolving { return Err("faction declaration source was not acquired".into()); }
        let path = resource_path(definition.name.as_str());
        let marker = owner.resources.producer_scope(&path);
        let shared = self.shared;
        let value = owner.resources.acquire_with_sharing(&path, marker, shared, |_| Ok(definition))?;
        let entry = &mut owner.entries[index];
        entry.definition = Some(Arc::downgrade(&value));
        entry.prepared = (!shared).then_some(value);
        entry.resolving = false;
        Ok(())
    }
    /// 604f90 visits the live class array; unresolved entries stay explicit.
    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        owner.can_change_order()?;
        let mut definitions = Vec::with_capacity(owner.entries.len());
        for entry in &owner.entries {
            definitions.push(entry.definition.as_ref().and_then(Weak::upgrade).ok_or("registered faction declaration is still unresolved")?);
        }
        for entry in &mut owner.entries { entry.prepared = None; }
        let token = Arc::new(SnapshotPin { count: definitions.len() });
        owner.active_snapshot = Some(Arc::downgrade(&token));
        Ok(Snapshot { definitions, _token: token })
    }

}

/// 0661e0's selected header callback is the only preload registration route.
/// cpntFaction is not in its conditional heap-1 promotion list.
pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8) -> Result<(), String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    let name = path.strip_prefix("generated/decls/cpntfaction/").and_then(|path| path.strip_suffix(".decl"))
        .ok_or("invalid faction declaration resource path")?;
    let registry = SessionFactionRegistry::acquire(catalog)?;
    registry.inner.lock().map_err(|_| "faction registry is poisoned")?.begin_scoped(name, marker)?;
    Ok(())
}

pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(registry) = catalog.faction_registry.get() {
        registry.inner.lock().map_err(|_| "faction registry is poisoned")?.resources.promote_bootstrap();
    }
    Ok(())
}

pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    let Some(registry) = catalog.faction_registry.get() else { return; };
    let mut owner = registry.inner.lock().unwrap_or_else(|error| error.into_inner());
    // This scope boundary follows acquisition-worker and manager retirement.
    // Editable generations had only a temporary admission pin, never a cache pin.
    for entry in &mut owner.entries { entry.prepared = None; }
    let kind = owner.entries.iter().find_map(|entry| catalog.kind(&resource_path(entry.name.as_str())))
        .unwrap_or("cpntFaction").to_owned();
    owner.resources.release(catalog.identity(), &kind, marker);
    let pinned = owner.active_snapshot.as_ref().and_then(Weak::upgrade).map_or(0, |pin| pin.count);
    let mut index = pinned;
    while index < owner.entries.len() {
        let name = owner.entries[index].name.clone();
        if owner.resources.contains(&resource_path(name.as_str())) { index += 1; continue; }
        // 0610d0 swaps the final class entry into this retired slot.
        owner.indices.remove(&name);
        owner.entries.swap_remove(index);
        if let Some(moved) = owner.entries.get(index).map(|entry| entry.name.clone()) { owner.indices.insert(moved, index); }
    }
    owner.sources.retain(|_, source| source.strong_count() != 0);
    owner.names.retain(|_, name| name.strong_count() != 0);
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
        owner.resources.report(catalog.identity(), &kind, "scope-released", |value|
            value.relationships.capacity() * std::mem::size_of::<(ResourceName, u32)>() + value.faction_name.len());
        eprintln!("FACTION_RESOURCE_SCOPE catalog={} marker={marker} classes={} sources={} pinned_rows={pinned}",
            catalog.identity(), owner.entries.len(), owner.sources.len());
    }
}
