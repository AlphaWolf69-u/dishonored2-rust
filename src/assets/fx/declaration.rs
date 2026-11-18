//! FX declaration acquisition, including the parent's authored values.
//! 140022fd0 acquires the parent with 1400607f0 before applying child edits;
//! idDeclFX registers at 1413630d0 and releases backing at 1401c5990.
//! Resolved syntax stays temporary: callers retain only their typed FX data.
use crate::{archive::Catalog, assets::entities::{self, Value}};
use std::collections::HashSet;

type Fields = Vec<(String, Value)>;
pub(crate) fn read_with(name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Fields, String> {
    let _tag = crate::platform::allocation_tags::tag("FX declaration sources");
    let (first, parent) = layer(name, &mut read)?;
    // Ordinary definitions retain the former move-only edit-block path.
    let Some(parent) = parent else { return Ok(first) };
    let mut layers = vec![first];
    let mut active = HashSet::from([key(name)]);
    let mut next = Some(parent);
    while let Some(name) = next {
        let path = key(&name);
        if !active.insert(path.clone()) {
            return Err(format!("FX declaration inheritance cycle at {path}"));
        }
        let (fields, parent) = layer(&name, &mut read)?;
        layers.push(fields);
        next = parent;
    }
    let mut fields = layers.pop().expect("FX inheritance has a base layer");
    while let Some(local) = layers.pop() { entities::overlay_owned(&mut fields, local); }
    Ok(fields)
}
fn key(name: &str) -> String {
    format!("generated/decls/fx/{name}.decl").replace('\\', "/").to_ascii_lowercase()
}
fn layer(name: &str, read: &mut impl FnMut(&str) -> Result<Option<String>, String>)
    -> Result<(Fields, Option<String>), String> {
    let path = key(name);
    let source = read(&path)?.ok_or_else(|| format!("FX declaration missing: {path}"))?;
    let mut fields = entities::parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
    // 140143bb0 is the native length-zero check; an empty name has no parent.
    let parent = entities::event_graph::atom(&fields, "inherit").filter(|name| !name.is_empty()).map(str::to_owned);
    let edit = if let Some(index) = fields.iter().position(|(name, _)| name == "edit") {
        match fields.swap_remove(index).1 {
            Value::Scope(edit) => edit,
            _ => return Err(format!("{path}: edit is not a scope")),
        }
    } else { Vec::new() };
    Ok((edit, parent))
}

/// The original declaration identity owns typed descriptor data only. Render
/// materials/images and mutable effect clocks remain with their consumers.
pub(crate) fn acquire(catalog: &Catalog, name: &str, consumer: super::definition::Consumer, cancelled: Option<&dyn Fn() -> bool>)
    -> Result<std::sync::Arc<super::definition::Definition>, String> {
    acquire_with(catalog, name, consumer, cancelled, |path|
        crate::assets::declaration_texts::read_common_or_catalog(catalog, path))
}
pub(crate) fn acquire_with(catalog: &Catalog, name: &str, consumer: super::definition::Consumer, cancelled: Option<&dyn Fn() -> bool>,
    read: impl FnMut(&str) -> Result<Option<String>, String>)
    -> Result<std::sync::Arc<super::definition::Definition>, String> {
    let _tag = crate::platform::allocation_tags::tag("FX declaration resources");
    if cancelled.is_some_and(|stop| stop()) { return Err("effect acquisition cancelled".into()); }
    let resource = key(name);
    let mut owner = catalog.fx_resources.get_or_init(Default::default).lock()
        .map_err(|_| "FX declaration registry is poisoned")?;
    let trace = std::env::var_os("DH2_DEBUG_FX_DECLARATIONS").is_some_and(|value| value == "1");
    let started = trace.then(crate::loading::thread_cycles).flatten();
    let mut decoded = false;
    let mut extended = false;
    // The registry invokes these callbacks sequentially. Borrow one reader
    // across cold loading and warm preparation without a second source owner.
    let read = std::cell::RefCell::new(read);
    let load = |_| {
        decoded = true;
        let mut sources = Vec::new();
        let fields = read_with(name, |path| {
            let value = (read.borrow_mut())(path)?;
            if value.is_some() { sources.push(path.to_owned()); }
            Ok(value)
        })?;
        if cancelled.is_some_and(|stop| stop()) { return Err("effect acquisition cancelled".into()); }
        let mut definition = super::definition::Definition::from_fields(&fields, consumer, cancelled)?;
        // The host flattens inherited edit syntax into typed descriptors. Keep
        // source read credits with this generation, never a second syntax tree.
        definition.source_paths = sources;
        Ok(std::sync::Arc::new(definition))
    };
    let shared = !catalog.has_overlays()
        && std::env::var_os("DH2_DEBUG_SEPARATE_FX_DECLARATIONS").is_none_or(|value| value != "1");
    let prepare = |definition: &std::sync::Arc<super::definition::Definition>| {
        if definition.needs(consumer) {
            extended = true;
            let fields = read_with(name, |path| (read.borrow_mut())(path))?;
            definition.prepare(&fields, consumer, cancelled)?;
        }
        Ok(())
    };
    let definition = if shared {
        owner.acquire_prepared_when(&resource, 0, load, prepare,
            || !cancelled.is_some_and(|stop| stop()))
    } else {
        owner.acquire_with_sharing(&resource, 0, false, load)
    }?;
    if cancelled.is_some_and(|stop| stop()) { return Err("effect acquisition cancelled".into()); }
    if trace {
        let cycles = started.zip(crate::loading::thread_cycles()).map_or(0, |(a, b)| b.saturating_sub(a));
        eprintln!("FX_DECLARATION_ACQUIRE path={resource} decoded={decoded} extended={extended} shared={shared} groups={} sources={} cycles={cycles}",
            definition.groups.len(), definition.source_paths.len());
    }
    Ok(definition)
}
/// Called with the class owner already locked, following acquisition's owner
/// -> read-history lock order. Inherited source credit belongs to live typed
/// generations, even when the parent itself was never directly requested.
pub(crate) fn source_history(owner: &crate::assets::resource_registry::Registry<super::definition::Definition>) -> HashSet<String> {
    let mut paths = HashSet::new();
    for definition in owner.live_values() { paths.extend(definition.source_paths.iter().cloned()); }
    paths
}
pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8, persistent: bool) -> Result<(), String> {
    let _tag = crate::platform::allocation_tags::tag("FX declaration resources");
    catalog.fx_resources.get_or_init(Default::default).lock()
        .map_err(|_| "FX declaration registry is poisoned")?.register(path, marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.environment_resources.get() {
        owner.lock().map_err(|_| "environment resource registry is poisoned")?.promote_bootstrap();
    }
    if let Some(owner) = catalog.fx_resources.get() {
        owner.lock().map_err(|_| "FX declaration registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    crate::rendering::environment_resources::release(catalog, marker);
    if let Some(owner) = catalog.fx_resources.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "fx", marker);
        // Generic retirement removes direct identities; flattening inheritance
        // also requires dropping orphaned parent-source credits.
        let dependencies = source_history(&owner);
        crate::loading::retain_catalog_kind(catalog.identity(), "fx", |path|
            owner.owns_read_history(path) || dependencies.contains(path));
        if std::env::var_os("DH2_DEBUG_FX_DECLARATIONS").is_some_and(|value| value == "1") {
            eprintln!("FX_DECLARATION_PURGE catalog={} marker={marker} records={} live={} sources={}",
                catalog.identity(), owner.len(), owner.live_values().count(), dependencies.len());
        }
    }
}
