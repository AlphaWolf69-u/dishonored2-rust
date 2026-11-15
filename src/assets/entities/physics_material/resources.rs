//! Shared arkPhysicsMaterial records (141352520 / 14008b640).
//! Host typed values flatten inherited syntax while the catalog registry owns
//! declaration identity, selected preload markers and conditional purge.
use super::PhysicsMaterial;
use crate::archive::Catalog;
use std::sync::Arc;

pub(crate) struct Definition {
    material: PhysicsMaterial,
    contact_chain: Box<[String]>,
}
impl Definition {
    pub(crate) fn material(&self) -> &PhysicsMaterial { &self.material }
    pub(crate) fn contact_chain(&self) -> &[String] { &self.contact_chain }
    fn nested_bytes(&self) -> usize {
        self.material.name.capacity() + self.contact_chain.len() * std::mem::size_of::<String>()
            + self.contact_chain.iter().map(String::capacity).sum::<usize>()
    }
}
impl std::ops::Deref for Definition {
    type Target = PhysicsMaterial;
    fn deref(&self) -> &Self::Target { &self.material }
}
impl std::fmt::Debug for Definition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { std::fmt::Debug::fmt(&self.material, f) }
}

pub(crate) fn acquire_with(
    catalog: &Catalog,
    name: &str,
    read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Arc<Definition>, String> {
    let path = format!("generated/decls/physicsmaterial/{}.decl",
        name.replace('\\', "/").to_ascii_lowercase());
    let trace = std::env::var_os("DH2_DEBUG_PHYSICS_MATERIAL_RESOURCES").is_some_and(|value| value == "1");
    let shared = !catalog.has_overlays()
        && std::env::var_os("DH2_DEBUG_SEPARATE_PHYSICS_MATERIAL_RESOURCES").is_none_or(|value| value != "1");
    let start = trace.then(crate::loading::thread_cycles).flatten();
    let mut decoded = false;
    let value = catalog.physics_material_resources.get_or_init(Default::default).lock()
        .map_err(|_| "physics material registry is poisoned")?
        .acquire_with_sharing(&path, 0, shared, |_| {
            decoded = true;
            let mut read = read;
            let mut chain = Vec::new();
            let material = super::physics_material(name, |path| {
                let text = read(path)?;
                if text.is_some() {
                    let name = path.strip_prefix("generated/decls/physicsmaterial/")
                        .and_then(|path| path.strip_suffix(".decl"))
                        .ok_or("invalid inherited physics material path")?;
                    chain.push(name.to_owned());
                }
                Ok(text)
            })?;
            Ok(Arc::new(Definition { material, contact_chain: chain.into_boxed_slice() }))
        });
    if trace {
        let cycles = start.zip(crate::loading::thread_cycles())
            .map_or(0, |(start, end)| end.saturating_sub(start));
        let mut signature = blake3::Hasher::new();
        if let Ok(value) = &value { signature.update(format!("{value:?}").as_bytes()); }
        eprintln!("PHYSICS_MATERIAL_ACQUIRE catalog={} path={path} shared={shared} decoded={decoded} cycles={cycles} success={} signature={}",
            catalog.identity(), value.is_ok(), signature.finalize());
    }
    value
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8, persistent: bool) -> Result<(), String> {
    catalog.physics_material_resources.get_or_init(Default::default).lock()
        .map_err(|_| "physics material registry is poisoned")?
        .register(path, marker, persistent);
    Ok(())
}

pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(registry) = catalog.physics_material_resources.get() {
        registry.lock().map_err(|_| "physics material registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}

pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(registry) = catalog.physics_material_resources.get() {
        let mut registry = registry.lock().unwrap_or_else(|error| error.into_inner());
        registry.release(catalog.identity(), "physicsMaterial", marker);
        registry.report(catalog.identity(), "physicsMaterial", "scope-released", Definition::nested_bytes);
    }
}

pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(registry) = catalog.physics_material_resources.get() {
        registry.lock().unwrap_or_else(|error| error.into_inner())
            .report(catalog.identity(), "physicsMaterial", "catalog-retiring", Definition::nested_bytes);
    }
}

/// Observe real installed declaration leases independently of physics bodies.
#[cfg(feature = "game")]
pub(crate) fn observe_resource_scopes(base: &std::path::Path, map: &str) -> Result<(), String> {
    let catalog = Catalog::open(base).map_err(|error| error.to_string())?;
    let level = crate::assets::level::LevelResources::resolve(&catalog, map).map_err(|error| error.to_string())?;
    let map = level.entities.trim_start_matches("maps/").trim_end_matches(".entities");
    let language = crate::assets::lang::read_text_language(base)?;
    let list = crate::assets::map_resources::acquire_level_list(base, &catalog, map, language)?;
    list.register_preload_headers(&catalog, map, false)?;
    let name = |path: &str| path.strip_prefix("generated/decls/physicsmaterial/")
        .and_then(|path| path.strip_suffix(".decl")).map(str::to_owned);
    let read = |path: &str| catalog.read(path).map(|bytes| bytes.map(crate::entities::owned_declaration_text))
        .map_err(|error| error.to_string());
    let common_path = list.sections().take(2).flatten().find(|entry| entry.kind == "physicsMaterial")
        .ok_or("common physics material is absent")?.path.clone();
    let common_name = name(&common_path).ok_or("common physics material path is invalid")?;
    let common = acquire_with(&catalog, &common_name, read)?;
    let reference = super::physics_material(&common_name, read)?;
    if common.material() != &reference { return Err("typed physics material differs from its installed source".into()); }
    let common_weak = Arc::downgrade(&common);
    drop(common);

    let common_paths = list.sections().take(2).flatten().filter(|entry| entry.kind == "physicsMaterial")
        .map(|entry| entry.path.as_str()).collect::<std::collections::HashSet<_>>();
    let map_path = list.sections().nth(2).into_iter().flatten()
        .find(|entry| entry.kind == "physicsMaterial" && !common_paths.contains(entry.path.as_str()))
        .ok_or("map physics material outside common is absent")?.path.clone();
    let map_name = name(&map_path).ok_or("map physics material path is invalid")?;
    let material = acquire_with(&catalog, &map_name, read)?;
    let map_weak = Arc::downgrade(&material);
    catalog.release_level_resources();
    if !crate::loading::catalog_read_recorded(catalog.identity(), "physicsMaterial", &map_path) {
        return Err("a live map material lost its read history".into());
    }
    drop(material);
    catalog.release_level_resources();
    if map_weak.strong_count() != 0
        || crate::loading::catalog_read_recorded(catalog.identity(), "physicsMaterial", &map_path) {
        return Err("map material survived its final scope".into());
    }
    let again = acquire_with(&catalog, &common_name, read)?;
    if !std::ptr::addr_eq(common_weak.as_ptr(), Arc::as_ptr(&again)) {
        return Err("map purge replaced the bootstrap material".into());
    }
    // Exercise the promotion class with an actual installed map declaration.
    register_preload(&catalog, &map_path, 0, true)?;
    let persistent = acquire_with(&catalog, &map_name, read)?;
    drop(persistent);
    catalog.release_level_resources();
    let persistent_weak = {
        let value = acquire_with(&catalog, &map_name, read)?;
        Arc::downgrade(&value)
    };
    if persistent_weak.strong_count() != 1 { return Err("persistent material was not retained".into()); }
    catalog.release_persistent_level_resources();
    if persistent_weak.strong_count() != 0 {
        return Err("persistent material survived its conditional purge".into());
    }
    drop(list);
    drop(catalog);
    if common_weak.strong_count() != 1 {
        return Err("catalog release retained its bootstrap material lease".into());
    }
    drop(again);
    if common_weak.strong_count() != 0 { return Err("material survived its final consumer".into()); }
    eprintln!("PHYSICS_MATERIAL_SCOPES source=true map_retired=true bootstrap_reused=true persistent_retired=true session_retired=true");
    Ok(())
}

#[cfg(feature = "game")]
pub(crate) fn observe_inventory(base: &std::path::Path) -> Result<(), String> {
    let catalog = Catalog::open(base).map_err(|error| error.to_string())?;
    let read = |path: &str| catalog.read(path).map(|bytes| bytes.map(crate::entities::owned_declaration_text))
        .map_err(|error| error.to_string());
    let mut count = 0;
    let mut ancestry_names = 0;
    let mut signature = blake3::Hasher::new();
    for path in catalog.paths_with_suffix(".decl") {
        if catalog.kind(&path) != Some("physicsMaterial") { continue; }
        let name = path.strip_prefix("generated/decls/physicsmaterial/")
            .and_then(|path| path.strip_suffix(".decl")).ok_or("invalid physics material path")?;
        let chain = crate::entities::contacts::material_chain(name, read)?;
        let reference = super::physics_material(name, read)?;
        let definition = acquire_with(&catalog, name, read)?;
        if definition.material() != &reference || definition.contact_chain() != chain.as_slice() {
            return Err(format!("physics material or contact ancestry differs for {path}"));
        }
        signature.update(path.as_bytes());
        signature.update(format!("{reference:?}{chain:?}").as_bytes());
        count += 1;
        ancestry_names += chain.len();
    }
    if count == 0 { return Err("no installed physics materials were acquired".into()); }
    eprintln!("PHYSICS_MATERIAL_INVENTORY materials={count} ancestry_names={ancestry_names} matched=true signature={}", signature.finalize());
    Ok(())
}
