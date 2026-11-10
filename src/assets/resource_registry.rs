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
type SeparateGenerations<T> = HashMap<String, Vec<Weak<T>>>;

pub(crate) struct Registry<T> {
    entries: HashMap<String, Entry<T>>,
    separate: Option<Box<SeparateGenerations<T>>>,
}
impl<T> Default for Registry<T> {
    fn default() -> Self { Self { entries: HashMap::new(), separate: None } }
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
    fn admit(&mut self, path: &str, inherited_marker: u8) -> &mut Entry<T> {
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
        entry
    }

    pub(crate) fn acquire(&mut self, path: &str, inherited_marker: u8,
        load: impl FnOnce(u8) -> Result<Arc<T>, String>) -> Result<Arc<T>, String> {
        let entry = self.admit(path, inherited_marker);
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
    /// Editable particle generations share the path's marker and read credit.
    /// They are always reparsed and only their actual consumers retain them.
    pub(crate) fn acquire_separate(&mut self, path: &str, inherited_marker: u8,
        load: impl FnOnce(u8) -> Result<Arc<T>, String>) -> Result<Arc<T>, String> {
        let entry = self.admit(path, inherited_marker);
        let value = load(if entry.bootstrap_phase { 2 } else { entry.marker })?;
        let generations = self.separate.get_or_insert_with(Default::default).entry(path.to_owned()).or_default();
        generations.retain(|generation| generation.strong_count() != 0);
        generations.push(Arc::downgrade(&value));
        Ok(value)
    }

    /// Requested storage only: hash control bytes and allocator rounding are
    /// intentionally excluded. Weak identities do not keep decoded payloads.
    pub(crate) fn report(&self, catalog: u64, kind: &str, phase: &str,
        nested_bytes: impl Fn(&T) -> usize) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
        let record_storage_bytes = self.entries.capacity() * std::mem::size_of::<(String, Entry<T>)>()
            + self.entries.iter().map(|(path, entry)| path.capacity()
                + entry.failure.as_ref().map_or(0, String::capacity)).sum::<usize>();
        // Current host ArcInner layout: two usize atomics followed by T,
        // with the enclosing allocation padded to its alignment.
        let arc_bytes = std::alloc::Layout::new::<[usize; 2]>()
            .extend(std::alloc::Layout::new::<T>()).expect("resource Arc layout")
            .0.pad_to_align().size();
        let mut payloads = 0;
        let mut payload_bytes = 0;
        for entry in self.entries.values() {
            if let Some(value) = entry.current.upgrade() {
                payloads += 1;
                payload_bytes += arc_bytes + nested_bytes(&value);
            }
        }
        eprintln!("RESOURCE_STORAGE catalog={catalog} kind={kind} phase={phase} records={} record_storage_bytes={record_storage_bytes} payloads={payloads} payload_bytes={payload_bytes}", self.entries.len());
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
        if let Some(generations) = &mut self.separate {
            generations.retain(|_, values| {
                values.retain(|value| value.strong_count() != 0);
                !values.is_empty()
            });
            if generations.is_empty() { self.separate = None; }
        }
        let before = self.entries.len();
        let separate = self.separate.as_deref();
        self.entries.retain(|path, entry| {
            if !entry.retired || entry.current.strong_count() != 0
                || separate.is_some_and(|generations| generations.contains_key(path)) {
                return true;
            }
            crate::loading::forget(catalog, kind, path);
            false
        });
        if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
            eprintln!("RESOURCE_PURGE catalog={catalog} kind={kind} marker={marker} released={released} removed={} records={}", before - self.entries.len(), self.entries.len());
        }
    }
}

