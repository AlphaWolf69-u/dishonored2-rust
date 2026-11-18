//! Demand-decoded immutable resource identities. Registration, explicit map
//! marker writes, bootstrap promotion and scoped purge follow
//! 1400607f0/14005feb0/1400661e0/140060b60/140060180. Class owners supply
//! their parser, dependency acquisition and locking/publication boundary.
use std::{collections::HashMap, sync::{Arc, Weak}};

struct Entry<T: ?Sized, E> {
    marker: u8,
    bootstrap_phase: bool,
    map_preload_override: bool,
    retired: bool,
    current: Option<Weak<T>>,
    resident: Option<Arc<T>>,
    failure: Option<E>,
}
impl<T: ?Sized, E> Entry<T, E> {
    fn new(marker: u8) -> Self {
        Self { marker, bootstrap_phase: marker == 2, map_preload_override: false,
            retired: false, current: None, resident: None, failure: None }
    }
}
type SeparateGenerations<T> = HashMap<String, Vec<Weak<T>>>;

pub(crate) struct Registry<T: ?Sized, E = String> {
    entries: HashMap<String, Entry<T, E>>,
    separate: Option<Box<SeparateGenerations<T>>>,
}
impl<T: ?Sized, E> Default for Registry<T, E> {
    fn default() -> Self { Self { entries: HashMap::new(), separate: None } }
}
impl<T: ?Sized, E: Clone> Registry<T, E> {
    pub(crate) fn contains(&self, path: &str) -> bool { self.entries.contains_key(path) }
    /// Register a real dependency before its deferred payload is parsed,
    /// preserving the same bootstrap/explicit-map phase rules as acquire.
    pub(crate) fn inherit_scope(&mut self, path: &str, marker: u8) { self.admit(path, marker); }
    pub(crate) fn len(&self) -> usize { self.entries.len() }
    #[cfg(feature = "game")]
    pub(crate) fn registered_paths(&self) -> Vec<&str> {
        let mut paths = self.entries.keys().map(String::as_str).collect::<Vec<_>>();
        paths.sort_unstable();
        paths
    }

    #[cfg(feature = "game")]
    pub(crate) fn preload_paths(&self) -> Vec<&str> {
        let mut paths = self.entries.iter().filter(|(_, entry)|
            !entry.retired && (entry.marker != 0 || entry.bootstrap_phase))
            .map(|(path, _)| path.as_str()).collect::<Vec<_>>();
        paths.sort_unstable();
        paths
    }

    /// Class owners use this at scope boundaries to release dependency read
    /// history without retaining an additional strong payload table.
    pub(crate) fn live_values(&self) -> impl Iterator<Item = Arc<T>> + '_ {
        self.entries.values().filter_map(|entry| entry.current.as_ref().and_then(Weak::upgrade))
            .chain(self.separate.iter().flat_map(|values| values.values())
                .flat_map(|values| values.iter().filter_map(Weak::upgrade)))
    }

    pub(crate) fn owns_read_history(&self, path: &str) -> bool {
        self.entries.get(path).is_some_and(|entry|
            entry.current.as_ref().is_some_and(|value| value.strong_count() != 0) || entry.failure.is_some())
            || self.separate.as_ref().and_then(|generations| generations.get(path))
                .is_some_and(|values| values.iter().any(|generation| generation.strong_count() != 0))
    }
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
            entry.resident = entry.current.as_ref().and_then(Weak::upgrade);
        }
        if persistent { entry.marker = 1; entry.map_preload_override = true; }
    }
    pub(crate) fn promote_bootstrap(&mut self) {
        for entry in self.entries.values_mut() {
            entry.marker = 2;
            entry.bootstrap_phase = true;
            entry.map_preload_override = false;
            entry.retired = false;
            if entry.resident.is_none() { entry.resident = entry.current.as_ref().and_then(Weak::upgrade); }
        }
    }
    fn admit(&mut self, path: &str, inherited_marker: u8) -> &mut Entry<T, E> {
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
        load: impl FnOnce(u8) -> Result<Arc<T>, E>) -> Result<Arc<T>, E> {
        self.acquire_when(path, inherited_marker, load, || true)
    }

    /// A worker withdrawal is not a failed installed resource. Keep its
    /// registered header but publish neither its value nor its error.
    pub(crate) fn acquire_when(&mut self, path: &str, inherited_marker: u8,
        load: impl FnOnce(u8) -> Result<Arc<T>, E>, publish: impl FnOnce() -> bool,
    ) -> Result<Arc<T>, E> {
        self.acquire_prepared_when(path, inherited_marker, load, |_| Ok(()), publish)
    }

    /// Complete a consumer's fallible preparation before restoring residency.
    /// An old consumer may lease a purged generation whose new role needs more
    /// decoding; withdrawal must leave that generation unpinned. Preparation
    /// failures are consumer-local, unlike immutable errors from `load`.
    pub(crate) fn acquire_prepared_when(&mut self, path: &str, inherited_marker: u8,
        load: impl FnOnce(u8) -> Result<Arc<T>, E>,
        prepare: impl FnOnce(&Arc<T>) -> Result<(), E>, publish: impl FnOnce() -> bool,
    ) -> Result<Arc<T>, E> {
        let entry = self.admit(path, inherited_marker);
        if let Some(error) = &entry.failure { return Err(error.clone()); }
        if let Some(value) = entry.current.as_ref().and_then(Weak::upgrade) {
            prepare(&value)?;
            // A live consumer may still lease a generation whose cache pin
            // was retired. A withdrawn worker must not restore that pin.
            // An already resident value needs no redundant Arc replacement.
            if entry.resident.is_none() && publish() {
                entry.resident = Some(value.clone());
            }
            return Ok(value);
        }
        let loaded = load(if entry.bootstrap_phase { 2 } else { entry.marker });
        if let Ok(value) = &loaded { prepare(value)?; }
        if !publish() { return loaded; }
        match loaded {
            Ok(value) => {
                entry.current = Some(Arc::downgrade(&value));
                entry.resident = Some(value.clone());
                Ok(value)
            }
            Err(error) => { entry.failure = Some(error.clone()); Err(error) }
        }
    }
    pub(crate) fn acquire_with_sharing(&mut self, path: &str, marker: u8, shared: bool,
        load: impl FnOnce(u8) -> Result<Arc<T>, E>) -> Result<Arc<T>, E> {
        if shared { self.acquire(path, marker, load) }
        else { self.acquire_separate(path, marker, load) }
    }

    /// Editable generations share the path's marker and read credit.
    /// They are always reparsed and only their actual consumers retain them.
    fn acquire_separate(&mut self, path: &str, inherited_marker: u8,
        load: impl FnOnce(u8) -> Result<Arc<T>, E>) -> Result<Arc<T>, E> {
        let entry = self.admit(path, inherited_marker);
        let value = load(if entry.bootstrap_phase { 2 } else { entry.marker })?;
        let generations = self.separate.get_or_insert_with(Default::default).entry(path.to_owned()).or_default();
        generations.retain(|generation| generation.strong_count() != 0);
        generations.push(Arc::downgrade(&value));
        Ok(value)
    }

    /// Requested storage only: hash control bytes and allocator rounding are
    /// intentionally excluded. Weak identities do not keep decoded payloads.
    pub(crate) fn report_with(&self, catalog: u64, kind: &str, phase: &str,
        nested_bytes: impl Fn(&T) -> usize, failure_bytes: impl Fn(&E) -> usize) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
        let mut record_storage_bytes = self.entries.capacity() * std::mem::size_of::<(String, Entry<T, E>)>()
            + self.entries.iter().map(|(path, entry)| path.capacity()
                + entry.failure.as_ref().map_or(0, &failure_bytes)).sum::<usize>();
        // Current host ArcInner layout: two usize atomics followed by T,
        // with the enclosing allocation padded to its alignment.
        let payload_size = |value: &T| {
            std::alloc::Layout::new::<[usize; 2]>()
                .extend(std::alloc::Layout::for_value(value)).expect("resource Arc layout")
                .0.pad_to_align().size() + nested_bytes(value)
        };
        let mut payloads = 0;
        let mut payload_bytes = 0;
        for entry in self.entries.values() {
            if let Some(value) = entry.current.as_ref().and_then(Weak::upgrade) {
                payloads += 1;
                payload_bytes += payload_size(&value);
            }
        }
        if let Some(separate) = &self.separate {
            record_storage_bytes += std::mem::size_of_val(separate.as_ref())
                + separate.capacity() * std::mem::size_of::<(String, Vec<Weak<T>>)>();
            for (path, generations) in separate.iter() {
                record_storage_bytes += path.capacity() + generations.capacity() * std::mem::size_of::<Weak<T>>();
                for generation in generations.iter().filter_map(Weak::upgrade) {
                    payloads += 1;
                    payload_bytes += payload_size(&generation);
                }
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
            if !entry.retired || entry.current.as_ref().is_some_and(|value| value.strong_count() != 0)
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

impl<T: ?Sized> Registry<T> {
    pub(crate) fn report(&self, catalog: u64, kind: &str, phase: &str,
        nested_bytes: impl Fn(&T) -> usize) {
        self.report_with(catalog, kind, phase, nested_bytes, String::capacity);
    }
}
