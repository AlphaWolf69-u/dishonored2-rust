//! Equipment and ranged action declarations. Fields retain installed names;
//! 619ea0 is the equipment constructor, 1416f2eb0 its metadata, 6376c0 the
//! ranged constructor. Runtime preparation uses the normal player rig owners.
use crate::entities::{scene_motion::{atom, child, items}, Value};
use std::collections::HashMap;

type Fields = [(String, Value)];
fn text(fields: &Fields, key: &str) -> Option<String> {
    atom(fields, key).filter(|value| !value.is_empty() && *value != "NULL").map(str::to_owned)
}
fn boolean(fields: &Fields, key: &str, fallback: bool) -> Result<bool, String> {
    match atom(fields, key) {
        None => Ok(fallback), Some("true" | "1") => Ok(true), Some("false" | "0") => Ok(false),
        Some(value) => Err(format!("invalid equipment {key}={value}")),
    }
}

#[derive(Clone, Debug)]
pub struct Equipment {
    pub slot: u8,
    pub archetype: u32,
    pub equipped_bind_tag: Option<String>,
    pub unequipped_bind_tag: Option<String>,
    pub default_input: Option<String>,
    pub default_alias: Option<String>,
    pub loco_equip_state: Option<String>,
    pub usable_swimming: bool,
    pub hide_unequipped: bool,
    pub can_lower: bool,
    pub force_attachment: bool,
    pub auto_equip: bool,
    /// Exact component field name -> scene declaration. No guessed fallback
    /// between posture/fast variants is performed by this resource reader.
    pub scenes: HashMap<String, String>,
}
impl Equipment {
    /// Class constructors supply the defaults: 633c70 melee=(1,0),
    /// 6376c0 ranged=(subclass archetype,1); gun44dae0/crossbow62b1b0 use 2.
    /// Serialized enum overrides are resolved with the engine enum owner.
    pub fn read(fields: &Fields, default_archetype: u32, default_slot: u8,
        mut enumeration: impl FnMut(&str, &str) -> Option<i32>) -> Result<Self, String> {
        let mut value = |name: &str, kind: &str, fallback: i32| -> Result<i32, String> {
            atom(fields, name).map_or(Ok(fallback), |text| enumeration(kind, text).or_else(|| text.parse().ok())
                .ok_or_else(|| format!("unknown {kind}: {text}")))
        };
        let slot = value("m_slot", "arkEquipmentSlot_t", default_slot as i32)?;
        let archetype = value("m_archetype", "arkEquipmentArchetype_t", default_archetype as i32)?;
        if !(0..16).contains(&slot) || archetype <= 0 { return Err("equipment has no admissible inventory slot/archetype".into()); }
        let mut scenes = HashMap::new();
        // These are authored pointers, not bulk scene acquisition. The host
        // demands only the action admitted by its equipment state owner.
        for (key, value) in fields {
            if key.ends_with("Midnight") || key.ends_with("MidnightSlide") {
                if let Value::Atom(path) = value {
                    let path = path.trim_matches('"');
                    if path.ends_with(".midnightscene") { scenes.insert(key.clone(), path.to_owned()); }
                }
            }
        }
        Ok(Self { slot: slot as u8, archetype: archetype as u32,
            equipped_bind_tag: text(fields,"m_equippedBindTag"), unequipped_bind_tag: text(fields,"m_unequippedBindTag"),
            default_input: text(fields,"m_defaultAnimInputName"), default_alias: text(fields,"m_defaultAnimAliasName"),
            loco_equip_state: text(fields,"m_locoEquipStateName"), usable_swimming: boolean(fields,"m_usableWhenSwimming",false)?,
            hide_unequipped: boolean(fields,"m_hideWhenNotEquipped",false)?,
            can_lower: boolean(fields,"m_canLower",false)?, force_attachment: boolean(fields,"m_forceAttachment",false)?,
            auto_equip: boolean(fields,"m_autoEquip",false)?, scenes })
    }
}

#[derive(Clone, Debug)]
pub struct Ammunition {
    pub item: String,
    pub projectile: Option<String>,
    pub attachment: Option<String>,
    pub fits_magazine: bool,
}
#[derive(Clone, Debug)]
pub struct Ranged {
    pub equipment: Equipment,
    pub ammunition: Vec<Ammunition>,
    pub recoil_alias: Option<String>,
    pub recoil_weight_attribute: Option<String>,
}
impl Ranged {
    pub fn read(fields: &Fields, equipment: Equipment) -> Result<Self, String> {
        let mut ammunition = Vec::new();
        for entry in items(child(fields, "m_ammoTypes")) {
            let Some(item) = text(entry,"m_ammoItem") else { return Err("ranged ammunition has no inventory item".into()) };
            ammunition.push(Ammunition { item, projectile: text(entry,"m_projectileDef"), attachment: text(entry,"m_attachmentDef"),
                fits_magazine: boolean(entry,"m_fitInMagazine",false)? });
        }
        Ok(Self { equipment, ammunition, recoil_alias: text(fields,"m_recoilAnim"), recoil_weight_attribute: text(fields,"m_recoilWeightAttribute") })
    }
}
