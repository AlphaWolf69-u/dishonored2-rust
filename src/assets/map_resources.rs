//! The resources a map loads before play (`ResourceManager.cpp`).
//!
//! `FUN_1400661e0` runs for `init`, `common` and the map, in that order, and
//! loads every entry of `generated/buildGame/<name>.mapResources` by type and
//! name (`thunk_FUN_1400607f0`); an image's LoadResourceInternal
//! (`1402ea140 -> 1402e9ff0`) uploads its base mip chain then. The file holds
//! a big-endian header (the entry count at +8) and 28-byte entries whose
//! first big-endian word is a resource id. Ids number the records of the
//! index of the archive that holds the `.mapResources` file's bytes; a localized decl
//! has one record per language under the same id.
use super::archive::index::{self, IndexEntry};
use super::archive::Catalog;
use std::collections::HashMap;
use std::path::Path;

type Result<T> = std::result::Result<T, String>;

const HEADER: usize = 12;
const ENTRY: usize = 28;

/// One resource of a load list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub kind: String,
    pub path: String,
}

/// The ids of a `.mapResources` file, in load order.
pub fn ids(bytes: &[u8]) -> Result<Vec<u32>> {
    let count = bytes.get(8..HEADER).ok_or("map resources header is truncated")?;
    let count = u32::from_be_bytes(count.try_into().unwrap()) as usize;
    if bytes.len() != HEADER + count * ENTRY {
        return Err(format!("map resources hold {} bytes for {count} entries", bytes.len()));
    }
    Ok((0..count).map(|i| u32::from_be_bytes(bytes[HEADER + i * ENTRY..][..4].try_into().unwrap())).collect())
}

/// The load list of `name` (`init`, `common` or a map path such as
/// `campaign/edge_of_the_world/eotw_p_lowchaos`) for `language`.
pub fn load_list(base: &Path, catalog: &Catalog, name: &str, language: &str) -> Result<Vec<Resource>> {
    let manifest = format!("generated/buildgame/{}.mapresources", name.to_ascii_lowercase());
    // The manifest's record in the patched index names the archive holding
    // its bytes; the ids are that archive's index numbering (an unchanged
    // manifest keeps the base numbering, which the patch index renumbers).
    let mut location = None;
    for group in 1..=4 {
        index::visit_group(base, group, |r| {
            if location.is_none() && r.path.eq_ignore_ascii_case(&manifest) {
                location = Some((r.group, r.archive_slot));
            }
        })
        .map_err(|e| e.to_string())?;
        if location.is_some() {
            break;
        }
    }
    let (group, slot) = location.ok_or_else(|| format!("{manifest} is not indexed"))?;
    let mut records: HashMap<u32, Vec<IndexEntry>> = HashMap::new();
    let path = index::slot_index_path(base, group, slot).map_err(|e| e.to_string())?;
    index::visit_file(&path, group, |r| records.entry(r.id).or_default().push(r)).map_err(|e| e.to_string())?;
    let bytes = catalog.read(&manifest).map_err(|e| e.to_string())?.ok_or_else(|| format!("{manifest} is missing"))?;
    let localized = format!("/localized/{language}/");
    let mut list = Vec::new();
    let mut missing = Vec::new();
    for id in ids(&bytes)? {
        // The patched `common` list names one id (2789) its index lacks.
        let Some(entries) = records.get(&id) else {
            missing.push(id);
            continue;
        };
        let chosen: Vec<&IndexEntry> = if entries.iter().any(|e| e.path.contains("/localized/")) {
            entries.iter().filter(|e| e.path.contains(&localized)).collect()
        } else {
            entries.iter().collect()
        };
        list.extend(chosen.into_iter().map(|e| Resource { kind: e.kind.clone(), path: e.path.to_ascii_lowercase().replace('\\', "/") }));
    }
    if !missing.is_empty() {
        eprintln!("{manifest}: ids without an index record: {missing:?}");
    }
    Ok(list)
}

/// The map's load lists in load order: `init`, `common`, then the map.
pub fn level_load_list(base: &Path, catalog: &Catalog, map: &str, language: &str) -> Result<Vec<Resource>> {
    let mut list = load_list(base, catalog, "init", language)?;
    list.extend(load_list(base, catalog, "common", language)?);
    list.extend(load_list(base, catalog, map, language)?);
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The installed lists resolve completely, and the map's list holds its
    /// navmesh and entities (its world, tome, cfg and sound propagation have
    /// later ids: the map loader reads them, not the list).
    #[test]
    #[ignore = "requires installed game archives"]
    fn installed_lists_resolve() {
        let base = crate::assets::installation::game_base().unwrap();
        let language = crate::assets::lang::read_text_language(&base).unwrap();
        let catalog = Catalog::open(&base).unwrap();
        let map = load_list(&base, &catalog, "campaign/edge_of_the_world/eotw_p_lowchaos", language).unwrap();
        for suffix in ["eotw_p_lowchaos.bnavmesh", "eotw_p_lowchaos.entities"] {
            assert!(map.iter().any(|r| r.path.ends_with(suffix)), "{suffix}");
        }
        let all = level_load_list(&base, &catalog, "campaign/edge_of_the_world/eotw_p_lowchaos", language).unwrap();
        let images = all.iter().filter(|r| r.kind == "image").count();
        println!("{} resources, {images} images", all.len());
        assert!(all.iter().all(|r| !r.path.contains("/localized/") || r.path.contains(&format!("/localized/{language}/"))));
    }
}
