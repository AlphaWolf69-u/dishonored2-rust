//! Catalog identities for immutable particle payloads (1400607f0/14005feb0).
//! 140060b60 promotes bootstrap headers; 1400661e0 explicitly marks selected
//! map headers; 140060180 releases only the requested marker. Payload decoding
//! stays demand driven, a host deviation from eager preload deserialization.
use crate::{archive::Catalog, assets::{fx::particle_declaration::ParticleDeclaration, particle_model::ParticleModel}};
use std::{collections::HashMap, sync::{Arc, Mutex, Weak}};
use crate::assets::resource_registry::Registry;

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
            owner.declarations.lock().map_err(|_| "particle declaration registry is poisoned")?.len(),
            owner.models.lock().map_err(|_| "particle model registry is poisoned")?.len());
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.particle_resources.get() { owner.release(catalog.identity(), marker); }
}
pub(crate) fn declaration(catalog: &Catalog, path: &str, shared: bool, cancelled: Option<&dyn Fn() -> bool>,
    load: impl FnOnce(u8) -> Result<Arc<ParticleDeclaration>, String>) -> Result<Arc<ParticleDeclaration>, String> {
    // FX descriptors store suffixless declaration names; preload manifests
    // carry archive paths. Both must address the exact same header/history key.
    let resource = format!("generated/decls/particledt/{}.decl", key(path));
    let mut declarations = owner(catalog).declarations.lock().map_err(|_| "particle declaration registry is poisoned")?;
    if share(catalog, shared) {
        declarations.acquire_when(&resource, 0, load, || !cancelled.is_some_and(|stop| stop()))
    } else {
        declarations.acquire_with_sharing(&resource, 0, false, load)
    }
}
pub(crate) fn model(catalog: &Catalog, path: &str, marker: u8, shared: bool, cancelled: Option<&dyn Fn() -> bool>,
    load: impl FnOnce() -> Result<Arc<ParticleModel>, String>) -> Result<Arc<ParticleModel>, String> {
    let resource = key(path);
    let mut models = owner(catalog).models.lock().map_err(|_| "particle model registry is poisoned")?;
    if share(catalog, shared) {
        models.acquire_when(&resource, marker, |_| load(), || !cancelled.is_some_and(|stop| stop()))
    } else {
        models.acquire_with_sharing(&resource, marker, false, |_| load())
    }
}
pub(crate) fn field(catalog: &Catalog, path: &str, shared: bool, cancelled: Option<&dyn Fn() -> bool>,
    load: impl FnOnce() -> Result<Arc<super::particle_field::FieldPixels>, String>)
    -> Result<Arc<super::particle_field::FieldPixels>, String> {
    if !share(catalog, shared) { return load(); }
    let mut fields = owner(catalog).fields.lock().map_err(|_| "particle field registry is poisoned")?;
    if let Some(value) = fields.get(path).and_then(Weak::upgrade) { return Ok(value); }
    let value = load()?;
    if !cancelled.is_some_and(|stop| stop()) {
        fields.insert(path.to_owned(), Arc::downgrade(&value));
    }
    Ok(value)
}
