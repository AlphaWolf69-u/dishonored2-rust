//! Authored charm identities and spawn eligibility (610700, 60fad0).
use crate::{archive::Catalog, assets::entities::{self,Value}, gameplay::options::decl::{atom, scope, list_items}};
use std::{collections::HashMap, sync::Arc};

type Fields = [(String, Value)];

fn kind(value: &str) -> Result<i32, String> {
    match value {
        "BONE_CHARM_ANY" => Ok(-1),
        "BONE_CHARM" => Ok(0), "BLACK_BONE_CHARM" => Ok(1), "CRACKED_BONE_CHARM" => Ok(2),
        "CRAFTED_BONE_CHARM" => Ok(3), "CRACKED_CRAFTED_BONE_CHARM" => Ok(4),
        _ => value.parse().map_err(|_| format!("invalid bone charm type {value}")),
    }
}

#[derive(Debug)]
pub(crate) struct Placement { fixed: Option<String>, force: i32 }
impl Placement {
    pub(crate) fn read(fields: &Fields) -> Result<Self, String> {
        // 725b50 defaults to random, with no forced type.
        let fixed = if matches!(atom(fields, "m_randomBoneCharm"), Some("false" | "0")) {
            identity(fields, "m_boneCharmId").map(str::to_owned)
        } else { None };
        Ok(Self { fixed, force: kind(atom(fields, "m_forceType").unwrap_or("-1"))? })
    }
    pub(crate) fn fixed(&self) -> bool { self.fixed.is_some() }
}

fn identity<'a>(fields: &'a Fields, key: &str) -> Option<&'a str> {
    atom(fields, key).or_else(|| scope(fields, key).and_then(|id| atom(id, "m_id")))
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
}
fn rows<'a>(fields: &'a Fields, key: &str) -> impl Iterator<Item=&'a Fields> {
    scope(fields, key).map(list_items).unwrap_or_default().into_iter().filter_map(|row| match row {
        Value::Scope(fields) => Some(fields.as_slice()), _ => None,
    })
}
fn names<'a>(fields: &'a Fields, key: &str) -> impl Iterator<Item=&'a str> {
    scope(fields, key).map(list_items).unwrap_or_default().into_iter().filter_map(|row| match row {
        Value::Atom(name) => Some(name.trim_matches('"')), _ => None,
    })
}

pub(crate) struct Entry {
    pub(crate) name: Arc<str>,
    kind: i32,
    traits: Vec<usize>,
    available: bool,
    owned: bool,
}
pub(crate) struct Collection {
    entries: Vec<Entry>,
    trait_requirements: Vec<Vec<String>>,
}
impl Collection {
    pub(crate) fn load(catalog: &Catalog, character: &str) -> Result<Option<Self>, String> {
        let read = |path: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog, path);
        let player = format!("models/characters/player/{character}/body.def");
        let Some(component) = entities::player::entity_component(&player, "cpntbonecharminventory", read)? else { return Ok(None) };
        let Some(path) = atom(&component, "m_collection") else { return Ok(None) };
        let fields = entities::player::resolved_decl("bonecharmcollection", path, read)?;
        let mut traits = HashMap::new();
        let mut trait_requirements = Vec::new();
        for row in rows(&fields, "m_boneCharmTraits") {
            let Some(id) = atom(row, "m_id") else { continue };
            traits.insert(id, trait_requirements.len());
            trait_requirements.push(scope(row, "m_spawnCondition").into_iter()
                .flat_map(|condition| names(condition, "m_anyItem")).map(str::to_owned).collect());
        }
        let mut entries = Vec::new();
        for row in rows(&fields, "m_boneCharmList") {
            let Some(name) = identity(row, "m_id") else { continue };
            entries.push(Entry {
                name: Arc::from(name), kind: kind(atom(row, "m_type").unwrap_or("BONE_CHARM"))?,
                traits: names(row, "m_traits").map(|id| traits.get(id).copied().unwrap_or(usize::MAX)).collect(),
                available: true, owned: false,
            });
        }
        // 60e8b0 applies explicit player loadout overrides to the collection.
        if let Some(path) = atom(&component, "m_loadout").filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL")) {
            let loadout = entities::player::resolved_decl("bonecharmloadout", path, read)?;
            for row in rows(&loadout, "m_boneCharmList") {
                let Some(name) = identity(row, "m_boneCharmName") else { continue };
                if let Some(entry) = entries.iter_mut().find(|entry| entry.name.eq_ignore_ascii_case(name)) {
                    if matches!(atom(row, "m_addToInventory"), Some("true" | "1")) { entry.owned = true; }
                    if matches!(atom(row, "m_isAvailable"), Some("false" | "0")) { entry.available = false; }
                }
            }
        }
        Ok(Some(Self { entries, trait_requirements }))
    }
    pub(crate) fn name(&self, index: usize) -> Option<&Arc<str>> { self.entries.get(index).map(|entry| &entry.name) }
    /// Reserve fixed IDs first (725c10). 725d30 calls the random selector for
    /// random placements and fixed placements whose ID is empty.
    pub(crate) fn assign(&self, placement: &Placement, reserved: impl Fn(&str)->bool,
        owned: impl Fn(&str)->bool, previously_owned: impl Fn(&str)->bool, quantity: impl Fn(&str)->u32) -> Option<usize> {
        let index = if let Some(fixed) = &placement.fixed {
            self.entries.iter().position(|entry| entry.name.eq_ignore_ascii_case(fixed))?
        } else {
            let mut candidates = Vec::new();
            for (i, entry) in self.entries.iter().enumerate() {
                if reserved(&entry.name) || entry.owned || owned(&entry.name) || !entry.available
                    || (placement.force != -1 && entry.kind != placement.force) { continue; }
                let eligible = entry.traits.iter().all(|&t| self.trait_requirements.get(t)
                    .is_some_and(|items| items.is_empty() || items.iter().any(|path| quantity(path) > 0)));
                let priority = if !eligible { 0 } else if previously_owned(&entry.name) { 1 } else { 2 };
                candidates.push((i, priority));
            }
            let priority = candidates.iter().map(|(_, priority)| *priority).max()?;
            // 154940 returns the FILETIME tick count. 60fad0 temporarily
            // seeds the game RNG with its low word and restores the outer RNG.
            let time = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
            let mut seed = (time.as_nanos() / 100 + 116444736000000000) as u32;
            for i in (1..candidates.len()).rev() {
                seed = seed.wrapping_mul(0x19660d).wrapping_add(0x3c6ef35f);
                candidates.swap(i, ((seed >> 10) as usize & 0x7fff) % (i + 1));
            }
            candidates.into_iter().find(|(_, value)| *value == priority)?.0
        };
        Some(index)
    }
}
