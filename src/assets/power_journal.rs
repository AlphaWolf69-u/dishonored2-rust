//! Authored power and upgrade metadata used by the journal.
//! Item ownership remains with the shared inventory; these definitions are
//! loaded only while preparing the journal.
use std::collections::HashMap;
use crate::assets::{archive::Catalog, entities::{self, Value}};
use crate::gameplay::options::decl::{atom, enum_key, list_items, scope};

#[derive(Clone, Debug)]
pub struct Upgrade {
    pub id: String,
    pub item: Option<String>,
    pub children: Vec<Upgrade>,
    pub info: Option<UpgradeInfo>,
}
#[derive(Clone, Debug)]
pub struct UpgradeInfo {
    pub name: String,
    pub description: String,
    pub tips: String,
    pub image: Option<String>,
    pub rune_cost: i32,
}
#[derive(Clone, Debug)]
pub struct Power {
    /// Resolved equipment component, shared by casting and the Journal.
    pub component: Vec<(String, Value)>,
    pub item: String,
    pub name: String,
    pub description: String,
    pub tips: String,
    pub image: Option<String>,
    pub header_image: Option<String>,
    pub rune_cost: i32,
    pub power_id: String,
    pub hud_index: i32,
    pub usable_by: String,
    pub upgrades: Vec<Upgrade>,
}

/// 141a15350: arkPowerUpgradeId_t registration.
pub fn upgrade_id(name: &str) -> Option<i32> {
    Some(match name {
        "ARK_POWERUPGRADE_NONE" => 0,
        "ARK_POWERUPGRADE_SHADOWWALK_RUN" => 1,
        "ARK_POWERUPGRADE_SHADOWWALK_ASSASSINATION_1" => 2,
        "ARK_POWERUPGRADE_SHADOWWALK_ASSASSINATION_2" => 3,
        "ARK_POWERUPGRADE_SHADOWWALK_RAT_SHADOW" => 4,
        "ARK_POWERUPGRADE_DARKVISION_GREATER" => 5,
        "ARK_POWERUPGRADE_DARKVISION_PREMONITION" => 6,
        "ARK_POWERUPGRADE_MESMERIZE_THREE" => 7,
        "ARK_POWERUPGRADE_MESMERIZE_FOUR" => 8,
        "ARK_POWERUPGRADE_MESMERIZE_EXTENDEDDURATION" => 9,
        "ARK_POWERUPGRADE_MESMERIZE_COMBAT" => 10,
        "ARK_POWERUPGRADE_MESMERIZE_ANIMAL" => 11,
        "ARK_POWERUPGRADE_BENDTIME_STOPTIME" => 12,
        "ARK_POWERUPGRADE_BENDTIME_EXTENDEDDURATION" => 13,
        "ARK_POWERUPGRADE_BENDTIME_RELATIVITY" => 14,
        "ARK_POWERUPGRADE_BLINK_BLINKATTACK" => 15,
        "ARK_POWERUPGRADE_BLINK_IMPROVEDDISTANCE" => 16,
        "ARK_POWERUPGRADE_BLINK_TRANSVERSALTSLOWTIME" => 17,
        "ARK_POWERUPGRADE_BLINK_ASSAULT" => 18,
        "ARK_POWERUPGRADE_FARREACH_PULL_OBJECTS" => 19,
        "ARK_POWERUPGRADE_FARREACH_PULL_ENEMIES" => 20,
        "ARK_POWERUPGRADE_FARREACH_PULL_ATTACK" => 21,
        "ARK_POWERUPGRADE_FARREACH_DECELERATE" => 22,
        "ARK_POWERUPGRADE_POSSESSION_CHAIN_HOST" => 23,
        "ARK_POWERUPGRADE_POSSESSION_EXTENDEDDURATION" => 24,
        "ARK_POWERUPGRADE_POSSESSION_PARASITE" => 25,
        "ARK_POWERUPGRADE_POSSESSION_DECAYEDHARBOR" => 26,
        "ARK_POWERUPGRADE_WINDBLAST_GREATER" => 27,
        "ARK_POWERUPGRADE_WINDBLAST_SHOCKWAVE" => 28,
        "ARK_POWERUPGRADE_DEVOURINGSWARM_GREATER" => 29,
        "ARK_POWERUPGRADE_DEVOURINGSWARM_VERMINCOMPANIONS" => 30,
        "ARK_POWERUPGRADE_DEVOURINGSWARM_TWINSWARMS" => 31,
        "ARK_POWERUPGRADE_CLONE_BAFFLINGEXIT" => 32,
        "ARK_POWERUPGRADE_CLONE_SWAPPLACE" => 33,
        "ARK_POWERUPGRADE_CLONE_TWIN_SHADE" => 34,
        "ARK_POWERUPGRADE_CLONE_DEADLYSHADOW" => 35,
        "ARK_POWERUPGRADE_DOMINO_LINK_THREE" => 36,
        "ARK_POWERUPGRADE_DOMINO_LINK_FOUR" => 37,
        "ARK_POWERUPGRADE_MAX" => 38,
        _ => return None,
    })
}

fn ctor_power(kind: &str) -> Option<&'static str> {
    // Component constructors set arkPlayerPower_t at decl +0x190.
    Some(match kind {
        "cpntpowerbendtime" => "ARK_PLAYER_POWER_BENDTIME", // 14061c410: 1
        "cpntpowerblink" => "ARK_PLAYER_POWER_BLINK", // 14061d660: 2
        "cpntpowershadowwalk" => "ARK_PLAYER_POWER_SHADOWWALK", // 140621770: 3
        "cpntpowermesmerize" => "ARK_PLAYER_POWER_MESMERIZE", // 140621210: 4
        "cpntpowerdoppelganger" => "ARK_PLAYER_POWER_DOPPELGANGER", // 14043dd70: 5
        "cpntpowergrapple" => "ARK_PLAYER_POWER_GRAPPLE", // 14043e920: 6
        "cpntpowerpossession" => "ARK_PLAYER_POWER_POSSESSION", // 140447ad0: 7
        "cpntpowerdarkvision" => "ARK_PLAYER_POWER_DARK_VISION", // 14043d2d0: 9
        "cpntpowerwindblast" => "ARK_PLAYER_POWER_WINDBLAST", // 1404484a0: 10
        "cpntpowerdevouringswarm" => "ARK_PLAYER_POWER_DEVOURING_SWARM", // 14043d690: 11
        "cpntpowerphantasmallink" => "ARK_PLAYER_POWER_PHANTASMAL_LINK", // 140446f20: 12
        _ => return None,
    })
}
/// 1419fdb60: arkHUDElementItem_t values used by the power movie.
pub(crate) fn hud_index(name: &str) -> Option<i32> {
    Some(match name {
        "ARK_HUD_ELEMENT_ITEM_REGULAR_BOLT" => 1, "ARK_HUD_ELEMENT_ITEM_SLEEP_DART" => 2,
        "ARK_HUD_ELEMENT_ITEM_FLAME_BOLT" => 3, "ARK_HUD_ELEMENT_ITEM_HOWLING_BOLT" => 4,
        "ARK_HUD_ELEMENT_ITEM_BURNING_VENOM_DART" => 5, "ARK_HUD_ELEMENT_ITEM_ARC_MINE_STUN" => 6,
        "ARK_HUD_ELEMENT_ITEM_SPRINGRAZOR" => 7, "ARK_HUD_ELEMENT_ITEM_REGULAR_GRENADE" => 8,
        "ARK_HUD_ELEMENT_ITEM_STICKY_GRENADE" => 9, "ARK_HUD_ELEMENT_ITEM_REGULAR_BULLETS" => 10,
        "ARK_HUD_ELEMENT_ITEM_EXPLOSIVE_BULLETS" => 11, "ARK_HUD_ELEMENT_ITEM_HEART" => 12,
        "ARK_HUD_ELEMENT_ITEM_BEND_TIME" => 14, "ARK_HUD_ELEMENT_ITEM_BLINK" => 15,
        "ARK_HUD_ELEMENT_ITEM_DARK_VISION" => 16, "ARK_HUD_ELEMENT_ITEM_DOPPELGANGER" => 17,
        "ARK_HUD_ELEMENT_ITEM_MESMERIZE" => 18, "ARK_HUD_ELEMENT_ITEM_POSSESSION" => 19,
        "ARK_HUD_ELEMENT_ITEM_RATS_SWARM" => 20, "ARK_HUD_ELEMENT_ITEM_SHADOW_WALK" => 21,
        "ARK_HUD_ELEMENT_ITEM_WIND_BLAST" => 22, "ARK_HUD_ELEMENT_ITEM_FAR_REACH" => 23,
        "ARK_HUD_ELEMENT_ITEM_PHANTASMAL_LINK" => 24, "ARK_HUD_ELEMENT_ITEM_TIMESHIFT_DEVICE" => 13,
        "ARK_HUD_ELEMENT_ITEM_EMPTY" => 0, "ARK_HUD_ELEMENT_ITEM_INVALID" => -1,
        _ => return name.parse().ok(),
    })
}
pub fn power_id(name: &str) -> Option<i32> {
    Some(match name {
        "ARK_PLAYER_POWER_NONE" => 0, "ARK_PLAYER_POWER_BENDTIME" => 1, "ARK_PLAYER_POWER_BLINK" => 2,
        "ARK_PLAYER_POWER_SHADOWWALK" => 3, "ARK_PLAYER_POWER_MESMERIZE" => 4,
        "ARK_PLAYER_POWER_DOPPELGANGER" => 5, "ARK_PLAYER_POWER_GRAPPLE" => 6,
        "ARK_PLAYER_POWER_POSSESSION" => 7, "ARK_PLAYER_POWER_TIMETRAVEL" => 8,
        "ARK_PLAYER_POWER_DARK_VISION" => 9, "ARK_PLAYER_POWER_WINDBLAST" => 10,
        "ARK_PLAYER_POWER_DEVOURING_SWARM" => 11, "ARK_PLAYER_POWER_PHANTASMAL_LINK" => 12,
        "ARK_PLAYER_POWER_DOPPELGANGER_SWAP" => 13, "ARK_PLAYER_POWER_ALL" => 14,
        _ => return None,
    })
}
fn value_text(fields: &[(String, Value)], key: &str) -> String { atom(fields, key).unwrap_or("").to_owned() }
fn image(fields: &[(String, Value)], key: &str) -> Option<String> {
    atom(fields, key).filter(|v| !v.is_empty() && *v != "NULL").map(str::to_owned)
}
fn upgrades(catalog: &Catalog, fields: &[(String, Value)], mapping: &HashMap<String, String>) -> Result<Vec<Upgrade>, String> {
    let mut out = Vec::new();
    for value in list_items(fields) {
        let Value::Scope(entry) = value else { continue };
        let Some(id) = atom(entry, "m_upgradeId").map(str::to_owned) else { continue };
        let item = mapping.get(&id).cloned();
        let info = if let Some(item) = &item {
            let fields = crate::assets::inventory_item::item_fields(catalog, item)?;
            Some(UpgradeInfo { name: value_text(&fields, "m_name"), description: value_text(&fields, "m_description"),
                tips: value_text(&fields, "m_strategicTips"), image: image(&fields, "m_journalImage"),
                rune_cost: atom(&fields, "m_runeCost").and_then(|v| v.parse().ok()).unwrap_or(1) })
        } else { None };
        let children = match scope(entry, "m_subUpgrades") { Some(children) => upgrades(catalog, children, mapping)?, None => Vec::new() };
        out.push(Upgrade { id, item, info, children });
    }
    Ok(out)
}

/// Resolve the selected character's power-system upgrade topology once per
/// journal presentation; quantity and modifier state are queried separately.
pub fn definitions<'a>(catalog: &Catalog, character: &str, items: impl IntoIterator<Item = &'a str>) -> Result<(Vec<Power>, String), String> {
    let read = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(entities::owned_declaration_text))
    };
    let body = format!("models/characters/player/{character}/body.def");
    let system = entities::player::entity_component(&body, "cpntplayerpowersystem", read)?
        .ok_or("player has no power-system component")?;
    definitions_from_system(catalog, &system, items)
}

/// Use the level's already resolved player component rather than acquiring
/// the complete player entity again for a UI or casting consumer.
pub fn definitions_from_system<'a>(catalog: &Catalog, system: &[(String, Value)], items: impl IntoIterator<Item = &'a str>) -> Result<(Vec<Power>, String), String> {
    let read = |path: &str| -> Result<Option<String>, String> {
        Ok(catalog.read(path).map_err(|e| e.to_string())?.map(entities::owned_declaration_text))
    };
    let mut mapping = HashMap::new();
    if let Some(entries) = scope(&system, "m_powerUpgradeItemList") {
        for (key, value) in entries {
            if let (Some(id), Value::Scope(fields)) = (enum_key(key), value) {
                if let Some(item) = atom(fields, "m_item") { mapping.insert(id.to_owned(), item.to_owned()); }
            }
        }
    }
    let configured = scope(&system, "m_powers").map(list_items).unwrap_or_default();
    let rune = atom(&system, "m_rune").unwrap_or("").to_owned();
    let mut result = Vec::new();
    for item in items {
        let fields = crate::assets::inventory_item::item_fields(catalog, item)?;
        if atom(&fields, "m_type") != Some("TYPE_POWER") { continue; }
        let Some(entity) = atom(&fields, "m_equipment") else { continue };
        let components = entities::player::entity_components_with_prefix(entity, "cpntpower", read)?;
        let Some((kind, component)) = components.iter().find(|(kind, _)| ctor_power(kind).is_some()) else { continue };
        let power_id = atom(component, "m_powerId").or_else(|| ctor_power(kind)).unwrap_or("");
        let Some(configuration) = configured.iter().find_map(|value| match value {
            Value::Scope(entry) if atom(entry, "m_id") == Some(power_id) => Some(entry.as_slice()), _ => None,
        }) else { continue };
        result.push(Power {
            component: component.clone(),
            item: item.to_owned(), name: value_text(&fields, "m_name"), description: value_text(&fields, "m_description"),
            tips: value_text(&fields, "m_strategicTips"), image: image(&fields, "m_journalImage"),
            header_image: image(&fields, "m_journalHeaderImage"),
            rune_cost: atom(&fields, "m_runeCost").and_then(|v| v.parse().ok()).unwrap_or(1),
            power_id: power_id.to_owned(),
            hud_index: match atom(&fields, "m_uiIndex") { Some(value) => hud_index(value).ok_or_else(|| format!("unknown power HUD item index {value}"))?, None => 0 },
            usable_by: value_text(component, "m_canBeUsedBy"),
            upgrades: match scope(configuration, "m_upgrades") { Some(tree) => upgrades(catalog, tree, &mapping)?, None => Vec::new() },
        });
    }
    Ok((result, rune))
}
