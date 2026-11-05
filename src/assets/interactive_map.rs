//! Metadata for the original interactive-map presentation.
use crate::assets::{archive::Catalog, entities::{self, Value}};
use crate::gameplay::options::decl::{atom, boolean, list_items, scope, unquote};

#[derive(Clone, Debug)]
pub struct Map {
    pub title: String,
    pub item_name: String,
    pub can_be_picked_up: bool,
    pub image: String,
    pub thumbnail: Option<String>,
    pub description: String,
    pub legends: Vec<String>,
    pub map_type: i32,
    pub has_player_position: bool,
}

/// Resolve the item and map component inheritance on demand; callers retain
/// the metadata only for the presentation, not the source declaration trees.
pub fn resolve(catalog: &Catalog, item: &str) -> Result<Option<Map>, String> {
    let read = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| entities::declaration_text(&bytes).into_owned()))
    };
    let fields = entities::player::resolved_decl("inventoryitem", item, read)?;
    resolve_fields(catalog, item, &fields)
}

/// The interaction owner may reuse item fields it has already resolved.
pub fn resolve_fields(catalog: &Catalog, item: &str, fields: &[(String, Value)]) -> Result<Option<Map>, String> {
    if atom(fields, "m_type") != Some("TYPE_INTERACTIVE_MAP") { return Ok(None); }
    let read = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(|bytes| entities::declaration_text(&bytes).into_owned()))
    };
    let item_name = atom(fields, "m_name").unwrap_or("").to_owned();
    let can_be_picked_up = boolean(fields, "m_readableCanBePickedUp", true);
    let thumbnail = atom(&fields, "m_journalImage").filter(|name| !name.is_empty() && *name != "NULL").map(str::to_owned);
    let description = atom(&fields, "m_description").unwrap_or("").to_owned();
    // 1408450b0 falls through to the note presentation when map metadata is absent.
    let Some(info) = atom(&fields, "m_levelMapInfo").filter(|info| !info.is_empty() && *info != "NULL") else { return Ok(None) };
    let image = atom(&fields, "m_journalImageLarge").filter(|name| !name.is_empty() && *name != "NULL")
        .ok_or_else(|| format!("map item {item} has no large image"))?.to_owned();
    let fields = entities::player::resolved_decl("cpntinteractivemap", info, read)?;
    let title = atom(&fields, "m_title").unwrap_or("").to_owned();
    let legends = scope(&fields, "m_legends").map(|list| list_items(list).into_iter().filter_map(|value| match value {
        Value::Atom(text) => Some(unquote(text).to_owned()), Value::Scope(_) => None,
    }).collect()).unwrap_or_default();
    let map_type = match atom(&fields, "m_type") {
        None | Some("ARK_UI_INTERACTIVE_MAP_TYPE_NEUTRAL") => 0,
        Some(other) => return Err(format!("unsupported interactive map type {other}")),
    };
    Ok(Some(Map { title, item_name, can_be_picked_up, image, thumbnail, description, legends, map_type, has_player_position: boolean(&fields, "m_mapHasPlayerPosition", false) }))
}
