//! FX declaration acquisition, including the parent's authored values.
//! 140022fd0 acquires the parent with 1400607f0 before applying child edits;
//! idDeclFX registers at 1413630d0 and releases backing at 1401c5990.
//! Resolved syntax stays temporary: callers retain only their typed FX data.
use crate::{archive::Catalog, assets::entities::Value};

pub(crate) fn read(catalog: &Catalog, name: &str) -> Result<Vec<(String, Value)>, String> {
    read_with(name, |path| crate::assets::declaration_texts::read_common_or_catalog(catalog, path))
}

pub(crate) fn read_with(name: &str,
    read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Vec<(String, Value)>, String> {
    // The same inheritance resolver already serves contact FX and player
    // declarations. It preserves authored order and rejects ancestry cycles.
    crate::assets::entities::player::resolved_decl("fx", name, read)
}
