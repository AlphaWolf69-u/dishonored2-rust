//! Inventory item declarations used by readable interactions.
//! The source declaration is acquired on demand and is not retained.
use crate::assets::{archive::Catalog, entities::{self, Value}};

#[derive(Clone, Debug)]
pub struct Readable {
    pub title: String,
    pub text: String,
    pub note_type: i32,
    pub can_be_picked_up: bool,
}

fn atom<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    fields.iter().find(|(key, _)| key == name).and_then(|(_, value)| match value {
        Value::Atom(value) => Some(value.trim_matches('"')),
        _ => None,
    })
}

// arkUINoteType_t's registration function is 141a1bc80.
fn note_type(name: &str) -> Result<i32, String> {
    let value = match name {
        "ARK_UI_NOTETYPE_INVALID" => -1,
        "ARK_UI_NOTETYPE_NEUTRAL" => 0,
        "ARK_UI_NOTETYPE_OFFICE" => 1,
        "ARK_UI_NOTETYPE_THUG" => 2,
        "ARK_UI_NOTETYPE_RICH" => 3,
        "ARK_UI_NOTETYPE_OFFICE_DUNWALL" => 4,
        "ARK_UI_NOTETYPE_OFFICE_KARNACA" => 5,
        "ARK_UI_NOTETYPE_NEUTRAL_HANDWRITTEN" => 6,
        "ARK_UI_NOTETYPE_JINDOSH" => 7,
        "ARK_UI_NOTETYPE_DUNWALL_COURIER" => 8,
        "ARK_UI_NOTETYPE_KARNACA_GAZETTE" => 9,
        "ARK_UI_NOTETYPE_OFFICE_OVERSEER" => 10,
        "ARK_UI_NOTETYPE_BLACKMARKET" => 11,
        "ARK_UI_NOTETYPE_UNEDERGROUND_NEWSPAPER" => 12,
        "ARK_UI_BOOKTYPE_NEUTRAL" => 100,
        "ARK_UI_BOOKTYPE_DIARY_DIRTY" => 101,
        "ARK_UI_BOOKTYPE_DIARY_RICH" => 102,
        _ => return name.parse().map_err(|_| format!("unknown note type {name}")),
    };
    Ok(value)
}

/// Resolve the item declaration and inherited values. Non-readable items and
/// interactive maps do not enter the note movie. The latter use `_C_ShowMap`.
pub fn readable(catalog: &Catalog, item: &str) -> Result<Option<Readable>, String> {
    let mut chain = Vec::new();
    let mut next = Some(item.to_owned());
    while let Some(name) = next.take() {
        if chain.len() >= 32 || chain.iter().any(|(previous, _): &(String, Vec<(String, Value)>)| previous == &name) {
            return Err(format!("inventory item inheritance cycle or excessive depth: {name}"));
        }
        let path = format!("generated/decls/inventoryitem/{name}.decl");
        let bytes = catalog.read(&path).map_err(|e| e.to_string())?
            .ok_or_else(|| format!("inventory item missing: {path}"))?;
        let text = entities::declaration_text(&bytes);
        let fields = entities::parse_declaration(&text).map_err(|e| format!("{path}: {e}"))?;
        next = atom(&fields, "inherit").map(str::to_owned);
        chain.push((name, fields));
    }
    // Child fields override their parent. Constructor 14063c510 initializes
    // m_noteType to -1 and m_readableCanBePickedUp to true.
    let mut kind = None;
    let mut title = None;
    let mut text = None;
    let mut style = None;
    let mut can_pick_up = None;
    for (_, fields) in &chain {
        let Some(Value::Scope(edit)) = fields.iter().find(|(k, _)| k == "edit").map(|(_, v)| v) else { continue };
        kind = kind.or_else(|| atom(edit, "m_type"));
        title = title.or_else(|| atom(edit, "m_name"));
        text = text.or_else(|| atom(edit, "m_description"));
        style = style.or_else(|| atom(edit, "m_noteType"));
        can_pick_up = can_pick_up.or_else(|| atom(edit, "m_readableCanBePickedUp"));
    }
    if !matches!(kind, Some("TYPE_NOTE_DOCUMENT" | "TYPE_BOOK")) {
        return Ok(None);
    }
    let can_be_picked_up = match can_pick_up.unwrap_or("true") {
        "true" => true,
        "false" => false,
        other => return Err(format!("invalid m_readableCanBePickedUp {other}")),
    };
    Ok(Some(Readable {
        title: title.unwrap_or("").to_owned(),
        text: text.unwrap_or("").to_owned(),
        note_type: style.map(note_type).transpose()?.unwrap_or(-1),
        can_be_picked_up,
    }))
}
