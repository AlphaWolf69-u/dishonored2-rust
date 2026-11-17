//! Immutable arkDeclSoundEvent identities (141352070, 1400b3750/1400b37b0).
//! Header admission, bootstrap promotion and purge use the shared declaration
//! rules 1400607f0/14005feb0/1400661e0/140060b60/140060180. Voice/media state
//! remains owned by the Wwise playback and streaming modules.
use std::sync::Arc;
use crate::archive::Catalog;
use super::SoundEventDeclaration;

pub fn acquire_with(catalog: &Catalog, name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Arc<SoundEventDeclaration>, String> {
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let acquisition_start = if trace { crate::loading::thread_cycles() } else { None };
    let name = name.replace('\\', "/").to_ascii_lowercase();
    let path = format!("generated/decls/soundevent/{name}.decl");
    let shared = !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_SOUND_EVENTS").as_deref() != Ok("1");
    let mut decoded = false;
    let mut cycles = 0;
    let mut load = |_| {
        decoded = true;
        let start = if trace { crate::loading::thread_cycles() } else { None };
        let _tag = crate::platform::allocation_tags::tag("Sound event declarations");
        let value = read(&path).and_then(|source| match source {
            Some(source) => {
                //Retain this read only until the ordinary inheritance reader
                //consumes it, rather than reading the declaration twice.
                let mut source = Some(source);
                crate::assets::entities::player::resolved_decl("soundevent", &name, |requested| {
                    if requested == path {
                        if let Some(source) = source.take() { return Ok(Some(source)); }
                    }
                    read(requested)
                })
            }
            //0607f0→05ede0: a missing named resource is reconstructed with
            //its real constructor and default declaration, not a fake bank
            //event.011fc0 reads022c70's exact `{ }\n` default, then invokes
            //0b3880. Keep this generation in the same resource registry.
            None => Ok(Vec::new()),
        })
            .and_then(|fields| SoundEventDeclaration::from_fields(&fields)
                .map_err(|error| format!("{name}: {error}")))
            .and_then(|mut event| {
                //0b3880 derives bit0 from the resource name after parsing;
                //the reflected field cannot override that finalizer.
                event.game_defined = name.contains("game_defined/");
                if let Some(name)=event.game_properties.as_deref(){
                    event.game_descriptor=Some(super::game_properties::acquire_with(catalog,name,&mut read)?);
                }
                Ok(Arc::new(event))
            });
        if let (Some(start), Some(end)) = (start, if trace { crate::loading::thread_cycles() } else { None }) {
            cycles = end.saturating_sub(start);
        }
        value
    };
    // Publication is serialized with decoding, so concurrent consumers cannot
    // prepare duplicate generations or overwrite a recorded immutable failure.
    let value = catalog.sound_events.get_or_init(Default::default).lock()
        .map_err(|_| "sound event registry is poisoned")?.acquire_with_sharing(&path, 0, shared, load);
    let acquisition_cycles = acquisition_start.zip(if trace { crate::loading::thread_cycles() } else { None })
        .map_or(0, |(start, end)| end.saturating_sub(start));
    if trace {
        let mut crc = flate2::Crc::new();
        if let Ok(value) = &value { crc.update(format!("{value:?}").as_bytes()); }
        eprintln!("SOUND_EVENT_ACQUIRE path={path} decoded={decoded} shared={shared} cycles={cycles} acquisition_cycles={acquisition_cycles} signature={:08x} success={}", crc.sum(), value.is_ok());
    }
    value
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8, persistent: bool) -> Result<(), String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    catalog.sound_events.get_or_init(Default::default).lock()
        .map_err(|_| "sound event registry is poisoned")?.register(&path, marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.sound_events.get() {
        owner.lock().map_err(|_| "sound event registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.sound_events.get() {
        let mut registry = owner.lock().unwrap_or_else(|error| error.into_inner());
        registry.release(catalog.identity(), "soundevent", marker);
        registry.report(catalog.identity(), "soundevent", "scope-released", |event| event.game_properties.as_ref().map_or(0, String::capacity));
    }
}

pub(crate) fn report_retirement(catalog: &Catalog) {
    if let Some(owner) = catalog.sound_events.get() {
        owner.lock().unwrap_or_else(|error| error.into_inner())
            .report(catalog.identity(), "soundevent", "catalog-retiring", |event| event.game_properties.as_ref().map_or(0, String::capacity));
    }
}
