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
    /// 836120/8362c0 select the exact request's fast/ordinary pointer, with
    /// an optional authored sneak pointer. Skipping is a request flag, not
    /// a guessed zero-duration scene. Missing required pointers are errors.
    pub fn equip_scene(&self, flags: u32, sneaking: bool) -> Result<Option<&str>, String> {
        self.switch_scene(flags, sneaking, true)
    }
    pub fn unequip_scene(&self, flags: u32, sneaking: bool) -> Result<Option<&str>, String> {
        self.switch_scene(flags, sneaking, false)
    }
    fn switch_scene(&self, flags: u32, sneaking: bool, equipping: bool) -> Result<Option<&str>, String> {
        let (skip, fast, normal, quick, sneak, sneak_quick) = if equipping {
            (0x10, 0x40, "m_equipMidnight", "m_equipFastMidnight", "m_sneakEquipMidnight", "m_sneakEquipFastMidnight")
        } else { (0x20, 0x80, "m_unequipMidnight", "m_unequipFastMidnight", "m_sneakUnequipMidnight", "m_sneakUnequipFastMidnight") };
        if flags & skip != 0 { return Ok(None); }
        let (normal, sneak) = if flags & fast != 0 { (quick, sneak_quick) } else { (normal, sneak) };
        if sneaking {
            if let Some(scene) = self.scenes.get(sneak) { return Ok(Some(scene)); }
        }
        self.scenes.get(normal).map(|scene| Some(scene.as_str())).ok_or_else(|| format!("equipment has no required {normal}"))
    }
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
    pub family: RangedFamily,
    pub equipment: Equipment,
    pub ammunition: Vec<Ammunition>,
    pub recoil_alias: Option<String>,
    pub recoil_weight_attribute: Option<String>,
    pub fire_playrate_attribute: Option<String>,
    pub reload_playrate_attribute: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangedFamily { Gun, Crossbow }
impl Ranged {
    /// Gun44e720 rounds the actual capacity attribute; bolts retain the
    /// ranged constructor's one-round capacity. 14137b1c0 registers this tag.
    pub fn capacity(&self, ammunition: usize, attributes: &super::states::Attributes) -> i32 {
        if self.family == RangedFamily::Crossbow || !self.ammunition[ammunition].fits_magazine { return 1; }
        attributes.0.get("ARK_ATTRIBUTE_GUN_AMMO_CAPACITY").map_or(0, |value| (*value + 0.5).floor() as i32)
    }
    pub fn read(fields: &Fields, equipment: Equipment, family: RangedFamily) -> Result<Self, String> {
        let mut ammunition = Vec::new();
        for entry in items(child(fields, "m_ammoTypes")) {
            let Some(item) = text(entry,"m_ammoItem") else { return Err("ranged ammunition has no inventory item".into()) };
            ammunition.push(Ammunition { item, projectile: text(entry,"m_projectileDef"), attachment: text(entry,"m_attachmentDef"),
                fits_magazine: boolean(entry,"m_fitInMagazine",false)? });
        }
        Ok(Self { family, equipment, ammunition, recoil_alias: text(fields,"m_recoilAnim"), recoil_weight_attribute: text(fields,"m_recoilWeightAttribute"),
            fire_playrate_attribute: text(fields,"m_firePlayrateAttribute"), reload_playrate_attribute: text(fields,"m_reloadPlayrateAttribute") })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action { Fire, Charge, Reload, OutOfAmmo }

/// Inventory quantities include rounds currently loaded. 639360 consumes one
/// inventory item when its independent magazine-consumption message arrives;
/// 63a2b0 reloads from the total without consuming inventory a second time.
pub trait AmmunitionInventory {
    fn quantity(&self, item: &str) -> u32;
    fn consume(&mut self, item: &str, count: u32) -> u32;
}

#[derive(Clone, Debug, Default)]
pub struct FireParameters {
    pub shoot_on_aiming_target: bool,
    pub attack_tag: Option<String>,
    pub charged_shot: bool,
}

#[derive(Clone, Debug)]
pub enum WeaponEffect {
    /// The projectile owner receives the actual selected declaration and event
    /// properties. This is not replaced with a fabricated hitscan attack.
    Fire { ammunition: usize, projectile: Option<String>, parameters: FireParameters },
    MagazineChanged { previous: i32, current: i32 },
    AnimateUpgradeForFire,
    AnimateUpgradeForReload,
    AnimateMagazineForFire,
    ShowLoadedAttachment,
    Targeting { enabled: bool },
}

/// The selected ranged component's persistent state (637580, 639360, 63a2b0).
/// Scene ownership remains in the player's Midnight manager. No clip or model
/// acquisition occurs here, and reload/fire timing comes from actual keys.
pub struct Runtime {
    pub settings: std::sync::Arc<Ranged>,
    pub magazine: i32,
    pub capacity: i32,
    pub total_ammo: u32,
    /// The upgrade flags at ranged+0xb8: burst=1, targeting=2, charge=4.
    pub target_flags: u32,
    pub targeting_requested: bool,
    pub reload_requested: bool,
    pub charged_shot: bool,
    selected_ammunition: usize,
    active_scene: u32,
    equipment_target: usize,
    effects: Vec<WeaponEffect>,
    pending_fire: Vec<FireParameters>,
}

impl Runtime {
    /// Capacity is supplied by the actual subclass/attribute owner; the
    /// generic constructor's default is one (637580+0x67).
    pub fn new(settings: std::sync::Arc<Ranged>, capacity: i32, total_ammo: u32, equipment_target: usize) -> Result<Self, String> {
        if settings.ammunition.is_empty() { return Err("ranged equipment has no ammunition declarations".into()); }
        if !(3..=4).contains(&equipment_target) { return Err("ranged equipment has no player equipment cast".into()); }
        if capacity < 0 { return Err("negative ranged magazine capacity".into()); }
        let mut value = Self { settings, magazine: 0, capacity, total_ammo, target_flags: 0, selected_ammunition: 0,
            active_scene: 0, equipment_target, effects: Vec::new(), pending_fire: Vec::new(), targeting_requested: false, reload_requested: false, charged_shot: false };
        value.reload_magazine();
        Ok(value)
    }

    pub fn ammunition(&self) -> &Ammunition { &self.settings.ammunition[self.selected_ammunition] }
    pub fn refresh_attributes(&mut self, attributes: &super::states::Attributes) {
        self.capacity = self.settings.capacity(self.selected_ammunition, attributes);
    }
    pub fn scene_handle(&self) -> u32 { self.active_scene }
    pub fn scene_playrate(&self, action: Action, attributes: &super::states::Attributes) -> f32 {
        let tag = match action {
            Action::Fire => self.settings.fire_playrate_attribute.as_deref(),
            Action::Reload => self.settings.reload_playrate_attribute.as_deref(),
            _ => None,
        };
        // 63bc00 / 63bd90 leave the incoming rate 1 when the tag is empty;
        // an authored unresolved tag explicitly writes zero.
        tag.map_or(1.0, |tag| attributes.get(tag))
    }
    pub fn set_scene(&mut self, handle: u32) { self.active_scene = handle; }
    pub fn finish_scene(&mut self, handle: u32) { if self.active_scene == handle { self.active_scene = 0; } }
    pub fn deactivate(&mut self) {
        if self.targeting_requested { self.end_targeting(); }
        self.pending_fire.clear();
        self.active_scene = 0;
        self.reload_requested = false;
    }
    pub fn can_fire(&self) -> bool { self.magazine > 0 }
    pub fn can_reload(&self) -> bool { self.magazine < self.capacity && i64::from(self.magazine) < i64::from(self.total_ammo) }
    pub fn update_inventory(&mut self, total: u32) { self.total_ammo = total; }
    pub fn request_reload(&mut self) { if self.can_reload() { self.reload_requested = true; } }
    /// 63bfc0 keeps a pending explicit reload until it succeeds, and requests
    /// an automatic reload when the current magazine is empty.
    pub fn take_reload_request(&mut self) -> bool {
        if !self.can_reload() { self.reload_requested = false; return false; }
        self.reload_requested || self.magazine == 0
    }
    pub fn begin_targeting(&mut self) { self.targeting_requested = true; self.effects.push(WeaponEffect::Targeting { enabled: true }); }
    pub fn end_targeting(&mut self) { self.targeting_requested = false; self.effects.push(WeaponEffect::Targeting { enabled: false }); }
    pub fn set_capacity(&mut self, capacity: i32) -> Result<(), String> {
        if capacity < 0 { return Err("negative ranged magazine capacity".into()); }
        self.capacity = capacity;
        Ok(())
    }

    /// 63a490 selects a matching authored ammo record, clears the old magazine
    /// and runs the subclass reload callback. The caller supplies its updated
    /// capacity because upgrades and fit-in-magazine rules live there.
    pub fn select_ammunition(&mut self, item: &str, capacity: i32, inventory: &impl AmmunitionInventory) -> Result<bool, String> {
        let Some(index) = self.settings.ammunition.iter().position(|ammo| ammo.item == item) else { return Ok(false) };
        self.set_capacity(capacity)?;
        self.clear_magazine();
        self.selected_ammunition = index;
        self.total_ammo = inventory.quantity(item);
        self.reload_magazine();
        Ok(true)
    }

    /// 63bc00/63bd40/63bd90 choose a non-null authored slide variant, falling
    /// back to the ordinary pointer. Missing ordinary pointers remain absent.
    pub fn scene(&self, action: Action, sliding: bool) -> Option<&str> {
        let key = match action { Action::Fire => "m_fireMidnight", Action::Charge => "m_chargeMidnight", Action::Reload => "m_reloadMidnight", Action::OutOfAmmo => "m_outOfAmmoMidnight" };
        if action == Action::Charge { return self.settings.equipment.scenes.get(key).map(String::as_str); }
        if sliding {
            let slide = match action { Action::Fire => "m_fireMidnightSlide", Action::Reload => "m_reloadMidnightSlide", Action::OutOfAmmo => "m_outOfAmmoMidnightSlide", Action::Charge => unreachable!() };
            if let Some(scene) = self.settings.equipment.scenes.get(slide) { return Some(scene); }
        }
        self.settings.equipment.scenes.get(key).map(String::as_str)
    }

    pub fn take_effects(&mut self) -> Vec<WeaponEffect> { std::mem::take(&mut self.effects) }
    pub fn drain_effects(&mut self) -> std::vec::Drain<'_, WeaponEffect> { self.effects.drain(..) }

    fn change_magazine(&mut self, current: i32) {
        let previous = std::mem::replace(&mut self.magazine, current);
        self.effects.push(WeaponEffect::MagazineChanged { previous, current });
    }
    pub fn reload_magazine(&mut self) {
        self.change_magazine(i64::from(self.capacity).min(i64::from(self.total_ammo)) as i32);
    }
    pub fn emit_reload(&mut self) {
        // Gun44f440 shows its already-loaded bullet; crossbow62bfe0 invokes
        // the reload callback directly. The separate gun203 key fills it.
        match self.settings.family {
            RangedFamily::Gun => self.effects.push(WeaponEffect::ShowLoadedAttachment),
            RangedFamily::Crossbow => self.reload_magazine(),
        }
    }
    pub fn clear_magazine(&mut self) { self.change_magazine(0); }

    /// 866dc0 posts ARK_MSG_WEAPON_FIRE directly only when the weapon's
    /// selected fire-scene pointer is null. The caller performs that decision.
    pub fn emit_fire(&mut self) {
        self.pending_fire.push(FireParameters { charged_shot: self.charged_shot, ..Default::default() });
    }
    pub fn flush_pending_fire(&mut self, inventory: &mut (impl AmmunitionInventory + ?Sized)) {
        let mut pending = std::mem::take(&mut self.pending_fire);
        for parameters in pending.drain(..) { self.fire_with_inventory(parameters, inventory); }
        self.pending_fire = pending;
    }
    fn fire_with_inventory(&mut self, mut parameters: FireParameters, inventory: &mut (impl AmmunitionInventory + ?Sized)) {
        // Both 44e0c0 and 62b560 check virtual110, consume via virtual140,
        // then dispatch the selected projectile and muzzle effects.
        if !self.can_fire() { return; }
        parameters.charged_shot = self.charged_shot;
        self.change_magazine((self.magazine - 1).max(0));
        let item = self.ammunition().item.as_str();
        inventory.consume(item, 1);
        self.total_ammo = inventory.quantity(item);
        self.effects.push(WeaponEffect::Fire { ammunition: self.selected_ammunition,
            projectile: self.ammunition().projectile.clone(), parameters });
    }

    /// Native Midnight message IDs c8..cd are registered at 141a11c80.
    /// Target and live scene handle prevent a cancelled/other weapon's keys
    /// from spending this component's ammunition.
    pub fn handle_event(&mut self, event: &super::AnimationEvent, inventory: &mut (impl AmmunitionInventory + ?Sized)) -> Result<bool, String> {
        if event.target != self.equipment_target || event.handle != self.active_scene || self.active_scene == 0 { return Ok(false); }
        match event.name.as_str() {
            "ARK_MDNT_MSG_WEAPON_FIRE" => {
                self.fire_with_inventory(FireParameters {
                    shoot_on_aiming_target: boolean(&event.properties, "m_shootOnAimingTarget", false)?,
                    attack_tag: text(&event.properties, "m_attackTag"),
                    charged_shot: self.charged_shot,
                }, inventory);
            }
            "ARK_MDNT_MSG_WEAPON_CONSUME_AMMO_FROM_MAGAZINE" => {
                // Gun44f3d0 animates the upgrade magazine. Despite the name,
                // the native callback never consumes an inventory item here.
                if self.settings.family == RangedFamily::Gun && self.capacity > 1 {
                    self.effects.push(WeaponEffect::AnimateMagazineForFire);
                }
            }
            "ARK_MDNT_MSG_WEAPON_RELOAD" => self.emit_reload(),
            "ARK_MDNT_MSG_WEAPON_RELOAD_MAGAZINE" if self.settings.family == RangedFamily::Gun => {
                self.total_ammo = inventory.quantity(&self.ammunition().item);
                self.reload_magazine();
            }
            "ARK_MDNT_MSG_WEAPON_UPGRADE_ANIMATE_FOR_FIRE" => self.effects.push(WeaponEffect::AnimateUpgradeForFire),
            "ARK_MDNT_MSG_WEAPON_UPGRADE_ANIMATE_FOR_RELOAD" => self.effects.push(WeaponEffect::AnimateUpgradeForReload),
            _ => return Ok(false),
        }
        Ok(true)
    }
}
