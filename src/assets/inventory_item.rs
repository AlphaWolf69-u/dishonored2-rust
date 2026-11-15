//! Inventory item declarations used by readable interactions.
//! Catalog-owned identities serve typed consumers; temporary generic fields
//! retain read-generation leases rather than another declaration cache.
use crate::assets::{archive::Catalog, entities::{self, Value}};
mod sources;
pub(crate) mod configuration;
pub use sources::{PickableItem, PickableReadable, PickableOtherKind, PickableSources, PickableSourceLease, ItemFields};
pub(crate) use sources::{register_preload, promote_bootstrap};

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

#[derive(Clone, Copy, Debug, Default)]
pub struct Consumable {
    /// 141705ee0 registers these signed fields at offsets 0 and 4.
    pub health: i32,
    pub mana: i32,
}

fn consumable_fields(fields: &[(String, Value)]) -> Result<Consumable, String> {
    let Some(Value::Scope(fields)) = fields.iter().find(|(name, _)| name == "m_consumable").map(|(_, value)| value) else { return Ok(Consumable::default()) };
    let number = |name| atom(fields, name).unwrap_or("0").parse().map_err(|_| format!("invalid consumable {name}"));
    Ok(Consumable { health: number("m_healthQuantity")?, mana: number("m_manaQuantity")? })
}

/// 1419fe7e0 registers these item type names in this exact order.
pub fn item_type_number(value: &str) -> Result<u32, String> {
    const TYPES: [&str; 28] = ["TYPE_MISC", "TYPE_COIN", "TYPE_PLAYER_WEAPONRY", "TYPE_NPC_WEAPONRY",
        "TYPE_AMMO", "TYPE_BONE_CHARM", "TYPE_CONSUMABLE", "TYPE_RUNE", "TYPE_SHRINE", "TYPE_POWER",
        "TYPE_ARMOR", "TYPE_MISSIONITEM", "TYPE_NOTE_DOCUMENT", "TYPE_AUDIOGRAPH", "TYPE_POTION",
        "TYPE_PAINTING", "TYPE_BLUEPRINT", "TYPE_KEY", "TYPE_ENHANCEMENT", "TYPE_EQUIPMENT_UPGRADE",
        "TYPE_POWER_UPGRADE", "TYPE_GEAR_UPGRADE", "TYPE_REWIRE_TOOL", "TYPE_JOURNAL_GEAR_SECTION_INFO",
        "TYPE_INTERACTIVE_MAP", "TYPE_BOOK", "TYPE_RAW_WHALE_BONE", "TYPE_FAVOR"];
    TYPES.iter().position(|name| *name == value).map(|index| index as u32)
        .or_else(|| value.parse().ok().filter(|value| *value < TYPES.len() as u32))
        .ok_or_else(|| format!("unknown inventory item type {value}"))
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
    Ok(default_loadout_definition(catalog, character)?.map_or_else(Vec::new, |loadout|
        loadout.assignments().map(|(name, quantity)| (name.to_owned(), quantity.max(0) as u32)).collect()))
}

pub fn default_loadout_definition(catalog: &Catalog, character: &str) -> Result<Option<InventoryLoadout>, String> {
    let entity = format!("models/characters/player/{character}/body.def");
    let component = entities::player::entity_component(&entity, "cpntinventory", |path| {
        Ok(catalog.read(path).map_err(|error| error.to_string())?
            .map(entities::owned_declaration_text))
    })?.ok_or_else(|| format!("{entity} has no inventory component"))?;
    let Some(name) = atom(&component, "m_defaultLoadout").filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL")) else { return Ok(None) };
    InventoryLoadout::load(catalog, name).map(Some)
}

/// Share the selected collection declaration with the actual consumer.
/// The native first-entry rule and signed/default maxima live in its owner.
pub struct ReadableLimits { declaration: std::sync::Arc<configuration::Definition> }

/// A retained loadout declaration, including the flags consumed by
/// 613ff0/617930. The item source owner also owns this configuration.
pub struct InventoryLoadout { declaration: std::sync::Arc<configuration::Definition> }
impl InventoryLoadout {
    pub fn load(catalog: &Catalog, name: &str) -> Result<Self, String> {
        let sources = PickableSourceLease::for_catalog(catalog)?;
        let declaration = sources.0.lock().map_err(|_| "inventory declaration owner is poisoned")?.configurations.loadout(catalog, name)?;
        Ok(Self { declaration })
    }
    pub fn clear(&self) -> bool { self.declaration.clear }
    pub fn allow_auto_equip(&self) -> bool { self.declaration.allow_auto_equip }
    pub fn assignments(&self) -> impl Iterator<Item = (&str, i32)> { self.declaration.assignments() }
}
impl ReadableLimits {
    pub fn maximum(&self, item: &str) -> u32 { self.declaration.maximum(item) }
    /// Enumerate the authored collection, preserving first-entry capacity
    /// semantics through maximum(). The caller acquires typed items from the
    /// existing PickableSources owner, never a second declaration cache.
    pub fn assignments(&self) -> impl Iterator<Item = (&str, u32)> {
        self.declaration.assignments().map(|(path, _)| (path, self.maximum(path)))
    }
    /// Prepare the actual collection records off the game thread. Each
    /// result retains the source owner's declaration generation; no generic
    /// field trees or a second declaration owner escape this operation.
    pub fn acquire_items(&self, catalog: &Catalog) -> Result<Vec<(std::sync::Arc<PickableItem>, u32)>, String> {
        let sources = PickableSourceLease::for_catalog(catalog)?;
        let mut owner = sources.0.lock().map_err(|_| "inventory declaration owner is poisoned")?;
        let mut result = Vec::new();
        for (path, maximum) in self.assignments() { result.push((owner.acquire(catalog, path)?, maximum)); }
        Ok(result)
    }
    pub fn load(catalog: &Catalog, character: &str) -> Result<Self, String> {
        let entity = format!("models/characters/player/{character}/body.def");
        let component = entities::player::entity_component(&entity, "cpntinventory", |path| {
            Ok(catalog.read(path).map_err(|error| error.to_string())?
                .map(entities::owned_declaration_text))
        })?.ok_or_else(|| format!("{entity} has no inventory component"))?;
        let name = atom(&component, "m_collection").ok_or("inventory collection is absent")?;
        let sources = PickableSourceLease::for_catalog(catalog)?;
        let declaration = sources.0.lock().map_err(|_| "inventory declaration owner is poisoned")?.configurations.collection(catalog, name)?;
        Ok(Self { declaration })
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
pub fn item_fields(catalog: &Catalog, item: &str) -> Result<ItemFields, String> {
    let sources = PickableSourceLease::for_catalog(catalog)?;
    let fields = sources.0.lock().map_err(|_| "inventory source owner is poisoned")?.read_fields(catalog, item)?;
    Ok(fields)
}

pub fn readable_fields(fields: &[(String, Value)]) -> Result<Option<Readable>, String> {
    // Constructor 14063c510 initializes m_noteType to -1 and
    // m_readableCanBePickedUp to true.
    let kind = match atom(fields, "m_type") {
        Some("TYPE_NOTE_DOCUMENT" | "12") => ReadableKind::Document,
        Some("TYPE_BOOK" | "25") => ReadableKind::Book,
        _ => return Ok(None),
    };
    let note_type = atom(fields, "m_noteType").map(note_type).transpose()?.unwrap_or(-1);
    // 140844bf0 rejects an invalid note style except for books and maps.
    if kind == ReadableKind::Document && note_type == -1 { return Ok(None); }
    readable_presentation(fields, kind, note_type).map(Some)
}

/// The read interaction accepts any item with an authored note style, not
/// only journal document/book types. Maps without map info fall through to
/// the same note presentation even with the default style (1408450b0).
pub fn other_readable_fields(fields: &[(String, Value)]) -> Result<Option<(Readable, &str)>, String> {
    let item_type = atom(fields, "m_type").unwrap_or("");
    if matches!(item_type, "TYPE_NOTE_DOCUMENT" | "12" | "TYPE_BOOK" | "25") { return Ok(None); }
    let style = atom(fields, "m_noteType").map(note_type).transpose()?.unwrap_or(-1);
    if style == -1 && !matches!(item_type, "TYPE_INTERACTIVE_MAP" | "24") { return Ok(None); }
    Ok(Some((readable_presentation(fields, ReadableKind::Document, style)?, item_type)))
}

fn readable_presentation(fields: &[(String, Value)], kind: ReadableKind, note_type: i32) -> Result<Readable, String> {
    let can_be_picked_up = match atom(fields, "m_readableCanBePickedUp").unwrap_or("true") {
        "true" => true,
        "false" => false,
        other => return Err(format!("invalid m_readableCanBePickedUp {other}")),
    };
    Ok(Readable {
        kind,
        title: atom(fields, "m_name").unwrap_or("").to_owned(),
        text: atom(fields, "m_description").unwrap_or("").to_owned(),
        note_type,
        can_be_picked_up,
        journal_image: atom(fields, "m_journalImage").filter(|image| !image.is_empty()).map(str::to_owned),
    })
}
