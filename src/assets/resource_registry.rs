//! Demand-decoded immutable resource identities. Registration, explicit map
//! marker writes, bootstrap promotion and scoped purge follow
//! 1400607f0/14005feb0/1400661e0/140060b60/140060180. Class owners supply
//! their parser, dependency acquisition and locking/publication boundary.
use std::{collections::HashMap, sync::{Arc, Weak}};

struct Entry<T> {
    marker: u8,
    bootstrap_phase: bool,
    map_preload_override: bool,
    retired: bool,
    current: Weak<T>,
    resident: Option<Arc<T>>,
    failure: Option<String>,
}
impl<T> Entry<T> {
    fn new(marker: u8) -> Self {
        Self { marker, bootstrap_phase: marker == 2, map_preload_override: false,
            retired: false, current: Weak::new(), resident: None, failure: None }
    }
}
pub(crate) struct Registry<T> { entries: HashMap<String, Entry<T>> }
impl<T> Default for Registry<T> {
    fn default() -> Self { Self { entries: HashMap::new() } }
}
impl<T> Registry<T> {
    pub(crate) fn len(&self) -> usize { self.entries.len() }
    pub(crate) fn producer_scope(&self, path: &str) -> u8 {
        self.entries.get(path).map_or(0, |entry| if entry.bootstrap_phase { 2 } else { entry.marker })
    }
    pub(crate) fn register(&mut self, path: &str, marker: u8, persistent: bool) {
        let entry = self.entries.entry(path.to_owned()).or_insert_with(|| Entry::new(marker));
        if entry.retired {
            entry.marker = marker;
            entry.bootstrap_phase = marker == 2;
            entry.map_preload_override = false;
            entry.retired = false;
            entry.resident = entry.current.upgrade();
        }
        if persistent { entry.marker = 1; entry.map_preload_override = true; }
    }
    pub(crate) fn promote_bootstrap(&mut self) {
        for entry in self.entries.values_mut() {
            entry.marker = 2;
            entry.bootstrap_phase = true;
            entry.map_preload_override = false;
            entry.retired = false;
            if entry.resident.is_none() { entry.resident = entry.current.upgrade(); }
        }
    }
    pub(crate) fn acquire(&mut self, path: &str, inherited_marker: u8,
        load: impl FnOnce(u8) -> Result<Arc<T>, String>) -> Result<Arc<T>, String> {
        let entry = self.entries.entry(path.to_owned()).or_insert_with(|| Entry::new(inherited_marker));
        // A request after purge rejoins a still-live generation in the new
        // caller's scope; a previous map's marker must not be resurrected.
        if entry.retired {
            entry.marker = inherited_marker;
            entry.bootstrap_phase = inherited_marker == 2;
            entry.map_preload_override = false;
            entry.retired = false;
        }
        // A deferred bootstrap parent's dependencies were originally acquired
        // before map preload. Replay that promotion without overwriting a
        // later explicit map marker (the same phase rule as AnimSet children).
        if inherited_marker == 2 {
            entry.bootstrap_phase = true;
            if !entry.map_preload_override { entry.marker = 2; }
        }
        if let Some(error) = &entry.failure { return Err(error.clone()); }
        if let Some(value) = entry.current.upgrade() {
            entry.resident = Some(value.clone());
            return Ok(value);
        }
        match load(if entry.bootstrap_phase { 2 } else { entry.marker }) {
            Ok(value) => {
                entry.current = Arc::downgrade(&value);
                entry.resident = Some(value.clone());
                Ok(value)
            }
            Err(error) => { entry.failure = Some(error.clone()); Err(error) }
        }
    }
    pub(crate) fn release(&mut self, catalog: u64, kind: &str, marker: u8) {
        let mut released = 0;
        for entry in self.entries.values_mut() {
            if entry.marker == marker && !entry.retired {
                released += usize::from(entry.resident.take().is_some());
                entry.failure = None;
                entry.retired = true;
            }
        }
        let before = self.entries.len();
        self.entries.retain(|path, entry| {
            if !entry.retired || entry.current.strong_count() != 0 { return true; }
            crate::loading::forget(catalog, kind, path);
            false
        });
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("RESOURCE_PURGE catalog={catalog} kind={kind} marker={marker} released={released} removed={} records={}", before - self.entries.len(), self.entries.len());
        }
    }
}

