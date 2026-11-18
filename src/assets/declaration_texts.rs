//! Shared declaration-text acquisition for physics, contact FX and sound.
//! Session init/common and map-only snapshots retain distinct lifetimes.
use crate::assets::archive::Catalog;
use std::{borrow::Cow, collections::HashMap, sync::Arc};

// Archive source buffers reserve an extra byte for oversized-stream detection.
// The native lexer rules remain in entities::declaration_text (140135f70).
static HANDOFF_MODE: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
static HANDOFF_STATS: [std::sync::atomic::AtomicU64; 6] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 6];
fn owned_text(bytes: Vec<u8>) -> String {
    let (copy, trace) = *HANDOFF_MODE.get_or_init(|| (
        std::env::var("DH2_DEBUG_COPY_CONTACT_TEXT").as_deref() == Ok("1"),
        std::env::var("DH2_DEBUG_CONTACT_HANDOFF").as_deref() == Ok("1"),
    ));
    let started = trace.then(crate::loading::thread_cycles).flatten();
    let input_len = bytes.len();
    let input_pointer = bytes.as_ptr();
    let text = if copy {
        crate::entities::declaration_text(&bytes).into_owned()
    } else {
        // Keep the archive reader's existing one-byte spare reservation.
        // Shrinking it moves the allocation on the measured Windows heap.
        crate::entities::owned_declaration_text(bytes)
    };
    if trace {
        let direct = !copy && text.as_ptr() == input_pointer;
        let cycles = started.zip(crate::loading::thread_cycles())
            .map_or(0, |(start, end)| end.saturating_sub(start));
        for (counter, value) in HANDOFF_STATS.iter().zip([
            1, input_len as u64, u64::from(direct),
            u64::from(direct), text.capacity() as u64, cycles,
        ]) { counter.fetch_add(value, std::sync::atomic::Ordering::Relaxed); }
    }
    text
}
pub(crate) fn report_text_handoff(phase: &str) {
    if HANDOFF_MODE.get().is_none_or(|&(_, trace)| !trace) { return; }
    let [calls, bytes, direct, same_buffer, retained_capacity, cycles] =
        std::array::from_fn(|index| HANDOFF_STATS[index].load(std::sync::atomic::Ordering::Relaxed));
    eprintln!("DECLARATION_TEXT_HANDOFF phase={phase} calls={calls} source_bytes={bytes} direct={direct} same_buffer={same_buffer} retained_capacity={retained_capacity} cycles={cycles}");
}

/// Preserve the text-only readers' UTF-8 behavior without running the
/// signed-byte declaration lexer normalization on their archive buffers.
pub(crate) fn adopt_utf8(bytes: Vec<u8>, owner: &str) -> String {
    static MODE: std::sync::OnceLock<(bool, bool)> = std::sync::OnceLock::new();
    let (copy, trace) = *MODE.get_or_init(|| (
        std::env::var_os("DH2_DEBUG_COPY_RAW_DECLARATION_TEXT").is_some_and(|value| value == "1"),
        std::env::var_os("DH2_DEBUG_RAW_DECLARATION_TEXT").is_some_and(|value| value == "1"),
    ));
    let start = trace.then(crate::loading::thread_cycles).flatten();
    let length = bytes.len();
    let original = bytes.as_ptr();
    let text = if copy {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        crate::entities::decode_utf8_lossy(bytes)
    };
    if trace {
        let cycles = start.zip(crate::loading::thread_cycles())
            .map_or(0, |(a, b)| b.saturating_sub(a));
        let adopted = length != 0 && text.as_ptr() == original;
        eprintln!("RAW_DECLARATION_TEXT owner={owner} bytes={length} adopted={adopted} cycles={cycles} signature={}",
            blake3::hash(text.as_bytes()));
    }
    text
}

pub(crate) type TextMap = HashMap<String, String>;
#[derive(Default)]
pub(crate) struct DeclarationTexts {
    common: Option<Arc<TextMap>>,
    level: TextMap,
}
#[derive(Default)]
struct TextLoads {
    reads: usize,
    bytes: usize,
}
pub(crate) struct SharedTexts {
    language: String,
    // None means that a mutable overlay requires the existing per-level scan.
    common: Option<Arc<TextMap>>,
}
#[derive(Clone, Copy)]
pub(crate) enum PhysicsUse { World, Model, Machinery, Runtime }

#[derive(Default)]
struct PhysicsStats {
    requests: usize,
    reads: usize,
    bytes: usize,
    common_hits: usize,
    local_hits: usize,
    cycle_reads: usize,
    cycles: u64,
}

/// Temporary physics source ownership until the contact table takes the
/// listed declarations. 1400607f0 shares each acquired resource identity.
#[derive(Default)]
pub(crate) struct PhysicsSeed {
    enabled: bool,
    common: Option<Arc<TextMap>>,
    local: TextMap,
    stats: [PhysicsStats; 4],
    cached_bytes: usize,
    peak_cached_bytes: usize,
    transfers: usize,
}

impl PhysicsSeed {
    pub(crate) fn new(catalog: &Catalog, language: &str) -> Self {
        let enabled = !catalog.has_overlays()
            && std::env::var("DH2_DEBUG_SEPARATE_PHYSICS_TEXTS").as_deref() != Ok("1")
            && std::env::var("DH2_DEBUG_SEPARATE_CONTACT_TEXTS").as_deref() != Ok("1");
        let common = if enabled {
            catalog.common_declaration_texts.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
                .filter(|entry| entry.language == language)
                .and_then(|entry| entry.common.clone())
        } else { None };
        Self { enabled, common, ..Default::default() }
    }

    pub(crate) fn read(&mut self, catalog: &Catalog, path: &str, role: PhysicsUse) -> Result<Option<String>, String> {
        static CPU_TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let trace = *CPU_TRACE.get_or_init(|| std::env::var("DH2_DEBUG_PHYSICS_CPU").as_deref() == Ok("1"));
        let start = if trace { crate::loading::thread_cycles() } else { None };
        let result = self.read_inner(catalog, path, role);
        if let (Some(start), Some(end)) = (start, if trace { crate::loading::thread_cycles() } else { None }) {
            let stats = &mut self.stats[role as usize];
            stats.cycle_reads += 1;
            stats.cycles = stats.cycles.saturating_add(end.saturating_sub(start));
        }
        result
    }

    fn read_inner(&mut self, catalog: &Catalog, path: &str, role: PhysicsUse) -> Result<Option<String>, String> {
        let stats = &mut self.stats[role as usize];
        stats.requests += 1;
        let key: Cow<'_, str> = if path.bytes().all(|byte| byte != b'\\' && !byte.is_ascii_uppercase()) {
            Cow::Borrowed(path)
        } else {
            Cow::Owned(path.replace('\\', "/").to_ascii_lowercase())
        };
        if self.enabled {
            if let Some(value) = self.common.as_deref().and_then(|texts| texts.get(key.as_ref())) {
                stats.common_hits += 1;
                return Ok(Some(value.clone()));
            }
            if let Some(value) = self.local.get(key.as_ref()) {
                stats.local_hits += 1;
                return Ok(Some(value.clone()));
            }
        }
        stats.reads += 1;
        let bytes = catalog.read(path).map_err(|error| error.to_string())?;
        stats.bytes += bytes.as_ref().map_or(0, Vec::len);
        let text = bytes.map(|bytes| owned_text(bytes));
        if self.enabled {
            if let Some(value) = &text {
                let key = key.into_owned();
                let retained = value.clone();
                self.cached_bytes += key.capacity() + retained.capacity();
                self.peak_cached_bytes = self.peak_cached_bytes.max(self.cached_bytes);
                self.local.insert(key, retained);
            }
        }
        Ok(text)
    }

    fn take(&mut self, path: &str) -> Option<String> {
        let (key, value) = self.local.remove_entry(path)?;
        self.cached_bytes = self.cached_bytes.saturating_sub(key.capacity() + value.capacity());
        self.transfers += 1;
        Some(value)
    }

    fn report(&self, map: &str) {
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1")
            && std::env::var("DH2_DEBUG_PHYSICS_CPU").as_deref() != Ok("1") { return; }
        for (role, stats) in ["world", "model", "machinery", "runtime"].into_iter().zip(&self.stats) {
            eprintln!("MEMORY physics text source: map={map} role={role} enabled={} requests={} reads={} decoded_bytes={} common_hits={} local_hits={}",
                self.enabled, stats.requests, stats.reads, stats.bytes, stats.common_hits, stats.local_hits);
            if stats.cycle_reads != 0 {
                eprintln!("MEMORY physics text CPU: map={map} role={role} calls={} cycles={}", stats.cycle_reads, stats.cycles);
            }
        }
        eprintln!("MEMORY physics text handoff: map={map} transfers={} remaining={} remaining_capacity_bytes={} peak_capacity_bytes={}",
            self.transfers, self.local.len(), self.cached_bytes, self.peak_cached_bytes);
    }
}

/// Later runtime consumers share session declarations when available.
pub(crate) fn read_common_or_catalog(catalog: &Catalog, path: &str) -> Result<Option<String>, String> {
    if !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_PHYSICS_TEXTS").as_deref() != Ok("1")
    {
        let owner = catalog.common_declaration_texts.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = owner.as_ref().and_then(|entry| entry.common.as_deref()).and_then(|texts| texts.get(path))
        {
            return Ok(Some(value.clone()));
        }
    }
    catalog.read(path)
        .map(|bytes| bytes.map(|bytes| owned_text(bytes)))
        .map_err(|error| error.to_string())
}

fn read_text(
    catalog: &Catalog,
    path: &str,
    stats: &mut TextLoads,
) -> Result<Option<String>, String> {
    stats.reads += 1;
    let bytes = catalog.read(path).map_err(|error| error.to_string())?;
    stats.bytes += bytes.as_ref().map_or(0, Vec::len);
    Ok(bytes.map(|bytes| owned_text(bytes)))
}

pub(crate) fn read(
    catalog: &Catalog,
    texts: &DeclarationTexts,
    path: &str,
) -> Result<Option<String>, String> {
    if let Some(value) = texts.get(path) {
        return Ok(Some(value.to_owned()));
    }
    catalog
        .read(path)
        .map(|bytes| bytes.map(|bytes| owned_text(bytes)))
        .map_err(|error| error.to_string())
}

impl Drop for DeclarationTexts {
    fn drop(&mut self) {
        if (!self.level.is_empty() || self.common.is_some())
            && std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1")
        {
            let bytes: usize = self
                .level
                .iter()
                .map(|(key, value)| key.capacity() + value.capacity())
                .sum();
            eprintln!("MEMORY declaration text release: level_entries={} level_string_capacity={bytes} common_entries={} common_owners={}",
                self.level.len(), self.common.as_ref().map_or(0, |common| common.len()),
                self.common.as_ref().map_or(0, Arc::strong_count));
        }
    }
}

impl DeclarationTexts {
    /// Called after world FX and audio owners stop. The separately retained
    /// common table remains available to subsequent levels.
    pub(crate) fn release_level_history(&self, catalog: &Catalog) {
        let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
        let started = trace.then(crate::loading::thread_cycles).flatten();
        let mut removed = 0usize;
        let mut bytes = 0usize;
        for kind in ["fx", "env", "soundevent", "physicsMaterial"] {
            // Typed event acquisition takes its registry before recording
            // a read. Hold it in that same order through this history sweep.
            let events = (kind == "soundevent").then(|| catalog.sound_events.get()).flatten()
                .map(|registry| registry.lock().unwrap_or_else(|error| error.into_inner()));
            let materials = (kind == "physicsMaterial").then(|| catalog.physics_material_resources.get()).flatten()
                .map(|registry| registry.lock().unwrap_or_else(|error| error.into_inner()));
            let effects = (kind == "fx").then(|| catalog.fx_resources.get()).flatten()
                .map(|registry| registry.lock().unwrap_or_else(|error| error.into_inner()));
            let fx_history = effects.as_ref().map(|registry| crate::assets::fx::declaration::source_history(registry));
            let environments = (kind == "env").then(|| catalog.environment_resources.get()).flatten()
                .map(|registry| registry.lock().unwrap_or_else(|error| error.into_inner()));
            let env_history = environments.as_ref().map(|registry| crate::rendering::environment_resources::source_history(registry));
            let (count, capacity, _, _) = crate::loading::retain_catalog_kind(catalog.identity(), kind, |path| {
                self.common.as_deref().is_some_and(|values| values.contains_key(path))
                    || effects.as_ref().is_some_and(|registry| registry.owns_read_history(path))
                    || fx_history.as_ref().is_some_and(|paths| paths.contains(path))
                    || environments.as_ref().is_some_and(|registry| registry.owns_read_history(path))
                    || env_history.as_ref().is_some_and(|paths| paths.contains(path))
                    || events.as_ref().is_some_and(|registry| registry.owns_read_history(path))
                    || materials.as_ref().is_some_and(|registry| registry.owns_read_history(path))
            });
            removed += count;
            bytes += capacity;
            if trace { eprintln!("MEMORY declaration history release: kind={kind} entries={count} path_capacity_bytes={capacity}"); }
        }
        if trace {
            let cycles = started.zip(crate::loading::thread_cycles()).map_or(0, |(start, end)| end.saturating_sub(start));
            eprintln!("MEMORY contact history release: entries={removed} path_capacity_bytes={bytes} cycles={cycles}");
        }
    }

    pub(crate) fn get(&self, path: &str) -> Option<&str> {
        let key = path.to_ascii_lowercase();
        self.common
            .as_deref()
            .and_then(|shared| shared.get(&key))
            .or_else(|| self.level.get(&key))
            .map(String::as_str)
    }
}

fn contact_resource(resource: &crate::assets::map_resources::Resource) -> bool {
    resource.kind == "physicsMaterial"
        || resource.kind == "soundevent"
        || (resource.kind == "fx" && resource.path.contains("/contactsystem/"))
}

/// 1400661e0 requests init/common before the map; 1400607f0 resolves their
/// existing identities. Share only immutable common text; active overlays keep
/// their previous per-level reads because their contents can change in place.
fn shared_texts(
    base: &std::path::Path,
    catalog: &Catalog,
    language: &str,
    stats: &mut TextLoads,
    seed: &mut PhysicsSeed,
) -> Result<Option<Arc<TextMap>>, String> {
    if std::env::var("DH2_DEBUG_SEPARATE_CONTACT_TEXTS").as_deref() == Ok("1") {
        return Ok(None);
    }
    let mut owner = catalog.common_declaration_texts.lock().unwrap_or_else(|e| e.into_inner());
    // Loose bootstrap manifests can change their membership during a session.
    if catalog.has_overlays()
        && (catalog.has_overlay_file("generated/buildgame/init.mapresources")
            || catalog.has_overlay_file("generated/buildgame/common.mapresources"))
    {
        crate::loading::set_common_texts(catalog.identity(), None);
        *owner = Some(SharedTexts {
            language: language.to_owned(),
            common: None,
        });
        return Ok(None);
    }
    if let Some(entry) = owner
        .as_ref()
        .filter(|entry| entry.language == language)
    {
        return Ok(entry.common.clone());
    }
    // A different language replaces this catalog's previous session table.
    crate::loading::set_common_texts(catalog.identity(), None);
    *owner = None;
    let lists = [
        crate::assets::map_resources::acquire_list(base, catalog, "init", language)?,
        crate::assets::map_resources::acquire_list(base, catalog, "common", language)?,
    ];
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") && !catalog.has_overlays() {
        let paths = lists
            .iter()
            .flat_map(|list| list.iter())
            .filter(|r| contact_resource(r))
            .count();
        eprintln!("MEMORY declaration overlay scan: skipped_bootstrap=2 skipped_common={paths}");
    }
    if catalog.has_overlays()
        && lists
            .iter()
            .flat_map(|list| list.iter())
            .filter(|r| contact_resource(r))
            .any(|r| catalog.has_overlay_file(&r.path))
    {
        crate::loading::set_common_texts(catalog.identity(), None);
        *owner = Some(SharedTexts {
            language: language.to_owned(),
            common: None,
        });
        return Ok(None);
    }
    let mut shared = TextMap::new();
    for resource in lists
        .iter()
        .flat_map(|list| list.iter())
        .filter(|r| contact_resource(r))
    {
        if !shared.contains_key(&resource.path) {
            let value = match seed.take(&resource.path) {
                Some(value) => Some(value),
                None => read_text(catalog, &resource.path, stats)?,
            };
            if let Some(value) = value {
                shared.insert(resource.path.clone(), value);
            }
        }
    }
    let shared = Arc::new(shared);
    *owner = Some(SharedTexts {
        language: language.to_owned(),
        common: Some(shared.clone()),
    });
    Ok(Some(shared))
}

/// Prepare the listed declarations before contacts are admitted. The shared
/// table has one session owner; the level table drops with ContactSounds.
pub(crate) fn acquire(
    base: &std::path::Path,
    catalog: &Catalog,
    map: &str,
    language: &str,
    mut seed: PhysicsSeed,
) -> Result<DeclarationTexts, String> {
    let map = map.trim_start_matches("maps/").trim_end_matches(".map");
    let list = crate::assets::map_resources::acquire_level_list(base, catalog, map, language)?;
    let mut stats = TextLoads::default();
    let common = shared_texts(base, catalog, language, &mut stats, &mut seed)?;
    crate::loading::set_common_texts(catalog.identity(), common.clone());
    let mut level = TextMap::new();
    for resource in list.iter().filter(|r| contact_resource(r)) {
        if level.contains_key(&resource.path)
            || common
                .as_deref()
                .is_some_and(|shared| shared.contains_key(&resource.path))
        {
            continue;
        }
        let value = match seed.take(&resource.path) {
            Some(value) => Some(value),
            None => read_text(catalog, &resource.path, &mut stats)?,
        };
        if let Some(value) = value {
            level.insert(resource.path.clone(), value);
        }
    }
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        let signature = if std::env::var("DH2_DEBUG_CONTACT_SIGNATURES").as_deref() == Ok("1") {
            let mut entries: Vec<_> = level
                .iter()
                .chain(common.iter().flat_map(|shared| shared.iter()))
                .collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
            let mut crc = flate2::Crc::new();
            for (path, value) in entries {
                crc.update(path.as_bytes());
                crc.update(&[0]);
                crc.update(value.as_bytes());
                crc.update(&[0]);
            }
            Some(crc.sum())
        } else {
            None
        };
        let bytes = |map: &TextMap| {
            map.iter()
                .map(|(k, v)| k.capacity() + v.capacity())
                .sum::<usize>()
        };
        eprintln!("MEMORY contact text acquisition: map={map} shared={} common={} level={} common_string_capacity={} level_string_capacity={} reads={} decoded_bytes={} signature={signature:?}",
            common.is_some(), common.as_ref().map_or(0, |shared| shared.len()), level.len(),
            common.as_deref().map_or(0, bytes), bytes(&level), stats.reads, stats.bytes);
    }
    seed.report(map);
    Ok(DeclarationTexts { common, level })
}

/// Observe the installed contact acquisition and final namespace release
/// across an interleaved pair of catalogs, without opening the client.
#[cfg(feature = "game")]
pub(crate) fn observe_catalog_scopes(base: &std::path::Path, map: &str) -> Result<(), String> {
    let a = Catalog::open(base).map_err(|error| error.to_string())?;
    let b = Catalog::open(base).map_err(|error| error.to_string())?;
    let level = crate::assets::level::LevelResources::resolve(&a, map).map_err(|error| error.to_string())?;
    let map = level.entities.trim_start_matches("maps/").trim_end_matches(".entities");
    let language = crate::assets::lang::read_text_language(base)?;
    let load = |catalog: &Catalog| acquire(base, catalog, map, language, PhysicsSeed::new(catalog, language));
    let first = load(&a)?;
    let other = load(&b)?;
    let again = load(&a)?;
    let a_common = first.common.as_ref().ok_or("first common declaration table absent")?;
    let b_common = other.common.as_ref().ok_or("second common declaration table absent")?;
    if !again.common.as_ref().is_some_and(|common| Arc::ptr_eq(common, a_common))
        || Arc::ptr_eq(a_common, b_common)
        || a_common != b_common
        || first.level != other.level || first.level != again.level {
        return Err("interleaved catalog declaration sources did not retain their exact scope".into());
    }
    use crate::assets::{camera_blur, entities::contacts::ContactDatabase, wwise::event_resources};
    let a_blur = camera_blur::BlurResources::acquire(&a)?;
    let b_blur = camera_blur::BlurResources::acquire(&b)?;
    let a_database = ContactDatabase::acquire(&a)?;
    let b_database = ContactDatabase::acquire(&b)?;
    if !Arc::ptr_eq(&a_blur, &camera_blur::BlurResources::acquire(&a)?)
        || !Arc::ptr_eq(&a_database, &ContactDatabase::acquire(&a)?)
        || Arc::ptr_eq(&a_blur, &b_blur) || Arc::ptr_eq(&a_database, &b_database)
        || a_blur != b_blur || a_database != b_database {
        return Err("camera or contact database acquisition crossed catalogs".into());
    }
    let camera_materials = camera_blur::retained_material_paths(&a);
    if camera_materials.is_empty() || camera_materials != camera_blur::retained_material_paths(&b) {
        return Err("camera material retention differs between catalogs".into());
    }
    let (path, kind) = first.level.keys().filter_map(|path| a.kind(path).map(|kind| (path, kind)))
        .min_by_key(|(path, _)| *path).ok_or("installed map-only contact source absent")?;
    let recorded = |catalog: &Catalog, kind, path| crate::loading::catalog_read_recorded(catalog.identity(), kind, path);
    if !recorded(&a, kind, path) || !recorded(&b, kind, path) {
        return Err("map declaration read history absent".into());
    }
    let mut sound_paths: Vec<_> = first.level.keys()
        .filter(|candidate| *candidate != path && a.kind(candidate) == Some("soundevent")).collect();
    sound_paths.sort_unstable();
    let (sound_path, event) = sound_paths.into_iter().find_map(|path| {
        let name = path.strip_prefix("generated/decls/soundevent/")?.strip_suffix(".decl")?;
        event_resources::acquire_with(&a, name, |path| read(&a, &first, path))
            .ok().map(|event| (path, event))
    }).ok_or("installed map-only sound event absent")?;
    drop(again);
    first.release_level_history(&a);
    if recorded(&a, kind, path) || !recorded(&b, kind, path)
        || !recorded(&a, "soundevent", sound_path) {
        return Err("contact history crossed catalogs or removed a live sound event".into());
    }
    drop(event);
    a.release_level_resources();
    if recorded(&a, "soundevent", sound_path) { return Err("sound event history survived its marker".into()); }
    let a_common = Arc::downgrade(a_common);
    let b_common = Arc::downgrade(b_common);
    let databases = a_database.len();
    let first_blur = Arc::downgrade(&a_blur);
    let first_database = Arc::downgrade(&a_database);
    let second_blur = Arc::downgrade(&b_blur);
    let second_database = Arc::downgrade(&b_database);
    drop((first, a_blur, a_database, a));
    if a_common.strong_count() != 0 || first_blur.strong_count() != 0 || first_database.strong_count() != 0
        || b_common.strong_count() == 0 || second_blur.strong_count() == 0 || second_database.strong_count() == 0 {
        return Err("common declaration session did not retire independently".into());
    }
    drop((other, b_blur, b_database, b));
    if b_common.strong_count() != 0 || second_blur.strong_count() != 0 || second_database.strong_count() != 0 {
        return Err("session resources outlived their catalog".into());
    }
    eprintln!("CONTACT_CATALOG_SCOPES reused=true independent=true payloads_match=true sound_credit=true camera_materials={} database_entries={databases}", camera_materials.len());
    Ok(())
}
