//! Equipment and ranged action declarations. Fields retain installed names;
//! 619ea0 is the equipment constructor, 1416f2eb0 its metadata, 6376c0 the
//! ranged constructor. Runtime preparation uses the normal player rig owners.
use crate::entities::{scene_motion::{atom, child, items}, Value};
use std::collections::HashMap;
pub mod shooting;
pub mod heart;
pub mod throwable;

/// Decl aimSettings (6376c0; reflection141700bc0). Retained once with the
/// weapon, so firing and Think do not revisit the generic declaration tree.
#[derive(Clone, Debug)]
pub struct AimSettings {
    pub ideal_distance: f32,
    pub max_deviation_angle: f32,
    pub minimum_dispersion: f32,
    pub maximum_dispersion: f32,
    pub increase: [f32; 3],
    pub decrease: [f32; 3],
}
impl AimSettings {
    fn read(fields: &Fields) -> Result<Self, String> {
        use crate::gameplay::damage::number;
        let smooth = |name| {
            let fields = child(fields, name);
            Ok::<_, String>([number(fields, "m_acceleration", 160.0)?,
                number(fields, "m_deceleration", 160.0)?, number(fields, "m_maxSpeed", 14.0)?])
        };
        Ok(Self { ideal_distance: number(fields, "m_idealDistance", 50.0)?,
            max_deviation_angle: number(fields, "m_maxDeviationAngle", 15.0)?,
            minimum_dispersion: number(fields, "m_minDispersion", 0.0)?,
            maximum_dispersion: number(fields, "m_maxDispersion", 30.0)?,
            increase: smooth("m_dispersionIncrease")?, decrease: smooth("m_dispersionDecrease")? })
    }
}

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrosshairInfo {
    pub use_crosshair: bool,
    pub use_dot: bool,
    pub kind: i32,
    pub minimum_size: f32,
}

#[derive(Clone, Debug)]
pub struct Equipment {
    pub slot: u8,
    pub archetype: u32,
    pub action_state: super::states::StateId,
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
    ///619ea0 constructor;1416f2eb0 reflects this nested record at+e8.
    pub hud: CrosshairInfo,
    pub default_upgrades: Vec<String>,
    /// Exact component field name -> scene declaration. No guessed fallback
    /// between posture/fast variants is performed by this resource reader.
    pub scenes: HashMap<String, String>,
    /// 6376c0/86c6a0: the left equipment's response to the real slash type.
    pub slash_midnights: [Option<String>; 3],
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
        let action_state = atom(fields, "m_playerQuickUseState").map_or(Ok(super::states::StateId::NULL), |text|
            enumeration("arkPlayerStateId_t", text).map(super::states::StateId)
                .or_else(|| super::decls::state_id(text)).or_else(|| text.parse().ok().map(super::states::StateId))
                .ok_or_else(|| format!("unknown equipment quick-use state {text}")))?;
        if !(0..16).contains(&slot) || archetype <= 0 { return Err("equipment has no admissible inventory slot/archetype".into()); }
        let hud = child(fields, "m_hudInformation");
        let hud = CrosshairInfo {
            use_crosshair: boolean(hud, "m_useCrosshair", false)?,
            use_dot: boolean(hud, "m_useDot", true)?,
            kind: atom(hud, "m_crosshairParm").map_or(Ok(-1), |value|
                enumeration("arkUIElementUICrosshairType_t", value).or_else(|| value.parse().ok())
                    .ok_or_else(|| format!("unknown equipment crosshair type {value}")))?,
            minimum_size: crate::gameplay::damage::number(hud, "m_minimumCrosshairSize", 10.0)?,
        };
        let mut scenes = HashMap::new();
        let mut default_upgrades: Vec<_> = child(fields, "m_defaultUpgrades").iter().filter_map(|(key, value)| {
            let index: usize = key.strip_prefix("item[")?.strip_suffix(']')?.parse().ok()?;
            let Value::Atom(value) = value else { return None };
            let value = value.trim_matches('"');
            (!value.is_empty() && value != "NULL").then(|| (index, value.to_owned()))
        }).collect();
        default_upgrades.sort_by_key(|(index, _)| *index);
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
        let slash_fields = child(fields, "m_slashMidnights");
        let slash_midnights = std::array::from_fn(|index| text(slash_fields, &format!("m_slashMidnights[{index}]")));
        Ok(Self { slot: slot as u8, archetype: archetype as u32, action_state,
            equipped_bind_tag: text(fields,"m_equippedBindTag"), unequipped_bind_tag: text(fields,"m_unequippedBindTag"),
            default_input: text(fields,"m_defaultAnimInputName"), default_alias: text(fields,"m_defaultAnimAliasName"),
            loco_equip_state: text(fields,"m_locoEquipStateName"), usable_swimming: boolean(fields,"m_usableWhenSwimming",false)?,
            hide_unequipped: boolean(fields,"m_hideWhenNotEquipped",false)?,
            can_lower: boolean(fields,"m_canLower",false)?, force_attachment: boolean(fields,"m_forceAttachment",false)?,
            auto_equip: boolean(fields,"m_autoEquip",false)?, hud, default_upgrades: default_upgrades.into_iter().map(|(_, value)| value).collect(), scenes, slash_midnights })
    }
}

#[derive(Clone, Debug)]
pub struct Ammunition {
    pub item: String,
    pub projectile: Option<String>,
    pub attachment: Option<String>,
    pub fits_magazine: bool,
    pub muzzle_world_fx: Option<String>,
    pub damage_increment_attribute: Option<String>,
    pub bypass_big_hit_immunity_attribute: Option<String>,
    pub gravity_multiplier_attribute: Option<String>,
    pub sleep_timer_multiplier_attribute: Option<String>,
    pub spread_damage: bool,
}
///Common ranged6376c0 ammunition list, also inherited by WeaponThrowable.
///One typed source projection; no Gun/Crossbow substitute is needed.
pub(super) fn read_ammunition(fields: &Fields) -> Result<Vec<Ammunition>, String> {
    let mut ammunition = Vec::new();
    for entry in items(child(fields, "m_ammoTypes")) {
        let Some(item) = text(entry,"m_ammoItem") else { return Err("ranged ammunition has no inventory item".into()) };
        ammunition.push(Ammunition { item, projectile: text(entry,"m_projectileDef"), attachment: text(entry,"m_attachmentDef"),
            fits_magazine: boolean(entry,"m_fitInMagazine",false)?, muzzle_world_fx: text(entry,"m_muzzleWorldFx"),
            damage_increment_attribute: text(entry,"m_damageIncrementAttribute"),
            bypass_big_hit_immunity_attribute: text(entry,"m_bypassBigHitImmunityAttribute"),
            gravity_multiplier_attribute: text(entry,"m_gravityMultiplierAttribute"),
            sleep_timer_multiplier_attribute: text(entry,"m_sleepTimerMultiplierAttribute"),
            spread_damage: boolean(entry,"m_spreadDamage",false)? });
    }
    Ok(ammunition)
}
#[derive(Clone, Debug)]
pub struct Ranged {
    pub family: RangedFamily,
    pub equipment: Equipment,
    pub ammunition: Vec<Ammunition>,
    pub aim: AimSettings,
    pub recoil_alias: Option<String>,
    pub recoil_weight_attribute: Option<String>,
    pub fire_playrate_attribute: Option<String>,
    pub reload_playrate_attribute: Option<String>,
    pub projectile_attachment_tag: Option<String>,
    pub projectile_fire_tag: Option<String>,
    pub fired_animation: Option<String>,
    pub muzzle_tag: Option<String>,
    pub muzzle_attached_fx: Option<String>,
    pub spread_area: Option<String>,
    pub spread_area_reduced: Option<String>,
    pub spread_area_upgraded: Option<String>,
    pub spread_area_offset: bevy_math::Vec3,
    pub magazine_fire_aliases: [Option<String>; 3],
    pub magazine_reload_aliases: [Option<String>; 3],
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
    pub fn capacity_for_ammunition(&self, item: &str, attributes: &super::states::Attributes) -> Option<i32> {
        self.ammunition.iter().position(|ammunition| ammunition.item == item).map(|index| self.capacity(index, attributes))
    }
    pub fn read(fields: &Fields, equipment: Equipment, family: RangedFamily) -> Result<Self, String> {
        let ammunition = read_ammunition(fields)?;
        Ok(Self { family, equipment, ammunition, aim: AimSettings::read(child(fields,"m_aimSettings"))?, recoil_alias: text(fields,"m_recoilAnim"), recoil_weight_attribute: text(fields,"m_recoilWeightAttribute"),
            fire_playrate_attribute: text(fields,"m_firePlayrateAttribute"), reload_playrate_attribute: text(fields,"m_reloadPlayrateAttribute"),
            projectile_attachment_tag: text(fields, match family { RangedFamily::Gun => "m_bulletAttachmentTag", RangedFamily::Crossbow => "m_boltAttachmentTag" }),
            projectile_fire_tag: text(fields,"m_bulletFiredTag"), fired_animation: text(fields,"m_boltFiredAnimName"),
            muzzle_tag: text(fields,"m_muzzleAttachmentTag"), muzzle_attached_fx: text(fields,"m_muzzleAttachedFx"),
            spread_area: text(fields,"m_spreadArea"), spread_area_reduced: text(fields,"m_spreadAreaReduced"),
            spread_area_upgraded: text(fields,"m_spreadAreaUpgraded"),
            spread_area_offset: {
                let offset = child(fields, "m_spreadAreaOffset");
                bevy_math::Vec3::new(crate::gameplay::damage::number(offset, "x", 0.0)?,
                    crate::gameplay::damage::number(offset, "y", 0.0)?, crate::gameplay::damage::number(offset, "z", 0.0)?)
            },
            magazine_fire_aliases: std::array::from_fn(|index| text(child(fields,"m_magFireAnimNames"), &format!("m_magFireAnimNames[{index}]"))),
            magazine_reload_aliases: std::array::from_fn(|index| text(child(fields,"m_magReloadAnimNames"), &format!("m_magReloadAnimNames[{index}]"))) })
    }

    /// Gun44df30: 202 animates the loaded magazine without spending a round.
    pub fn magazine_fire_animation(&self, magazine: i32) -> Option<(&str, super::animation::PlayMode)> {
        if magazine < 1 { return None; }
        self.magazine_fire_aliases[(magazine - 1).min(2) as usize].as_deref().map(|alias| (alias, super::animation::PlayMode::AndStay))
    }
    /// Gun44e920's pending final pose after the actual type5 child starts.
    pub fn magazine_final_animation(&self, magazine: i32) -> Option<(&str, super::animation::PlayMode)> {
        let alias = if magazine > 1 { self.magazine_reload_aliases[(magazine - 2).min(2) as usize].as_deref() }
            else { self.magazine_fire_aliases[0].as_deref() };
        alias.map(|alias| (alias, super::animation::PlayMode::StayOnLastFrame))
    }
    /// Gun44e7b0 attempts the reload alias first; only a failed lookup for
    /// counts <=3 falls back to the fire alias's last frame.
    pub fn magazine_reload_animations(&self, capacity: i32, total: u32) -> [Option<(&str, super::animation::PlayMode)>; 2] {
        use super::animation::PlayMode;
        let count = i64::from(capacity).min(i64::from(total));
        if count < 2 { return [self.magazine_fire_aliases[0].as_deref().map(|alias| (alias, PlayMode::StayOnLastFrame)), None]; }
        [self.magazine_reload_aliases[(count - 2).min(2) as usize].as_deref().map(|alias| (alias, PlayMode::AndStay)),
            (count <= 3).then(|| self.magazine_fire_aliases[(count - 1).min(2) as usize].as_deref().map(|alias| (alias, PlayMode::StayOnLastFrame))).flatten()]
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

/// Source identity and prepared resources are captured at the actual FIRE
/// callback, before ammunition consumption or an equipment-state transition.
#[derive(Clone, Debug)]
pub struct FireSource {
    pub prepared: std::sync::Arc<crate::assets::projectiles::Prepared>,
    pub attacker: crate::gameplay::entity_lifetime::EntityId,
    pub equipment: crate::gameplay::entity_lifetime::EntityId,
    pub eye: bevy_math::Vec3,
    pub camera_axis: bevy_math::Mat3,
    pub weapon_axis: bevy_math::Mat3,
    pub muzzle_frame: crate::gameplay::reference_frame::ReferenceFrame,
    pub muzzle_world_fx: Option<std::sync::Arc<str>>,
    pub explosion_fx_override: Option<std::sync::Arc<str>>,
    pub spread: Option<std::sync::Arc<crate::assets::explosions::Declaration>>,
    pub aim: bevy_math::Vec3,
    pub clip_from_world: bevy_math::Mat4,
    pub world_from_clip: bevy_math::Mat4,
    pub viewport: bevy_math::Vec2,
    pub gravity_multiplier: Option<f32>,
    pub damage_increment: f32,
    pub sleep_timer_multiplier: f32,
    pub bypass_big_hit_immunity: bool,
    pub speed_multiplier: f32,
    pub minimum_dispersion: f32,
    pub maximum_dispersion: f32,
    pub dispersion_ratio: f32,
    pub extra_dispersion_chance: f32,
    pub upgrade_flags: u32,
    pub always_break_on_hit: bool,
}

#[derive(Clone, Debug)]
pub enum WeaponEffect {
    /// The projectile owner receives the actual selected declaration and event
    /// properties. This is not replaced with a fabricated hitscan attack.
    Fire { weapon: std::sync::Arc<Ranged>, ammunition: usize, source: FireSource,
        frame: (bevy_math::Vec3, bevy_math::Mat3), parameters: FireParameters, dispersion_samples: [f32; 3] },
    MagazineChanged { previous: i32, current: i32 },
    AmmunitionChanged { previous: usize, current: usize },
    AnimateUpgradeForFire,
    AnimateUpgradeForReload,
    AnimateMagazineForFire,
    ShowLoadedAttachment,
    Targeting { enabled: bool },
    /// Actual active component Think ->61a650/61b4c0. The source identity
    /// survives a same-frame selection change without a new path allocation.
    UpgradeAnimation { identity: std::sync::Arc<str>, mode: u8, play: bool },
    /// 44dd00 ->44e720 marks the final magazine pose pending, separately
    /// from round count changes. The child waits for actual parent Think.
    MagazineCapacityRecomputed { identity: std::sync::Arc<str>, weapon: std::sync::Arc<Ranged>, magazine: i32 },
}

/// The original message receiver is a particular component, not whichever
/// weapon happens to be selected when the host drains its callback queue.
#[derive(Clone, Debug)]
pub struct WeaponMessage {
    pub identity: std::sync::Arc<str>,
    pub entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub weapon: std::sync::Arc<Ranged>,
    pub ammunition: usize,
    pub magazine: i32,
    pub capacity: i32,
    pub total_ammo: u32,
    pub effect: WeaponEffect,
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
    /// Crossbow62b440 requires its real, visible loaded-bolt entity.
    pub projectile_attachment_ready: bool,
    /// The canonical entity/socket frame captured before queued state changes.
    pub projectile_frame: Option<(bevy_math::Vec3, bevy_math::Mat3)>,
    /// Host admission boundary: the real projectile birth and collision
    /// consumer must exist before accepting a shot that spends ammunition.
    pub bevy_backend_projectile_ready: bool,
    /// Borrowed from actual owned-component preparation. Switching ammo selects
    /// a lease; it never opens a declaration/model on the firing callback.
    projectiles: Vec<Option<std::sync::Arc<crate::assets::projectiles::Prepared>>>,
    ammunition_flags: Vec<u32>,
    pub fire_source: Option<FireSource>,
    selected_ammunition: usize,
    active_scene: u32,
    equipment_target: usize,
    effects: Vec<WeaponMessage>,
    pending_fire: Vec<FireParameters>,
    pub(super) identity: Option<std::sync::Arc<str>>,
    pub(super) managed_entity: Option<crate::gameplay::entity_lifetime::EntityId>,
    active: bool,
    charge_upgrade_playing: bool,
    dispersion_ratio: f32,
    dispersion_velocity: f32,
}

impl Runtime {
    pub(super) fn write_diagnostics(&self, report: &mut String) {
        use std::fmt::Write;
        let _ = writeln!(report, "  ranged entity={:?} family={:?} active={} ammo={} item={:.160} magazine={}/{} total={} scene={} ready={} attachment_ready={} prepared={} targeting={} reload={} charged={} flags={:#x} queued_fire={} queued_effects={} dispersion={}", self.managed_entity, self.settings.family, self.active, self.selected_ammunition, self.ammunition().item, self.magazine, self.capacity, self.total_ammo, self.active_scene, self.bevy_backend_projectile_ready, self.projectile_attachment_ready, self.prepared_projectile().is_some(), self.targeting_requested, self.reload_requested, self.charged_shot, self.target_flags, self.pending_fire.len(), self.effects.len(), self.dispersion_ratio);
    }
    /// Capacity is supplied by the actual subclass/attribute owner; the
    /// generic constructor's default is one (637580+0x67).
    pub fn new(settings: std::sync::Arc<Ranged>, capacity: i32, total_ammo: u32, equipment_target: usize) -> Result<Self, String> {
        if settings.ammunition.is_empty() { return Err("ranged equipment has no ammunition declarations".into()); }
        if !(3..=4).contains(&equipment_target) { return Err("ranged equipment has no player equipment cast".into()); }
        if capacity < 0 { return Err("negative ranged magazine capacity".into()); }
        let projectile_attachment_ready = settings.family == RangedFamily::Gun;
        let mut value = Self { settings, magazine: 0, capacity, total_ammo, target_flags: 0, selected_ammunition: 0,
            active_scene: 0, equipment_target, effects: Vec::new(), pending_fire: Vec::new(), targeting_requested: false, reload_requested: false, charged_shot: false, projectile_attachment_ready, projectile_frame: None, bevy_backend_projectile_ready: false,
            identity: None, managed_entity: None, active: false, charge_upgrade_playing: false, projectiles: Vec::new(), ammunition_flags: Vec::new(), fire_source: None,
            dispersion_ratio: 1.0, dispersion_velocity: 0.0 };
        value.reload_magazine();
        Ok(value)
    }

    pub fn ammunition(&self) -> &Ammunition { &self.settings.ammunition[self.selected_ammunition] }
    pub fn default_animation(&self) -> Option<&str> {
        if self.settings.family == RangedFamily::Crossbow && self.magazine < 1 {
            self.settings.fired_animation.as_deref()
        } else { self.settings.equipment.default_alias.as_deref() }
    }
    pub fn message(&self, effect: WeaponEffect) -> Option<WeaponMessage> {
        Some(WeaponMessage { identity: self.identity.clone()?, entity: self.managed_entity, weapon: self.settings.clone(),
            ammunition: self.selected_ammunition, magazine: self.magazine, capacity: self.capacity,
            total_ammo: self.total_ammo, effect })
    }
    fn push_effect(&mut self, effect: WeaponEffect) {
        if let Some(message) = self.message(effect) { self.effects.push(message); }
    }
    pub fn ammunition_index(&self) -> usize { self.selected_ammunition }
    pub fn install_projectiles(&mut self, prepared: Vec<Option<std::sync::Arc<crate::assets::projectiles::Prepared>>>) -> Result<(), String> {
        if prepared.len() != self.settings.ammunition.len() { return Err("ranged projectile leases do not match authored ammunition records".into()); }
        for (ammo, lease) in self.settings.ammunition.iter().zip(&prepared) {
            if let Some(lease) = lease {
                if ammo.projectile.as_deref() != Some(lease.declaration.entity.as_str()) {
                    return Err("ranged projectile lease belongs to another ammunition declaration".into());
                }
            }
        }
        self.projectiles = prepared;
        self.fire_source = None;
        self.bevy_backend_projectile_ready = false;
        Ok(())
    }
    pub fn prepared_projectile(&self) -> Option<&std::sync::Arc<crate::assets::projectiles::Prepared>> {
        self.projectiles.get(self.selected_ammunition)?.as_ref()
    }
    pub fn install_ammunition_flags(&mut self, flags: Vec<u32>) -> Result<(), String> {
        if flags.len() != self.settings.ammunition.len() { return Err("ranged upgrade flags do not match actual ammunition records".into()); }
        self.target_flags = flags[self.selected_ammunition];
        self.ammunition_flags = flags;
        Ok(())
    }
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
    /// 61aed0 ->7e7670 starts the selected component's update ownership;
    /// registered-but-holstered Start61ac10 only initializes it (7e7400).
    pub fn activate(&mut self) { self.active = true; self.dispersion_ratio = 1.0; self.dispersion_velocity = 0.0; }
    pub fn update_aim(&mut self, attributes: &super::states::Attributes, dt: f32) {
        if self.magazine == 0 { self.dispersion_ratio = 1.0; self.dispersion_velocity = 0.0; return; }
        let target = attributes.get("ARK_ATTRIBUTE_RANGED_WEAPON_DISPERSION_RATIO");
        let speed = attributes.get(match self.settings.family {
            RangedFamily::Gun => "ARK_ATTRIBUTE_GUN_DISPERSION_SPEED_MULTIPLIER",
            RangedFamily::Crossbow => "ARK_ATTRIBUTE_CROSSBOW_DISPERSION_SPEED_MULTIPLIER",
        });
        let smoothing = if target <= self.dispersion_ratio { self.settings.aim.decrease } else { self.settings.aim.increase };
        shooting::smooth_ratio(&mut self.dispersion_ratio, &mut self.dispersion_velocity, target,
            smoothing.map(|value| value * speed), dt);
    }
    pub fn dispersion_ratio(&self) -> f32 { self.dispersion_ratio }
    pub fn is_active(&self) -> bool { self.active }
    pub(super) fn capacity_recomputed(&self) -> Option<WeaponEffect> {
        if self.settings.family != RangedFamily::Gun { return None; }
        Some(WeaponEffect::MagazineCapacityRecomputed { identity: self.identity.clone()?, weapon: self.settings.clone(), magazine: self.magazine })
    }
    pub fn deactivate(&mut self) {
        // Crossbow62bd20 stops charge upgrades before 63a1e0/61b010
        // removes this component's Think ownership through 7e7750.
        if self.charge_upgrade_playing {
            self.upgrade_animation(2, false);
            self.charge_upgrade_playing = false;
        }
        self.active = false;
        if self.targeting_requested { self.end_targeting(); }
        self.pending_fire.clear();
        self.active_scene = 0;
        self.reload_requested = false;
    }
    fn upgrade_animation(&mut self, mode: u8, play: bool) {
        if let Some(identity) = &self.identity {
            self.push_effect(WeaponEffect::UpgradeAnimation { identity: identity.clone(), mode, play });
        }
    }
    /// 44f240/62bf40 dispatch once for each actual selected-component Think.
    /// The child animation owner performs 53e770's already-playing check;
    /// repeated loop requests must not restart an animation or be discarded.
    pub fn think(&mut self, fire_state_charging: bool) {
        if !self.active { return; }
        match self.settings.family {
            RangedFamily::Gun => self.upgrade_animation(1, true),
            RangedFamily::Crossbow => {
                if self.target_flags & 4 != 0 && fire_state_charging {
                    self.upgrade_animation(2, true);
                    self.charge_upgrade_playing = true;
                } else if self.charge_upgrade_playing {
                    self.upgrade_animation(2, false);
                    self.charge_upgrade_playing = false;
                }
            }
        }
    }
    pub fn can_fire(&self) -> bool { self.bevy_backend_projectile_ready && self.fire_source.is_some()
        && self.projectile_frame.is_some() && self.magazine > 0 && (self.settings.family == RangedFamily::Gun || self.projectile_attachment_ready) }
    pub fn can_reload(&self) -> bool { self.magazine < self.capacity && i64::from(self.magazine) < i64::from(self.total_ammo) }
    pub fn update_inventory(&mut self, total: u32) { self.total_ammo = total; }
    pub fn request_reload(&mut self) { if self.can_reload() { self.reload_requested = true; } }
    /// 63bfc0 keeps a pending explicit reload until it succeeds, and requests
    /// an automatic reload when the current magazine is empty.
    pub fn take_reload_request(&mut self) -> bool {
        if !self.can_reload() { self.reload_requested = false; return false; }
        self.reload_requested || self.magazine == 0
    }
    pub fn begin_targeting(&mut self) { self.targeting_requested = true; self.push_effect(WeaponEffect::Targeting { enabled: true }); }
    pub fn end_targeting(&mut self) { self.targeting_requested = false; self.push_effect(WeaponEffect::Targeting { enabled: false }); }
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
        if capacity < 0 { return Err("negative ranged magazine capacity".into()); }
        self.clear_magazine();
        let previous = self.selected_ammunition;
        self.selected_ammunition = index;
        if let Some(flags) = self.ammunition_flags.get(index) { self.target_flags = *flags; }
        self.capacity = capacity;
        if previous != index {
            self.bevy_backend_projectile_ready = false;
            self.fire_source = None;
            if self.settings.family == RangedFamily::Crossbow { self.projectile_attachment_ready = false; self.projectile_frame = None; }
            self.push_effect(WeaponEffect::AmmunitionChanged { previous, current: index });
        }
        self.total_ammo = inventory.quantity(item);
        self.reload_magazine();
        // 63a490 invokes OnAmmoChanged100 only when the actual record changes;
        // its gun override44dd00 performs44e720 even for an equal capacity.
        if previous != index {
            if let Some(effect) = self.capacity_recomputed() { self.push_effect(effect); }
        }
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

    pub fn take_effects(&mut self) -> Vec<WeaponMessage> { std::mem::take(&mut self.effects) }
    pub fn drain_effects(&mut self) -> std::vec::Drain<'_, WeaponMessage> { self.effects.drain(..) }

    fn change_magazine(&mut self, current: i32) {
        let previous = std::mem::replace(&mut self.magazine, current);
        self.push_effect(WeaponEffect::MagazineChanged { previous, current });
    }
    pub fn reload_magazine(&mut self) {
        self.change_magazine(i64::from(self.capacity).min(i64::from(self.total_ammo)) as i32);
    }
    pub fn emit_reload(&mut self) {
        // Gun44f440 shows its already-loaded bullet; crossbow62bfe0 invokes
        // the reload callback directly. The separate gun203 key fills it.
        match self.settings.family {
            RangedFamily::Gun => self.push_effect(WeaponEffect::ShowLoadedAttachment),
            RangedFamily::Crossbow => self.reload_magazine(),
        }
    }
    pub fn clear_magazine(&mut self) { self.change_magazine(0); }

    /// 866dc0 posts ARK_MSG_WEAPON_FIRE directly only when the weapon's
    /// selected fire-scene pointer is null. The caller performs that decision.
    pub fn emit_fire(&mut self) {
        self.pending_fire.push(FireParameters { charged_shot: self.charged_shot, ..Default::default() });
    }
    pub fn flush_pending_fire(&mut self, inventory: &mut (impl AmmunitionInventory + ?Sized), random: &mut u32) {
        let mut pending = std::mem::take(&mut self.pending_fire);
        for parameters in pending.drain(..) { self.fire_with_inventory(parameters, inventory, random); }
        self.pending_fire = pending;
    }
    fn fire_with_inventory(&mut self, mut parameters: FireParameters, inventory: &mut (impl AmmunitionInventory + ?Sized), random: &mut u32) {
        // Both 44e0c0 and 62b560 check virtual110, consume via virtual140,
        // then dispatch the selected projectile and muzzle effects.
        if !self.can_fire() { return; }
        let mut source = self.fire_source.as_ref().expect("can_fire validates source").clone();
        source.dispersion_ratio = self.dispersion_ratio;
        source.upgrade_flags = self.target_flags;
        let frame = self.projectile_frame.expect("can_fire validates actual firing socket");
        parameters.charged_shot = self.charged_shot;
        self.change_magazine((self.magazine - 1).max(0));
        let item = self.ammunition().item.as_str();
        inventory.consume(item, 1);
        self.total_ammo = inventory.quantity(item);
        self.push_effect(WeaponEffect::Fire { weapon: self.settings.clone(), ammunition: self.selected_ammunition,
            source, frame, parameters, dispersion_samples: if self.settings.family == RangedFamily::Gun && self.target_flags & 0x20 != 0 {
                [0.0; 3]
            } else { std::array::from_fn(|_| crate::random::lcg_unit(random)) } });
        // Both subclass fire callbacks end in63a8a0, after their shots.
        self.dispersion_ratio = 1.0;
        self.dispersion_velocity = 0.0;
    }

    /// Native Midnight message IDs c8..cd are registered at 141a11c80.
    /// Target and live scene handle prevent a cancelled/other weapon's keys
    /// from spending this component's ammunition.
    pub fn handle_event(&mut self, event: &super::AnimationEvent, inventory: &mut (impl AmmunitionInventory + ?Sized), random: &mut u32) -> Result<bool, String> {
        if event.target != self.equipment_target || event.handle != self.active_scene || self.active_scene == 0 { return Ok(false); }
        match event.name.as_str() {
            "ARK_MDNT_MSG_WEAPON_FIRE" => {
                self.fire_with_inventory(FireParameters {
                    shoot_on_aiming_target: boolean(&event.properties, "m_shootOnAimingTarget", false)?,
                    attack_tag: text(&event.properties, "m_attackTag"),
                    charged_shot: self.charged_shot,
                }, inventory, random);
            }
            "ARK_MDNT_MSG_WEAPON_CONSUME_AMMO_FROM_MAGAZINE" => {
                // Gun44f3d0 animates the upgrade magazine. Despite the name,
                // the native callback never consumes an inventory item here.
                if self.settings.family == RangedFamily::Gun && self.capacity > 1 {
                    self.push_effect(WeaponEffect::AnimateMagazineForFire);
                }
            }
            "ARK_MDNT_MSG_WEAPON_RELOAD" => self.emit_reload(),
            "ARK_MDNT_MSG_WEAPON_RELOAD_MAGAZINE" if self.settings.family == RangedFamily::Gun => {
                self.total_ammo = inventory.quantity(&self.ammunition().item);
                self.reload_magazine();
            }
            "ARK_MDNT_MSG_WEAPON_UPGRADE_ANIMATE_FOR_FIRE" => self.push_effect(WeaponEffect::AnimateUpgradeForFire),
            "ARK_MDNT_MSG_WEAPON_UPGRADE_ANIMATE_FOR_RELOAD" => {
                //Gun44f470 invokes virtual148 before animating its magazine.
                if self.settings.family == RangedFamily::Gun {
                    self.total_ammo = inventory.quantity(&self.ammunition().item);
                    self.reload_magazine();
                }
                self.push_effect(WeaponEffect::AnimateUpgradeForReload);
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}
