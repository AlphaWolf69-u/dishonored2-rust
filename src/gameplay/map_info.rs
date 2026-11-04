//! `idDeclMapInfo`: the decl of the loaded map (`gameLocal.m_mapInfo`,
//! `+0x337118`), found by its `m_mapPath` / `m_highChaosMapPath`.
use crate::assets::archive::Catalog;
use crate::gameplay::options::decl::{atom, int, read_edit, scope, Fields};
use std::sync::RwLock;

/// The map the client loaded (`maps/....map`).
static CURRENT_MAP: RwLock<Option<String>> = RwLock::new(None);

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
    base: std::path::PathBuf,
    catalog_identity: u64,
    paths: Vec<String>,
    next: usize,
    targets: std::collections::HashMap<String, usize>,
    declarations: std::collections::HashMap<usize, std::sync::Arc<Fields>>,
    resolved: std::collections::HashMap<String, Option<std::sync::Arc<Fields>>>,
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
            || paths.iter().any(|path| catalog.has_overlay_file(path));
        Self { base: catalog.base().to_owned(), catalog_identity: catalog.identity(), paths, next: 0, targets: Default::default(),
            declarations: Default::default(), resolved: Default::default(), reference }
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

    fn scan_reference(&self, catalog: &Catalog, map: &str, counts: &mut LookupCounts) -> Option<std::sync::Arc<Fields>> {
        for index in 0..self.paths.len() {
            counts.scanned += 1;
            let Ok(edit) = self.read(catalog, index, counts) else { continue; };
            if ["m_mapPath", "m_highChaosMapPath"].iter()
                .any(|key| atom(&edit, key).is_some_and(|value| value.eq_ignore_ascii_case(map))) {
                return Some(std::sync::Arc::new(edit));
            }
        }
        None
    }

    fn disable_index(&mut self) {
        self.reference = true;
        self.targets = Default::default();
        self.declarations = Default::default();
    }

    fn lookup(&mut self, catalog: &Catalog, map: &str, counts: &mut LookupCounts) -> Option<std::sync::Arc<Fields>> {
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
                    let edit = std::sync::Arc::new(edit);
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
                    return Some(std::sync::Arc::new(edit));
                }
                continue;
            }
            for key in ["m_mapPath", "m_highChaosMapPath"] {
                if let Some(value) = atom(&edit, key) {
                    self.targets.entry(value.to_ascii_lowercase()).or_insert(index);
                }
            }
            if self.targets.get(map) == Some(&index) {
                let edit = std::sync::Arc::new(edit);
                self.declarations.insert(index, edit.clone());
                return Some(edit);
            }
        }
        None
    }
}

/// The mapinfo decl whose low- or high-chaos map is `map`.
pub fn find(catalog: &Catalog, map: &str) -> Option<std::sync::Arc<Fields>> {
    static RESIDENT: std::sync::Mutex<Option<MapInfoRegistry>> = std::sync::Mutex::new(None);
    let map = map.to_ascii_lowercase();
    let mut resident = RESIDENT.lock().unwrap();
    if resident.as_ref().is_none_or(|registry| registry.base != catalog.base() || registry.catalog_identity != catalog.identity()) {
        *resident = Some(MapInfoRegistry::new(catalog));
    }
    let registry = resident.as_mut().unwrap();
    if !registry.reference {
        if let Some(edit) = registry.resolved.get(&map) { return edit.clone(); }
    }
    let mut counts = LookupCounts::default();
    let result = registry.lookup(catalog, &map, &mut counts);
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
    if !registry.reference { registry.resolved.insert(map, result.clone()); }
    result
}

/// The loaded map's loading info, looked up once per map (the game holds
/// the decl as `m_mapInfo` from the map load).
pub fn current_loading_info(catalog: &Catalog) -> Option<LoadingInfo> {
    type Key = (std::path::PathBuf, String, u64);
    static CACHE: RwLock<Option<(Key, Option<LoadingInfo>)>> = RwLock::new(None);
    let map = current_map()?;
    let key = (catalog.base().to_owned(), map, catalog.identity());
    // A loose mapinfo declaration may be edited while this catalog is live.
    let mutable_mapinfo = catalog.has_overlays()
        && catalog.paths_containing("generated/decls/mapinfo/").iter()
            .any(|path| catalog.has_overlay_file(path));
    if let Some((cached, info)) = CACHE.read().unwrap().as_ref() {
        if !mutable_mapinfo && *cached == key {
            return info.clone();
        }
    }
    let info = find(catalog, &key.1).and_then(|edit| loading_info(&edit));
    *CACHE.write().unwrap() = Some((key, info.clone()));
    info
}

pub fn loading_info(edit: &Fields) -> Option<LoadingInfo> {
    let info = scope(edit, "m_loadingInfo")?;
    Some(LoadingInfo { mission_index: int(info, "m_missionIndex", 0), mission_name: atom(info, "m_missionName").unwrap_or("").to_string() })
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
