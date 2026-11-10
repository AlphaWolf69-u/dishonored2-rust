//! `idDeclMapInfo`: the decl of the loaded map (`gameLocal.m_mapInfo`,
//! `+0x337118`), found by its `m_mapPath` / `m_highChaosMapPath`.
use crate::assets::archive::Catalog;
use crate::gameplay::options::decl::{atom, int, read_edit, scope, Fields};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

/// The map the client loaded (`maps/....map`).
static CURRENT_MAP: RwLock<Option<String>> = RwLock::new(None);

#[derive(Default)]
pub(crate) struct MapInfoCache {
    registry: OnceLock<Mutex<MapInfoRegistry>>,
    loading: RwLock<Option<(String, Option<LoadingInfo>)>>,
}

impl MapInfoCache {
    pub(crate) fn entry_count(&mut self) -> usize {
        usize::from(self.registry.get().is_some())
            + usize::from(self.loading.get_mut().unwrap_or_else(|error| error.into_inner()).is_some())
    }
}

fn cache(catalog: &Catalog) -> &MapInfoCache {
    catalog.map_info.get_or_init(Default::default)
}

pub fn set_current_map(map: &str) {
    *CURRENT_MAP.write().unwrap() = Some(map.to_ascii_lowercase());
}

pub fn current_map() -> Option<String> {
    CURRENT_MAP.read().unwrap().clone()
}

/// `m_loadingInfo` (`arkLoadingScreenInfo_t` at decl `+0x200`): the fields
/// the pause header reads.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadingInfo {
    /// `m_missionIndex` (`+0x20c`).
    pub mission_index: i32,
    /// `m_missionName` (`+0x210`), a string key.
    pub mission_name: String,
}

/// Session metadata for the original declaration registry (1400607f0 /
/// 14005feb0). Only requested declarations retain their full parsed fields.
struct MapInfoRegistry {
    paths: Vec<String>,
    next: usize,
    targets: std::collections::HashMap<String, usize>,
    declarations: std::collections::HashMap<usize, Arc<Fields>>,
    reference: bool,
}

#[derive(Default)]
struct LookupCounts { reads: usize, bytes: usize, scanned: usize, shared: bool }

impl MapInfoRegistry {
    fn new(catalog: &Catalog) -> Self {
        let paths = catalog.paths_containing("generated/decls/mapinfo/");
        // A live overlay may change between calls. Preserve the existing scan
        // semantics for it instead of retaining a stale metadata index.
        let reference = std::env::var("DH2_DEBUG_SCAN_MAPINFO").as_deref() == Ok("1")
            || catalog.has_overlay_declaration("generated/decls/mapinfo/");
        Self { paths, next: 0, targets: Default::default(), declarations: Default::default(), reference }
    }

    fn read(&self, catalog: &Catalog, index: usize, counts: &mut LookupCounts) -> Result<Fields, String> {
        let path = &self.paths[index];
        let name = path.strip_prefix("generated/decls/mapinfo/")
            .and_then(|name| name.strip_suffix(".decl")).ok_or("invalid mapInfo resource path")?;
        let mut read = |path: &str| {
            counts.reads += 1;
            let bytes = catalog.read(path).ok().flatten();
            counts.bytes += bytes.as_ref().map_or(0, Vec::len);
            bytes
        };
        read_edit(&mut read, "mapinfo", name)
    }

    fn scan_reference(&self, catalog: &Catalog, map: &str, counts: &mut LookupCounts) -> Option<Arc<Fields>> {
        for index in 0..self.paths.len() {
            counts.scanned += 1;
            let Ok(edit) = self.read(catalog, index, counts) else { continue; };
            if ["m_mapPath", "m_highChaosMapPath"].iter()
                .any(|key| atom(&edit, key).is_some_and(|value| value.eq_ignore_ascii_case(map))) {
                return Some(Arc::new(edit));
            }
        }
        None
    }

    fn disable_index(&mut self) {
        self.reference = true;
        self.targets = Default::default();
        self.declarations = Default::default();
    }

    fn lookup(&mut self, catalog: &Catalog, map: &str, counts: &mut LookupCounts) -> Option<Arc<Fields>> {
        if self.reference { return self.scan_reference(catalog, map, counts); }
        if let Some(&index) = self.targets.get(map) {
            if let Some(edit) = self.declarations.get(&index) {
                counts.shared = true;
                return Some(edit.clone());
            }
            // If an indexed declaration becomes unreadable, retain the old
            // full-scan failure behavior for this and subsequent lookups.
            match self.read(catalog, index, counts) {
                Ok(edit) => {
                    let edit = Arc::new(edit);
                    self.declarations.insert(index, edit.clone());
                    return Some(edit);
                }
                Err(_) => { self.disable_index(); return self.scan_reference(catalog, map, counts); }
            }
        }
        while self.next < self.paths.len() {
            let index = self.next;
            self.next += 1;
            counts.scanned += 1;
            let edit = match self.read(catalog, index, counts) {
                Ok(edit) => edit,
                Err(_) => { self.disable_index(); continue; }
            };
            if self.reference {
                if ["m_mapPath", "m_highChaosMapPath"].iter()
                    .any(|key| atom(&edit, key).is_some_and(|value| value.eq_ignore_ascii_case(map))) {
                    return Some(Arc::new(edit));
                }
                continue;
            }
            for key in ["m_mapPath", "m_highChaosMapPath"] {
                if let Some(value) = atom(&edit, key) {
                    self.targets.entry(value.to_ascii_lowercase()).or_insert(index);
                }
            }
            if self.targets.get(map) == Some(&index) {
                let edit = Arc::new(edit);
                self.declarations.insert(index, edit.clone());
                return Some(edit);
            }
        }
        None
    }
}

/// The mapinfo decl whose low- or high-chaos map is `map`.
pub fn find(catalog: &Catalog, map: &str) -> Option<Arc<Fields>> {
    let normalized;
    let map = if map.bytes().any(|byte| byte.is_ascii_uppercase()) {
        normalized = map.to_ascii_lowercase();
        &normalized
    } else {
        map
    };
    let mut registry = cache(catalog).registry.get_or_init(|| Mutex::new(MapInfoRegistry::new(catalog)))
        .lock().unwrap_or_else(|error| error.into_inner());
    if !registry.reference {
        if let Some(edit) = registry.targets.get(map).and_then(|index| registry.declarations.get(index)) {
            return Some(edit.clone());
        }
    }
    let mut counts = LookupCounts::default();
    let result = registry.lookup(catalog, map, &mut counts);
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        let signature = if std::env::var("DH2_DEBUG_MAPINFO_SIGNATURES").as_deref() == Ok("1") {
            result.as_ref().map(|edit| {
                let mut crc = flate2::Crc::new();
                crc.update(format!("{edit:?}").as_bytes());
                crc.sum()
            })
        } else { None };
        let index_bytes = registry.paths.iter().map(String::len).sum::<usize>()
            + registry.targets.keys().map(String::len).sum::<usize>();
        eprintln!("MEMORY mapinfo lookup: map={map} reference={} reads={} decoded_bytes={} scanned={} indexed={} index_string_bytes={} shared={} signature={signature:?}",
            registry.reference, counts.reads, counts.bytes, counts.scanned, registry.targets.len(), index_bytes, counts.shared);
    }
    result
}

/// The loaded map's loading info, looked up once per map (the game holds
/// the decl as `m_mapInfo` from the map load).
pub fn current_loading_info(catalog: &Catalog) -> Option<LoadingInfo> {
    let map = current_map()?;
    // Editable map-info declarations are always read through the scan.
    if catalog.has_overlay_declaration("generated/decls/mapinfo/") {
        return find(catalog, &map).and_then(|edit| loading_info(&edit));
    }
    let owner = cache(catalog);
    if let Some((cached, info)) = owner.loading.read().unwrap_or_else(|error| error.into_inner()).as_ref() {
        if *cached == map { return info.clone(); }
    }
    let info = find(catalog, &map).and_then(|edit| loading_info(&edit));
    *owner.loading.write().unwrap_or_else(|error| error.into_inner()) = Some((map, info.clone()));
    info
}

pub fn loading_info(edit: &Fields) -> Option<LoadingInfo> {
    let info = scope(edit, "m_loadingInfo")?;
    Some(LoadingInfo { mission_index: int(info, "m_missionIndex", 0), mission_name: atom(info, "m_missionName").unwrap_or("").to_string() })
}

/// Observe installed map-info aliases and the loading-label owner on A/B/A.
#[cfg(feature = "game")]
pub(crate) fn observe_catalog_scopes(base: &std::path::Path, requested: &str) -> Result<(), String> {
    let a = Catalog::open(base).map_err(|error| error.to_string())?;
    let b = Catalog::open(base).map_err(|error| error.to_string())?;
    let level = crate::assets::level::LevelResources::resolve(&a, requested).map_err(|error| error.to_string())?;
    let map = format!("{}.map", level.entities.trim_end_matches(".entities"));
    set_current_map(&map);
    let first = find(&a, &map).ok_or("installed map info absent")?;
    let first_label = current_loading_info(&a).ok_or("installed loading label absent")?;
    let other = find(&b, &map).ok_or("second catalog map info absent")?;
    let other_label = current_loading_info(&b).ok_or("second catalog loading label absent")?;
    let again = find(&a, &map).ok_or("returning catalog map info absent")?;
    let again_label = current_loading_info(&a).ok_or("returning catalog loading label absent")?;
    let high = atom(&first, "m_highChaosMapPath").filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case(&map))
        .ok_or("installed map info has no separate high-chaos path")?;
    let alias = find(&a, high).ok_or("high-chaos map info absent")?;
    let signature = blake3::hash(format!("{first:?}").as_bytes());
    if !Arc::ptr_eq(&first, &again) || !Arc::ptr_eq(&first, &alias)
        || Arc::ptr_eq(&first, &other) || first_label != other_label || first_label != again_label
        || signature != blake3::hash(format!("{other:?}").as_bytes()) {
        return Err("interleaved catalogs lost their map info identity or label".into());
    }
    let a_lease = Arc::downgrade(&first);
    let b_lease = Arc::downgrade(&other);
    drop(a);
    if a_lease.strong_count() == 0 { return Err("catalog close invalidated an active map-info reader".into()); }
    drop((first, again, alias));
    if a_lease.strong_count() != 0 || b_lease.strong_count() == 0 {
        return Err("map info did not retire independently".into());
    }
    drop((other, b));
    if b_lease.strong_count() != 0 { return Err("map info survived its last session".into()); }
    eprintln!("MAP_INFO_CATALOG_SCOPES map={map} mission={} signature={signature} aliased=true reused=true retired=true",
        first_label.mission_index);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires installed game archives"]
    fn edge_of_the_world_is_mission_2() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::assets::archive::Catalog::open(&base).unwrap();
        let edit = super::find(&catalog, "maps/campaign/edge_of_the_world/eotw_p_lowchaos.map").unwrap();
        let info = super::loading_info(&edit).unwrap();
        assert_eq!(info, super::LoadingInfo { mission_index: 2, mission_name: "Mission_EotW".into() });
    }
}


#[cfg(test)]
mod debug_listing {
    #[test]
    #[ignore = "diagnostic listing"]
    fn debug_list_mapinfo() {
        let base = crate::assets::installation::game_base().unwrap();
        let catalog = crate::assets::archive::Catalog::open(&base).unwrap();
        let edit = super::find(&catalog, "maps/campaign/edge_of_the_world/eotw_p_lowchaos.map").unwrap();
        for (k, v) in super::scope(&edit, "m_loadingInfo").unwrap() {
            if k != "m_tips" {
                println!("info {k} = {v:?}");
            }
        }
    }
}
