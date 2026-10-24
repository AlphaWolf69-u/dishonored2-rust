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

/// The mapinfo decl whose low- or high-chaos map is `map`.
pub fn find(catalog: &Catalog, map: &str) -> Option<Fields> {
    let mut read = |p: &str| catalog.read(p).ok().flatten();
    catalog.paths_containing("generated/decls/mapinfo/").into_iter().find_map(|path| {
        let name = path.strip_prefix("generated/decls/mapinfo/")?.strip_suffix(".decl")?.to_string();
        let edit = read_edit(&mut read, "mapinfo", &name).ok()?;
        let matches = ["m_mapPath", "m_highChaosMapPath"].iter().any(|k| atom(&edit, k).is_some_and(|m| m.eq_ignore_ascii_case(map)));
        matches.then_some(edit)
    })
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
