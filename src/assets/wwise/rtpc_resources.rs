//! Immutable arkDeclSoundRTPC identities (1413520e0, 1400b4510/1400b4560).
//! Header admission, bootstrap promotion and purge use the shared declaration
//! rules 1400607f0/14005feb0/1400661e0/140060b60/140060180. Voice/media state
//! remains owned by the Wwise playback and streaming modules.
use std::sync::Arc;
use crate::archive::Catalog;

pub fn acquire_with(catalog: &Catalog, name: &str,
    read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Arc<super::RtpcDeclaration>, String> {
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let acquisition_start = if trace { crate::loading::thread_cycles() } else { None };
    let path = format!("generated/decls/soundrtpc/{}.decl", name.replace('\\', "/").to_ascii_lowercase());
    let shared = !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_SOUND_RTPCS").as_deref() != Ok("1");
    let mut decoded = false;
    let mut cycles = 0;
    let mut load = |_| {
        decoded = true;
        let start = if trace { crate::loading::thread_cycles() } else { None };
        let _tag = crate::platform::allocation_tags::tag("Sound RTPC declarations");
        let value = crate::assets::entities::player::resolved_decl("soundrtpc", name, read)
            .and_then(|fields| super::RtpcDeclaration::from_fields(&fields)
                .map(Arc::new).map_err(|error| format!("{name}: {error}")));
        if let (Some(start), Some(end)) = (start, if trace { crate::loading::thread_cycles() } else { None }) {
            cycles = end.saturating_sub(start);
        }
        value
    };
    // Publication is serialized with decoding, so concurrent consumers cannot
    // prepare duplicate generations or overwrite a recorded immutable failure.
    let value = if shared {
        catalog.sound_rtpcs.get_or_init(Default::default).lock()
            .map_err(|_| "sound RTPC registry is poisoned")?.acquire(&path, 0, load)
    } else { load(0) };
    let acquisition_cycles = acquisition_start.zip(if trace { crate::loading::thread_cycles() } else { None })
        .map_or(0, |(start, end)| end.saturating_sub(start));
    if trace {
        let mut crc = flate2::Crc::new();
        if let Ok(value) = &value { crc.update(format!("{value:?}").as_bytes()); }
        eprintln!("SOUND_RTPC_ACQUIRE path={path} decoded={decoded} shared={shared} cycles={cycles} acquisition_cycles={acquisition_cycles} signature={:08x} success={}", crc.sum(), value.is_ok());
    }
    value
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8, persistent: bool) -> Result<(), String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    catalog.sound_rtpcs.get_or_init(Default::default).lock()
        .map_err(|_| "sound RTPC registry is poisoned")?.register(&path, marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.sound_rtpcs.get() {
        owner.lock().map_err(|_| "sound RTPC registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.sound_rtpcs.get() {
        let mut registry = owner.lock().unwrap_or_else(|error| error.into_inner());
        registry.release(catalog.identity(), "soundrtpc", marker);
        registry.report(catalog.identity(), "soundrtpc", "scope-released", |_| 0);
    }
}

pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.sound_rtpcs.get() {
        owner.lock().unwrap_or_else(|error| error.into_inner())
            .report(catalog.identity(), "soundrtpc", "catalog-retiring", |_| 0);
    }
}
