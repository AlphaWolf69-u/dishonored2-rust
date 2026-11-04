//! Shared declaration-text acquisition for physics, contact FX and sound.
//! Session init/common and map-only snapshots retain distinct lifetimes.
use crate::assets::archive::Catalog;
use std::{borrow::Cow, collections::HashMap, sync::Arc};

type TextMap = HashMap<String, String>;
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
struct SharedTexts {
    catalog: u64,
    language: String,
    // None means that a mutable overlay requires the existing per-level scan.
    common: Option<Arc<TextMap>>,
}
static COMMON_TEXTS: std::sync::Mutex<Option<SharedTexts>> = std::sync::Mutex::new(None);

#[derive(Clone, Copy)]
pub(crate) enum PhysicsUse { World, Model, Machinery, Runtime }

#[derive(Default)]
struct PhysicsStats {
    requests: usize,
    reads: usize,
    bytes: usize,
    common_hits: usize,
    local_hits: usize,
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
            COMMON_TEXTS.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
                .filter(|entry| entry.catalog == catalog.identity() && entry.language == language)
                .and_then(|entry| entry.common.clone())
        } else { None };
        Self { enabled, common, ..Default::default() }
    }

    pub(crate) fn read(&mut self, catalog: &Catalog, path: &str, role: PhysicsUse) -> Result<Option<String>, String> {
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
        let text = bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned());
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
        if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() != Ok("1") { return; }
        for (role, stats) in ["world", "model", "machinery", "runtime"].into_iter().zip(&self.stats) {
            eprintln!("MEMORY physics text source: map={map} role={role} enabled={} requests={} reads={} decoded_bytes={} common_hits={} local_hits={}",
                self.enabled, stats.requests, stats.reads, stats.bytes, stats.common_hits, stats.local_hits);
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
        let owner = COMMON_TEXTS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(value) = owner.as_ref().filter(|entry| entry.catalog == catalog.identity())
            .and_then(|entry| entry.common.as_deref()).and_then(|texts| texts.get(path))
        {
            return Ok(Some(value.clone()));
        }
    }
    catalog.read(path)
        .map(|bytes| bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
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
    Ok(bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
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
        .map(|bytes| bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
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
    let mut owner = COMMON_TEXTS.lock().unwrap_or_else(|e| e.into_inner());
    // Loose bootstrap manifests can change their membership during a session.
    if catalog.has_overlays()
        && (catalog.has_overlay_file("generated/buildgame/init.mapresources")
            || catalog.has_overlay_file("generated/buildgame/common.mapresources"))
    {
        *owner = Some(SharedTexts {
            catalog: catalog.identity(),
            language: language.to_owned(),
            common: None,
        });
        return Ok(None);
    }
    if let Some(entry) = owner
        .as_ref()
        .filter(|entry| entry.catalog == catalog.identity() && entry.language == language)
    {
        return Ok(entry.common.clone());
    }
    // A different catalog/language cannot keep the old session's table pinned.
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
        *owner = Some(SharedTexts {
            catalog: catalog.identity(),
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
        catalog: catalog.identity(),
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
