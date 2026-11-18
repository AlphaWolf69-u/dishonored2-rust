//! arkMassPropertiesResource registration 141356de0, construction 140089390,
//! compiled input 140089690, source lookup 14008c7f0 and release 140089320.
//! Host: retain the already decoded principal frame instead of the Havok blob;
//! the existing decoder and body-specific override math remain unchanged.
use std::sync::Arc;
use crate::archive::Catalog;
use super::PrincipalMassProperties;

pub(super) fn acquire(catalog: &Catalog, path: &str) -> Result<Arc<Option<PrincipalMassProperties>>, String> {
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let start = if trace { crate::loading::thread_cycles() } else { None };
    let key = path.replace('\\', "/").to_ascii_lowercase();
    let shared = !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_MASS_RESOURCES").as_deref() != Ok("1");
    let mut decoded = false;
    let mut load = |_| {
        crate::gameplay::physics_world::thread_policy::enter();
        decoded = true;
        let _tag = crate::platform::allocation_tags::tag("Mass property resources");
        // Absence is a recorded immutable resource state. Preserve it separately
        // from archive/format failures so callers keep the actual missing path.
        let value = catalog.read(path).map_err(|error| format!("{path}: {error}"))?
            .map(|bytes| super::parse_principal(&bytes)).transpose()?;
        Ok(Arc::new(value))
    };
    let value = catalog.mass_resources.get_or_init(Default::default).lock()
        .map_err(|_| "mass resource registry is poisoned")?.acquire_with_sharing(&key, 0, shared, load);
    if trace {
        let cycles = start.zip(crate::loading::thread_cycles()).map_or(0, |(start, end)| end.saturating_sub(start));
        let mut crc = flate2::Crc::new();
        if let Ok(value) = &value { crc.update(format!("{value:?}").as_bytes()); }
        eprintln!("MASS_RESOURCE_ACQUIRE path={key} decoded={decoded} shared={shared} cycles={cycles} signature={:08x} success={}", crc.sum(), value.is_ok());
    }
    value
}
pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8, persistent: bool) -> Result<(), String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    catalog.mass_resources.get_or_init(Default::default).lock()
        .map_err(|_| "mass resource registry is poisoned")?.register(&path, marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.mass_resources.get() {
        owner.lock().map_err(|_| "mass resource registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.mass_resources.get() {
        let mut registry = owner.lock().unwrap_or_else(|error| error.into_inner());
        registry.release(catalog.identity(), "massprops", marker);
        registry.report(catalog.identity(), "massprops", "scope-released", |_| 0);
    }
}

pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.mass_resources.get() {
        owner.lock().unwrap_or_else(|error| error.into_inner())
            .report(catalog.identity(), "massprops", "catalog-retiring", |_| 0);
    }
}
