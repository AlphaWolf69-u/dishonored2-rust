//! Session-owned cpntFaction declaration-class identities (05fc00/0610d0).
//! The real preload/declaration owner calls reserve before parsing, preserving
//! registration order and sharing the same resolved declaration leases.
use std::{borrow::Borrow, collections::HashMap, sync::{Arc, LazyLock, Mutex, Weak}};
use crate::{assets::{archive::Catalog, map_resources::Resource}, gameplay::faction_manager::Definition};

/// One canonical declaration resource name. This is session-owned and never
/// points into PhysicsScene's level-owned class NamePool.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ResourceName(Arc<str>);
impl ResourceName {
    pub(crate) fn as_str(&self) -> &str { &self.0 }
    pub(crate) fn same_identity(&self, other: &Self) -> bool { Arc::ptr_eq(&self.0, &other.0) }
}
impl Borrow<str> for ResourceName { fn borrow(&self) -> &str { &self.0 } }
struct Entry { name: ResourceName, definition: Option<Arc<Definition>>, resolving: bool }

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
}

// Like the difficulty declaration owner, this lookup does not itself keep a
// session alive. The launch owner and level consumers hold the strong leases.
static SESSIONS: LazyLock<Mutex<HashMap<u64, Weak<Mutex<FactionRegistry>>>>> =
    LazyLock::new(Default::default);

/// One launch/session owner; clones are leases, not independent registries.
/// Its names and declaration leases survive individual PhysicsScene teardown.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
#[derive(Clone)]
pub(crate) struct SessionFactionRegistry {
    catalog: u64,
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
    pub(crate) fn into_manager(self) -> Result<ActiveFactionManager, String> {
        let manager = super::faction_manager::FactionManager::initialize(self.definitions.clone())?;
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
    fn begin(&mut self, name: &str) -> Result<(ResourceName, Admission), String> {
        let normalized = name.replace('\\', "/").to_ascii_lowercase();
        if normalized.is_empty() { return Err("unnamed faction declaration".into()); }
        if let Some(&index) = self.indices.get(normalized.as_str()) {
            let entry = &self.entries[index];
            let admission = match &entry.definition {
                Some(value) => Admission::Ready(value.clone()),
                None if entry.resolving => Admission::Resolving,
                None => Admission::Pending,
            };
            return Ok((entry.name.clone(), admission));
        }
        // Appending later registrations does not renumber any live row.
        self.entries.try_reserve(1).map_err(|e| format!("faction declaration list: {e}"))?;
        self.indices.try_reserve(1).map_err(|e| format!("faction declaration lookup: {e}"))?;
        let identity = ResourceName(Arc::from(normalized));
        let index = self.entries.len();
        self.entries.push(Entry { name: identity.clone(), definition: None, resolving: false });
        self.indices.insert(identity.clone(), index);
        Ok((identity, Admission::New))
    }
}
impl SessionFactionRegistry {
    pub(crate) fn acquire(catalog: &Catalog) -> Result<Self, String> {
        let id = catalog.identity();
        let mut sessions = SESSIONS.lock().map_err(|_| "faction session lookup is poisoned")?;
        if let Some(inner) = sessions.get(&id).and_then(Weak::upgrade) {
            return Ok(Self { catalog: id, inner });
        }
        sessions.retain(|_, session| session.strong_count() != 0);
        let inner = Arc::new(Mutex::new(FactionRegistry::default()));
        sessions.insert(id, Arc::downgrade(&inner));
        Ok(Self { catalog: id, inner })
    }
    /// Remove only the lookup at catalog shutdown. Existing leases retain the
    /// same registry until their own consumers have finished retiring.
    pub(crate) fn release_catalog(id: u64) {
        SESSIONS.lock().unwrap_or_else(|error| error.into_inner()).remove(&id);
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
            Admission::Ready(definition) => return Ok(definition),
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
            Ok(definition)
        });
        if result.is_err() { self.abandon_resolution(&identity)?; }
        result
    }
    /// The 0661e0 manifest registration pass reserves eligible missing
    /// resources before the later parse/preload pass. The caller invokes this
    /// only for the actual session/map manifest being mounted, in file order.
    /// It must still process all other resource types through the real loader,
    /// whose recursive declaration acquisitions also call begin on this owner.
    pub(crate) fn reserve_manifest<'a>(&self, catalog: &Catalog,
        resources: impl IntoIterator<Item = &'a Resource>) -> Result<usize, String> {
        self.check_catalog(catalog)?;
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut added = 0;
        for resource in resources {
            if !resource.kind.eq_ignore_ascii_case("cpntfaction") || !catalog.preloads(&resource.path) { continue; }
            let name = resource.path.strip_prefix("generated/decls/cpntfaction/")
                .and_then(|value| value.strip_suffix(".decl"))
                .ok_or("invalid faction manifest resource path")?;
            if matches!(owner.begin(name)?.1, Admission::New) { added += 1; }
        }
        Ok(added)
    }
    /// Pick the next registered resource in native array order. Resolution
    /// may append referenced declarations; the caller repeats until none are
    /// pending before the faction manager takes its numeric snapshot.
    pub(crate) fn next_pending(&self) -> Result<Option<ResourceName>, String> {
        let owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        if owner.entries.iter().any(|entry| entry.resolving) {
            return Err("cannot finish faction preload during recursive resolution".into());
        }
        Ok(owner.entries.iter().find(|entry| entry.definition.is_none()).map(|entry| entry.name.clone()))
    }
    /// Resolve only an already-parsed class entry. Component initialization
    /// must never turn a missing declaration into a manufactured faction.
    pub(crate) fn definition(&self, name: &str) -> Result<Arc<Definition>, String> {
        let owner = self.inner.lock().map_err(|_| "faction registry is poisoned")?;
        let normalized = name.replace('\\', "/").to_ascii_lowercase();
        let index = *owner.indices.get(normalized.as_str()).ok_or("faction declaration is not registered")?;
        owner.entries[index].definition.clone().ok_or_else(|| "faction declaration is not parsed".into())
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
        if entry.definition.is_some() || entry.resolving { return Ok(false); }
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
        if entry.definition.is_some() || !entry.resolving { return Err("faction declaration is not resolving".into()); }
        entry.resolving = false;
        Ok(())
    }

    /// Retains the exact existing source/ancestor lease. Failed resolution
    /// leaves its allocated slot pending, so later registrations keep order.
    pub(crate) fn finish(&self, definition: Arc<Definition>) -> Result<(), String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let index = *owner.indices.get(&definition.name).ok_or("faction declaration was not registered")?;
        let entry = &mut owner.entries[index];
        if !entry.name.same_identity(&definition.name) {
            return Err("faction declaration belongs to another name namespace".into());
        }
        if let Some(existing) = &entry.definition {
            if !Arc::ptr_eq(&existing.lease, &definition.lease) {
                return Err("faction declaration identity changed without retirement".into());
            }
            return Ok(());
        }
        if !entry.resolving { return Err("faction declaration source was not acquired".into()); }
        entry.definition = Some(definition);
        entry.resolving = false;
        Ok(())
    }
    /// 0610d0 swaps the final resource into the removed index. The caller
    /// invokes this at the real declaration release, after dependent matrices.
    pub(crate) fn retire(&self, name: &ResourceName) -> Result<bool, String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(&index) = owner.indices.get(name) else { return Ok(false) };
        if !owner.entries[index].name.same_identity(name) {
            return Err("faction declaration belongs to another name namespace".into());
        }
        if owner.active_snapshot.as_ref().and_then(Weak::upgrade)
            .is_some_and(|snapshot| index < snapshot.count) {
            return Err("cannot retire a faction identity used by a live matrix".into());
        }
        owner.indices.remove(name);
        owner.entries.swap_remove(index);
        if let Some(moved) = owner.entries.get(index).map(|entry| entry.name.clone()) {
            owner.indices.insert(moved, index);
        }
        Ok(true)
    }
    /// 604f90 visits the live class array; unresolved entries stay explicit.
    pub(crate) fn snapshot(&self) -> Result<Snapshot, String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        owner.can_change_order()?;
        let mut definitions = Vec::with_capacity(owner.entries.len());
        for entry in &owner.entries {
            definitions.push(entry.definition.as_ref().ok_or("registered faction declaration is still unresolved")?.clone());
        }
        let token = Arc::new(SnapshotPin { count: definitions.len() });
        owner.active_snapshot = Some(Arc::downgrade(&token));
        Ok(Snapshot { definitions, _token: token })
    }
    /// Session shutdown, after all live matrix snapshots have retired.
    pub(crate) fn clear(&self) -> Result<(), String> {
        let mut owner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        owner.can_change_order()?;
        owner.entries.clear();
        owner.indices.clear();
        owner.active_snapshot = None;
        Ok(())
    }
}
