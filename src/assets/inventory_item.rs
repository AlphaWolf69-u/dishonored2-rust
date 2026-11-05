//! Inventory item declarations used by readable interactions.
//! The source declaration is acquired on demand and is not retained.
use crate::assets::{archive::Catalog, entities::{self, Value}};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Readable {
    pub kind: ReadableKind,
    pub title: String,
    pub text: String,
    pub note_type: i32,
    pub can_be_picked_up: bool,
    pub journal_image: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadableKind { Document, Book }

#[derive(Clone, Debug)]
pub struct AttributeModifier {
    /// Declaration identity and array index correspond to the source record
    /// retained by 14060b780 for timed removal.
    pub item: String,
    pub index: usize,
    pub tag: String,
    pub increase: f32,
    pub percentage: f32,
    pub duration: f32,
}

#[derive(Clone, Debug, Default)]
pub struct ItemModifiers {
    pub enhancement: bool,
    pub normal: Vec<AttributeModifier>,
    pub suppressed: Vec<AttributeModifier>,
}

/// Immutable item modifiers, acquired off-thread with the loadout/item.
pub fn modifiers(catalog: &Catalog, item: &str) -> Result<ItemModifiers, String> {
    let fields = item_fields(catalog, item)?;
    modifiers_fields(item, &fields)
}

pub fn modifiers_fields(item: &str, fields: &[(String, Value)]) -> Result<ItemModifiers, String> {
    let number = |fields: &[(String, Value)], name: &str| -> Result<f32, String> {
        match atom(fields, name) {
            None => Ok(0.0),
            Some(value) => value.parse::<f32>().ok().filter(|value| value.is_finite())
                .ok_or_else(|| format!("invalid modifier {name}: {value}")),
        }
    };
    let parse_list = |name: &str, suppressed: bool| -> Result<Vec<AttributeModifier>, String> {
        let Some(Value::Scope(list)) = fields.iter().find(|(key, _)| key == name).map(|(_, value)| value) else { return Ok(Vec::new()) };
        let count = atom(list, "num").unwrap_or("0").parse::<usize>().map_err(|_| "invalid modifier count")?;
        let mut result = Vec::new();
        for index in 0..count {
            let key = format!("item[{index}]");
            let Some(Value::Scope(fields)) = list.iter().find(|(name, _)| name == &key).map(|(_, value)| value) else { continue };
            let Some(tag) = atom(fields, "m_attribute").filter(|tag| !tag.is_empty() && !tag.eq_ignore_ascii_case("NULL")) else { continue };
            // Two lists live at different declaration addresses, even when
            // both have item[0]. Preserve distinct stable record identities.
            let identity = index.checked_mul(2).and_then(|index| index.checked_add(usize::from(suppressed)))
                .ok_or("modifier identity overflow")?;
            result.push(AttributeModifier { item: item.to_owned(), index: identity, tag: tag.to_owned(),
                increase: number(fields, "m_increase")?, percentage: number(fields, "m_percentage")?,
                duration: number(fields, "m_duration")? });
        }
        Ok(result)
    };
    Ok(ItemModifiers {
        enhancement: matches!(atom(fields, "m_type"), Some("TYPE_ENHANCEMENT" | "18")),
        normal: parse_list("m_modifiers", false)?,
        suppressed: parse_list("m_magicSuppressedModifiers", true)?,
    })
}

/// 140617930 applies the character inventory component's default loadout
/// when there is no restored inventory, before any gameflow loadout.
pub fn default_loadout(catalog: &Catalog, character: &str) -> Result<Vec<(String, u32)>, String> {
    let read = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog.read(path).map_err(|error| error.to_string())?
            .map(|bytes| entities::declaration_text(&bytes).into_owned()))
    };
    let entity = format!("models/characters/player/{character}/body.def");
    let component = entities::player::entity_component(&entity, "cpntinventory", |path| read(path))?
        .ok_or_else(|| format!("{entity} has no inventory component"))?;
    let Some(name) = atom(&component, "m_defaultLoadout").filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL")) else {
        return Ok(Vec::new());
    };
    let mut next = Some(name.to_owned());
    let mut chain = Vec::new();
    while let Some(name) = next.take() {
        if chain.len() >= 32 || chain.iter().any(|(previous, _): &(String, Vec<(String, Value)>)| previous == &name) {
            return Err(format!("inventory loadout inheritance cycle or excessive depth: {name}"));
        }
        let path = format!("generated/decls/inventoryloadout/{name}.decl");
        let source = read(&path)?.ok_or_else(|| format!("inventory loadout missing: {path}"))?;
        let fields = entities::parse_declaration(&source).map_err(|error| format!("{path}: {error}"))?;
        next = atom(&fields, "inherit").map(str::to_owned);
        chain.push((name, fields));
    }
    let mut entries: Vec<(String, Option<String>, Option<u32>)> = Vec::new();
    for (_, fields) in chain.iter().rev() {
        let Some(Value::Scope(edit)) = fields.iter().find(|(key, _)| key == "edit").map(|(_, value)| value) else { continue };
        let Some(Value::Scope(items)) = edit.iter().find(|(key, _)| key == "m_items").map(|(_, value)| value) else { continue };
        for (key, value) in items {
            if let Some(id) = key.strip_prefix("item_remove[").and_then(|s| s.strip_suffix(']')) {
                entries.retain(|entry| entry.0 != id); continue;
            }
            let Some(id) = key.strip_prefix("item_add[").or_else(|| key.strip_prefix("item_override["))
                .and_then(|s| s.strip_suffix(']')) else { continue };
            let Value::Scope(fields) = value else { return Err("inventory loadout item is not a scope".into()) };
            let index = entries.iter().position(|entry| entry.0 == id).unwrap_or_else(|| {
                entries.push((id.to_owned(), None, None)); entries.len() - 1
            });
            if let Some(path) = atom(fields, "m_decl") { entries[index].1 = Some(path.to_owned()); }
            if let Some(quantity) = atom(fields, "m_quantity") {
                entries[index].2 = Some(quantity.parse().map_err(|_| format!("invalid loadout quantity {quantity}"))?);
            }
        }
    }
    entries.into_iter().filter_map(|(_, path, quantity)| path.filter(|path| !path.eq_ignore_ascii_case("NULL"))
        .map(|path| quantity.map(|quantity| (path, quantity)).ok_or_else(|| "loadout item quantity missing".to_owned())))
        .collect()
}

/// Per-item maximums from the selected player's inventory collection. Only
/// overrides for relevant readable items are kept after preparation.
pub struct ReadableLimits {
    default: u32,
    overrides: HashMap<String, u32>,
}
impl ReadableLimits {
    pub fn maximum(&self, item: &str) -> u32 {
        if item.bytes().all(|byte| !byte.is_ascii_uppercase()) {
            self.overrides.get(item).copied().unwrap_or(self.default)
        } else {
            self.overrides.get(&item.to_ascii_lowercase()).copied().unwrap_or(self.default)
        }
    }
    /// 140617300 gives every inventory item either its collection override or
    /// the collection type's default (0/1, enum 1419feba0).
    pub fn load(catalog: &Catalog, character: &str, wanted: impl Iterator<Item = String>) -> Result<Self, String> {
        let wanted: std::collections::HashSet<String> = wanted.map(|s| s.to_ascii_lowercase()).collect();
        let entity = format!("models/characters/player/{character}/body.def");
        let read = |path: &str| -> Result<Option<String>, String> {
            Ok(catalog.read(path).map_err(|e| e.to_string())?
                .map(|bytes| entities::declaration_text(&bytes).into_owned()))
        };
        let inventory = entities::player::entity_component(&entity, "cpntinventory", |path| read(path))?
            .ok_or_else(|| format!("{entity} has no inventory component"))?;
        let mut next = Some(atom(&inventory, "m_collection").ok_or("inventory collection is absent")?.to_owned());
        let mut chain = Vec::new();
        while let Some(name) = next.take() {
            if chain.len() >= 32 || chain.iter().any(|(previous, _): &(String, Vec<(String, Value)>)| previous == &name) {
                return Err(format!("inventory collection inheritance cycle or excessive depth: {name}"));
            }
            let path = format!("generated/decls/inventoryitemcollection/{name}.decl");
            let source = read(&path)?.ok_or_else(|| format!("inventory collection missing: {path}"))?;
            let fields = entities::parse_declaration(&source).map_err(|e| format!("{path}: {e}"))?;
            next = atom(&fields, "inherit").map(str::to_owned);
            chain.push((name, fields));
        }
        let mut default = 0;
        let mut entries: HashMap<String, (Option<String>, Option<u32>)> = HashMap::new();
        for (_, fields) in chain.iter().rev() {
            let Some(Value::Scope(edit)) = fields.iter().find(|(key, _)| key == "edit").map(|(_, v)| v) else { continue };
            if let Some(value) = atom(edit, "m_type") {
                default = match value { "ICT_DEFAULT_NOITEM" | "0" => 0, "ICT_DEFAULT_ONEITEM" | "1" => 1,
                    _ => return Err(format!("unknown inventory collection type {value}")) };
            }
            let Some(Value::Scope(items)) = edit.iter().find(|(key, _)| key == "m_items").map(|(_, v)| v) else { continue };
            for (key, value) in items {
                if let Some(id) = key.strip_prefix("item_remove[").and_then(|s| s.strip_suffix(']')) {
                    entries.remove(id); continue;
                }
                let Some(id) = key.strip_prefix("item_add[").or_else(|| key.strip_prefix("item_override["))
                    .and_then(|s| s.strip_suffix(']')) else { continue };
                let Value::Scope(fields) = value else { return Err("inventory collection entry is not a scope".into()) };
                let entry = entries.entry(id.to_owned()).or_default();
                if let Some(item) = atom(fields, "m_item") { entry.0 = Some(item.to_ascii_lowercase()); }
                if let Some(quantity) = atom(fields, "m_maxQuantity") {
                    entry.1 = Some(quantity.parse().map_err(|_| format!("invalid maximum quantity {quantity}"))?);
                }
            }
        }
        let mut overrides = HashMap::new();
        for (_, (item, maximum)) in entries {
            let Some(item) = item else { continue };
            if wanted.contains(&item) {
                overrides.insert(item, maximum.ok_or("inventory item maximum is absent")?);
            }
        }
        Ok(Self { default, overrides })
    }
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
    let fields = item_fields(catalog, item)?;
    readable_fields(&fields)
}

/// Resolve once, then pass these same fields to the note or map reader.
pub fn item_fields(catalog: &Catalog, item: &str) -> Result<Vec<(String, Value)>, String> {
    entities::player::resolved_decl("inventoryitem", item, |path| {
        Ok(catalog.read(path).map_err(|error| error.to_string())?
            .map(|bytes| entities::declaration_text(&bytes).into_owned()))
    })
}

pub fn readable_fields(fields: &[(String, Value)]) -> Result<Option<Readable>, String> {
    // Constructor 14063c510 initializes m_noteType to -1 and
    // m_readableCanBePickedUp to true.
    let kind = match atom(fields, "m_type") {
        Some("TYPE_NOTE_DOCUMENT") => ReadableKind::Document,
        Some("TYPE_BOOK") => ReadableKind::Book,
        _ => return Ok(None),
    };
    let note_type = atom(fields, "m_noteType").map(note_type).transpose()?.unwrap_or(-1);
    // 140844bf0 rejects an invalid note style except for books and maps.
    if kind == ReadableKind::Document && note_type == -1 { return Ok(None); }
    let can_be_picked_up = match atom(fields, "m_readableCanBePickedUp").unwrap_or("true") {
        "true" => true,
        "false" => false,
        other => return Err(format!("invalid m_readableCanBePickedUp {other}")),
    };
    Ok(Some(Readable {
        kind,
        title: atom(fields, "m_name").unwrap_or("").to_owned(),
        text: atom(fields, "m_description").unwrap_or("").to_owned(),
        note_type,
        can_be_picked_up,
        journal_image: atom(fields, "m_journalImage").filter(|image| !image.is_empty()).map(str::to_owned),
    }))
}
