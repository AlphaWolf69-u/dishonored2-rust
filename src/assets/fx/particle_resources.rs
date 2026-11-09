//! Catalog identities for immutable particle payloads (1400607f0/14005feb0).
//! 140060b60 promotes bootstrap headers; 1400661e0 explicitly marks selected
//! map headers; 140060180 releases only the requested marker. Payload decoding
//! stays demand driven, a host deviation from eager preload deserialization.
use crate::{archive::Catalog, assets::{fx::particle_declaration::ParticleDeclaration, particle_model::ParticleModel}};
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}};

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
struct Registry<T> { entries: HashMap<String, Entry<T>> }
impl<T> Default for Registry<T> {
    fn default() -> Self { Self { entries: HashMap::new() } }
}
impl<T> Registry<T> {
    fn register(&mut self, path: &str, marker: u8, persistent: bool) {
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
    fn promote_bootstrap(&mut self) {
        for entry in self.entries.values_mut() {
            entry.marker = 2;
            entry.bootstrap_phase = true;
            entry.map_preload_override = false;
            entry.retired = false;
            if entry.resident.is_none() { entry.resident = entry.current.upgrade(); }
        }
    }
    fn acquire(&mut self, path: &str, inherited_marker: u8,
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
    fn release(&mut self, catalog: u64, kind: &str, marker: u8) {
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
            eprintln!("PARTICLE_RESOURCE_PURGE catalog={catalog} kind={kind} marker={marker} released={released} removed={} records={}", before - self.entries.len(), self.entries.len());
        }
    }
}

/// Separate class locks: a declaration may acquire its mesh dependencies while
/// it is decoded. Purge takes these in the same declaration-before-model order.
/// Cold acquisition stays serialized through publication, so purge cannot race
/// an older read and leave its payload pinned after the retirement boundary.
#[derive(Default)]
pub(crate) struct Owner {
    declarations: Mutex<Registry<ParticleDeclaration>>,
    models: Mutex<Registry<ParticleModel>>,
    // Derived CPU sampling data has no independent resource marker: its
    // declarations own it. The image/streaming owner still owns GPU images.
    fields: Mutex<HashMap<String, Weak<super::particle_field::FieldPixels>>>,
}
impl Owner {
    pub(crate) fn release(&self, catalog: u64, marker: u8) {
        self.declarations.lock().unwrap_or_else(|e| e.into_inner()).release(catalog, "particledt", marker);
        self.models.lock().unwrap_or_else(|e| e.into_inner()).release(catalog, "staticParticleModel", marker);
        self.fields.lock().unwrap_or_else(|e| e.into_inner()).retain(|_, value| value.strong_count() != 0);
    }
}
fn owner(catalog: &Catalog) -> &Owner { catalog.particle_resources.get_or_init(Owner::default) }
fn key(path: &str) -> String { path.replace('\\', "/").to_ascii_lowercase() }
fn share(catalog: &Catalog, shared: bool) -> bool {
    shared && !catalog.has_overlays()
        && std::env::var_os("DH2_DEBUG_SEPARATE_PARTICLE_RESOURCES").is_none_or(|value| value != "1")
}

pub(crate) fn register_preload(catalog: &Catalog, kind: &str, path: &str,
    marker: u8, persistent: bool) -> Result<(), String> {
    let owner = owner(catalog);
    if kind.eq_ignore_ascii_case("particledt") {
        owner.declarations.lock().map_err(|_| "particle declaration registry is poisoned")?.register(&key(path), marker, persistent);
    } else {
        owner.models.lock().map_err(|_| "particle model registry is poisoned")?.register(&key(path), marker, persistent);
    }
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    let Some(owner) = catalog.particle_resources.get() else { return Ok(()) };
    owner.declarations.lock().map_err(|_| "particle declaration registry is poisoned")?.promote_bootstrap();
    owner.models.lock().map_err(|_| "particle model registry is poisoned")?.promote_bootstrap();
    if std::env::var_os("DH2_DEBUG_LOAD_TIMING").is_some_and(|value| value == "1") {
        eprintln!("PARTICLE_RESOURCE_BOOTSTRAP catalog={} declarations={} models={}", catalog.identity(),
            owner.declarations.lock().map_err(|_| "particle declaration registry is poisoned")?.entries.len(),
            owner.models.lock().map_err(|_| "particle model registry is poisoned")?.entries.len());
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.particle_resources.get() { owner.release(catalog.identity(), marker); }
}
pub(crate) fn declaration(catalog: &Catalog, path: &str, shared: bool,
    load: impl FnOnce(u8) -> Result<Arc<ParticleDeclaration>, String>) -> Result<Arc<ParticleDeclaration>, String> {
    // Editable generations remain exclusively leased by their actual users;
    // neither their successful payload nor an earlier error is cached here.
    if !share(catalog, shared) { return load(0); }
    // FX descriptors store suffixless declaration names; preload manifests
    // carry archive paths. Both must address the exact same header/history key.
    let resource = format!("generated/decls/particledt/{}.decl", key(path));
    owner(catalog).declarations.lock().map_err(|_| "particle declaration registry is poisoned")?.acquire(&resource, 0, load)
}
pub(crate) fn model(catalog: &Catalog, path: &str, marker: u8, shared: bool,
    load: impl FnOnce() -> Result<Arc<ParticleModel>, String>) -> Result<Arc<ParticleModel>, String> {
    if !share(catalog, shared) { return load(); }
    owner(catalog).models.lock().map_err(|_| "particle model registry is poisoned")?.acquire(&key(path), marker, |_| load())
}
pub(crate) fn field(catalog: &Catalog, path: &str, shared: bool,
    load: impl FnOnce() -> Result<Arc<super::particle_field::FieldPixels>, String>)
    -> Result<Arc<super::particle_field::FieldPixels>, String> {
    if !share(catalog, shared) { return load(); }
    let mut fields = owner(catalog).fields.lock().map_err(|_| "particle field registry is poisoned")?;
    if let Some(value) = fields.get(path).and_then(Weak::upgrade) { return Ok(value); }
    let value = load()?;
    fields.insert(path.to_owned(), Arc::downgrade(&value));
    Ok(value)
}
