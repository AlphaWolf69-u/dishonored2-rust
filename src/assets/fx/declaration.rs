//! FX declaration acquisition, including the parent's authored values.
//! 140022fd0 acquires the parent with 1400607f0 before applying child edits;
//! idDeclFX registers at 1413630d0 and releases backing at 1401c5990.
//! Resolved syntax stays temporary: callers retain only their typed FX data.
use crate::{archive::Catalog, assets::entities::{self, Value}};
use std::collections::HashSet;

type Fields = Vec<(String, Value)>;
pub(crate) fn read(catalog: &Catalog, name: &str) -> Result<Fields, String> {
    read_with(name, |path| crate::assets::declaration_texts::read_common_or_catalog(catalog, path))
}

pub(crate) fn read_with(name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Fields, String> {
    let (first, parent) = layer(name, &mut read)?;
    // Ordinary definitions retain the former move-only edit-block path.
    let Some(parent) = parent else { return Ok(first) };
    let mut layers = vec![first];
    let mut active = HashSet::from([key(name)]);
    let mut next = Some(parent);
    while let Some(name) = next {
        let path = key(&name);
        if !active.insert(path.clone()) {
            return Err(format!("FX declaration inheritance cycle at {path}"));
        }
        let (fields, parent) = layer(&name, &mut read)?;
        layers.push(fields);
        next = parent;
    }
    let mut fields = layers.pop().expect("FX inheritance has a base layer");
    while let Some(local) = layers.pop() { entities::overlay_owned(&mut fields, local); }
    Ok(fields)
}
fn key(name: &str) -> String {
    format!("generated/decls/fx/{name}.decl").replace('\\', "/").to_ascii_lowercase()
}
fn layer(name: &str, read: &mut impl FnMut(&str) -> Result<Option<String>, String>)
    -> Result<(Fields, Option<String>), String> {
    let path = key(name);
    let source = read(&path)?.ok_or_else(|| format!("FX declaration missing: {path}"))?;
    let mut fields = entities::parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
    // 140143bb0 is the native length-zero check; an empty name has no parent.
    let parent = entities::event_graph::atom(&fields, "inherit").filter(|name| !name.is_empty()).map(str::to_owned);
    let edit = if let Some(index) = fields.iter().position(|(name, _)| name == "edit") {
        match fields.swap_remove(index).1 {
            Value::Scope(edit) => edit,
            _ => return Err(format!("{path}: edit is not a scope")),
        }
    } else { Vec::new() };
    Ok((edit, parent))
}
