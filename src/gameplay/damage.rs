//! Health damage calculation: 717800/717dc0/717e40, evolved 718fe0/719d20.
//! Sources supply real hit data and entity owners supply immunity/armor state.
use crate::entities::Value;
use bevy_math::Vec3;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DamageType(pub u64);
impl DamageType {
    pub const SLEEP: Self = Self(1);
    pub const CANT_KILL_NPC: Self = Self(2);
    pub const INSTANT_KILL: Self = Self(4);
    pub const EXPLOSION: Self = Self(0x400);
    pub const FALL: Self = Self(0x2000);
    pub const WATER: Self = Self(0x100000);
    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
    /// Enum registration 1419f70b0; serialized masks may combine names.
    pub fn read(text: &str) -> Result<Self, String> {
        const NAMES: &[&str] = &[
            "SLEEP",
            "CANTKILLNPC",
            "INSTANTKILL",
            "NO_REACTION",
            "PHYSICS",
            "PHYSICS_HARMLESS",
            "PHYSICS_STUN_IN_HEAD",
            "MELEE",
            "BOLT",
            "BULLET",
            "EXPLOSION",
            "WALLOFLIGHT",
            "RATBITE",
            "FALL",
            "LIGHT",
            "BLOODFLIES",
            "FLASH",
            "BLOWDUST",
            "ELECTRIC",
            "WINDBLAST",
            "WATER",
            "HOUNDBITE",
            "IMMOLATE",
            "FIRE",
            "SHRIEK",
            "BLOWBACK",
            "OVERHEAT",
            "CURE",
            "PRE_SLEEP",
            "DEATHRAY",
            "SHOVED",
            "VINE_SMASH",
            "SPREAD",
            "STONE_SHROUD",
            "SLIDING",
            "HAZARDS",
            "AGAINST_MECH",
            "STICKY_HEAD",
            "MUSIC_BOX_ALLEGRO",
            "CRUSHED",
            "PHANTASMAL",
            "BURNING_VENOM",
            "FOCUS_BLINK_ATTACK",
            "GRAPPLE",
            "CANT_INCAPACITATE_PLAYER_AND_ALLIES",
            "STICK",
            "SPREAD_UPGRADED",
        ];
        let mut mask = 0;
        for word in text
            .trim_matches('"')
            .split(|ch: char| ch.is_whitespace() || ch == '|' || ch == ',')
            .filter(|word| !word.is_empty())
        {
            mask |= if word == "DAMAGETYPE_ALL" {
                0x7fffffffffff
            } else if let Some(index) = word
                .strip_prefix("DAMAGETYPE_")
                .and_then(|name| NAMES.iter().position(|candidate| *candidate == name))
            {
                1u64 << index
            } else if let Some(hex) = word.strip_prefix("0x") {
                u64::from_str_radix(hex, 16).map_err(|_| format!("invalid damage mask {word}"))?
            } else {
                word.parse::<u64>()
                    .map_err(|_| format!("unknown damage type {word}"))?
            };
        }
        Ok(Self(mask))
    }
}

pub(crate) fn atom<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| match value {
            Value::Atom(text) => Some(text.trim_matches('"')),
            _ => None,
        })
}
pub(crate) fn number(fields: &[(String, Value)], key: &str, default: f32) -> Result<f32, String> {
    atom(fields, key).map_or(Ok(default), |text| {
        text.trim_end_matches('f')
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| format!("invalid {key}: {text}"))
    })
}
fn mask(fields: &[(String, Value)], key: &str) -> Result<DamageType, String> {
    DamageType::read(atom(fields, key).unwrap_or("0"))
}
fn region(text: &str) -> Result<u32, String> {
    const NAMES: &[&str] = &[
        "NONE",
        "GLOBAL",
        "HEAD",
        "TORSO",
        "LEFT_ARM",
        "RIGHT_ARM",
        "LEFT_LEG",
        "RIGHT_LEG",
        "LEFT_ARM_BACK",
        "RIGHT_ARM_BACK",
        "LOWER_LEFT_ARM_BACK",
        "LOWER_RIGHT_ARM_BACK",
        "LOWER_LEFT_ARM",
        "LOWER_RIGHT_ARM",
        "LOWER_LEFT_LEG",
        "LOWER_RIGHT_LEG",
    ];
    text.strip_prefix("BODY_REGION_")
        .and_then(|name| NAMES.iter().position(|candidate| *candidate == name))
        .map(|index| index as u32)
        .or_else(|| text.parse().ok())
        .ok_or_else(|| format!("unknown body region {text}"))
}

#[derive(Clone, Debug)]
pub struct RegionResistance {
    pub region: u32,
    pub ignored: DamageType,
    pub scaled: DamageType,
    pub scaled_if_unaware: DamageType,
    pub scale: f32,
}
#[derive(Clone, Debug, Default)]
pub struct Receiver {
    pub allowed: DamageType,
    pub ignored: DamageType,
    pub infinite_health: bool,
    /// DIR_ALL=0, DIR_FRONT=1, DIR_BACK=2 (1419f9d40).
    pub direction: u8,
    pub regions: Vec<RegionResistance>,
    pub explosion_target_points: Vec<String>,
    pub explosion_aabb_samples: Vec<Vec3>,
}
impl Receiver {
    pub fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let direction = match atom(fields, "m_allowDamageDirection").unwrap_or("DIR_ALL") {
            "DIR_ALL" | "0" => 0,
            "DIR_FRONT" | "1" => 1,
            "DIR_BACK" | "2" => 2,
            value => return Err(format!("unknown damage direction {value}")),
        };
        let mut regions = Vec::new();
        if let Some(Value::Scope(list)) = fields
            .iter()
            .find(|(key, _)| key == "m_bodyRegions")
            .map(|(_, value)| value)
        {
            let count = atom(list, "num")
                .unwrap_or("0")
                .parse::<usize>()
                .map_err(|_| "invalid body region count")?;
            for index in 0..count {
                let key = format!("item[{index}]");
                let Some(Value::Scope(fields)) = list
                    .iter()
                    .find(|(name, _)| name == &key)
                    .map(|(_, value)| value)
                else {
                    return Err(format!("missing body region {index}"));
                };
                regions.push(RegionResistance {
                    region: region(atom(fields, "m_bodyRegion").unwrap_or("BODY_REGION_NONE"))?,
                    ignored: mask(fields, "m_ignoredDamageTypes")?,
                    scaled: mask(fields, "m_scaledDamageTypes")?,
                    scaled_if_unaware: mask(fields, "m_scaledDamageTypesIfUnaware")?,
                    scale: number(fields, "m_damageScale", 1.0)?,
                });
            }
        }
        let infinite_health = match atom(fields, "m_infiniteHealth").unwrap_or("false") {
            "true" | "1" => true,
            "false" | "0" => false,
            value => return Err(format!("invalid infinite health {value}")),
        };
        let list = |key: &str| -> Result<&[(String, Value)], String> {
            match crate::entities::field(fields, key) {
                None => Ok(&[]),
                Some(Value::Scope(v)) => Ok(v),
                _ => Err(format!("invalid {key} list")),
            }
        };
        let points = list("m_explosionTargetPoints")?;
        let mut explosion_target_points = Vec::new();
        for (name, value) in points.iter().filter(|(n, _)| n.starts_with("item[")) {
            let Value::Atom(value) = value else {
                return Err(format!("invalid explosion target point {name}"));
            };
            explosion_target_points.push(value.trim_matches('"').to_owned());
        }
        let mut explosion_aabb_samples = Vec::new();
        for (name, value) in list("m_explosionTargetAabbSample")?
            .iter()
            .filter(|(n, _)| n.starts_with("item["))
        {
            let Value::Scope(value) = value else {
                return Err(format!("invalid explosion AABB sample {name}"));
            };
            explosion_aabb_samples.push(Vec3::new(
                number(value, "x", 0.)?,
                number(value, "y", 0.)?,
                number(value, "z", 0.)?,
            ));
        }
        Ok(Self {
            allowed: mask(fields, "m_allowedDamageTypes")?,
            ignored: mask(fields, "m_ignoredDamageTypes")?,
            infinite_health,
            direction,
            regions,
            explosion_target_points,
            explosion_aabb_samples,
        })
    }
    pub fn accepts(&self, damage_type: DamageType) -> bool {
        if self.allowed.0 != 0 {
            self.allowed.intersects(damage_type)
        } else {
            !self.ignored.intersects(damage_type)
        }
    }
    pub fn evaluate(&self, current: f32, params: &Parameters, context: Context) -> DamageResult {
        let dot = context.forward.dot(params.hit_direction);
        // 717dc0 reverses incoming hit direction, then 6dd130 compares <0.
        let direction = 1 + u8::from(-dot < 0.0);
        if !self.accepts(params.damage_type)
            || (self.direction != 0 && direction != self.direction)
            || context.immune
            || !context.started
        {
            return DamageResult::default();
        }
        let instant = params.damage_type.intersects(DamageType::INSTANT_KILL);
        // 717800 still applies the selected difficulty factor before the
        // instant-kill branch replaces damage with current health.
        let mut damage = if instant {
            current
        } else {
            params.amount * context.difficulty_factor
        };
        if !instant && context.evolved {
            let mut factor = context.armor_factor;
            if let Some(region) = self
                .regions
                .iter()
                .find(|region| region.region == params.hit_region)
            {
                if region.ignored.intersects(params.damage_type) {
                    factor = 0.0;
                } else if region.scaled.intersects(params.damage_type)
                    || (context.unaware_of_player
                        && region.scaled_if_unaware.intersects(params.damage_type))
                {
                    factor *= region.scale;
                }
            }
            // 718fe0 adds deviations from 1, rather than multiplying.
            if params.damage_type.intersects(DamageType::EXPLOSION) {
                factor += context.explosion_factor - 1.0;
            }
            factor += context.incoming_scale - 1.0;
            damage *= factor;
        }
        if context.cannot_lose_health {
            damage = 0.0;
        }
        let mana_damage = if !instant && context.evolved && context.health_to_mana > 0.0 {
            (context.health_to_mana * damage).min(context.mana)
        } else {
            0.0
        };
        damage -= mana_damage;
        let minimum = if context.npc
            && (params.damage_type.intersects(DamageType::SLEEP)
                || params.damage_type.intersects(DamageType::CANT_KILL_NPC))
        {
            1.0
        } else {
            context.minimum_health
        };
        damage = damage.min(current - minimum);
        let health_damage = if self.infinite_health { 0.0 } else { damage };
        let incapacitated = !self.infinite_health && current - health_damage < 0.5;
        DamageResult {
            health_damage,
            mana_damage,
            incapacitated,
            sleep: !incapacitated && params.damage_type.intersects(DamageType::SLEEP),
            accepted: true,
        }
    }
    /// HealthDoor virtual+d0 (39edc0) first calls base717800, then applies
    /// door-only overrides, even when the base type/direction filter rejected
    /// the hit. Maximum and current health are intentionally different here.
    /// Context flags come from actual managed attacker/inflictor owners.
    pub fn evaluate_door(
        &self,
        current: f32,
        maximum: f32,
        params: &Parameters,
        context: Context,
        door: DoorContext,
    ) -> DamageResult {
        let mut result = self.evaluate(
            current,
            params,
            Context {
                evolved: false,
                ..context
            },
        );
        let mut override_damage = None;
        // WINDBLAST | FOCUS_BLINK_ATTACK (39edc0).
        if params.damage_type.0 & 0x40000080000 != 0 {
            override_damage = Some(maximum);
        }
        if params.damage_type.0 & 0x80 != 0 && door.attacker_exists {
            override_damage = Some(
                if door.attacker_is_player
                    && (door.greater_strength_modified || door.declaration_focus_blink_attack)
                {
                    maximum
                } else {
                    0.0
                },
            );
        }
        if params.damage_type.0 & 0x10 != 0 {
            override_damage = Some(if door.inflictor_npc_breaking_reaction {
                maximum
            } else {
                0.0
            });
        }
        if let Some(amount) = override_damage {
            result.health_damage = if self.infinite_health { 0.0 } else { amount };
            result.accepted = true;
            result.incapacitated = !self.infinite_health && current - result.health_damage < 0.5;
            result.sleep =
                !result.incapacitated && params.damage_type.intersects(DamageType::SLEEP);
        }
        result
    }
}

/// 39ef50 resolves STRENGTH_GREATER_STRENGTH on the actual player's
/// attributes; 39eed0 compares its live value with that attribute's base.
/// Physics override39edc0 requires an actual NPC AIPhysicsReaction+150 bit2.
/// Absence and unsupported actor owners must stay distinct at the adapter.
#[derive(Clone, Copy, Debug, Default)]
pub struct DoorContext {
    pub attacker_exists: bool,
    pub attacker_is_player: bool,
    pub greater_strength_modified: bool,
    pub declaration_focus_blink_attack: bool,
    pub inflictor_npc_breaking_reaction: bool,
}
impl DoorContext {
    pub fn player(
        attributes: &crate::gameplay::player::states::Attributes,
        declaration: &crate::assets::damage::Declaration,
    ) -> Self {
        let tag = "ARK_ATTRIBUTE_STRENGTH_GREATER_STRENGTH";
        Self {
            attacker_exists: true,
            attacker_is_player: true,
            greater_strength_modified: attributes
                .1
                .get(tag)
                .is_some_and(|value| attributes.get(tag) != value.base),
            declaration_focus_blink_attack: declaration.damage_type.0 & 0x40000000000 != 0,
            inflictor_npc_breaking_reaction: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Parameters {
    pub amount: f32,
    pub damage_type: DamageType,
    pub hit_direction: Vec3,
    /// Actual collision/source location and normal. None means the source
    /// did not supply a value; chunk/impact owners must require the actual
    /// hit data instead of substituting the receiver's center or a plane.
    pub hit_location: Option<Vec3>,
    pub hit_normal: Option<Vec3>,
    pub hit_distance: f32,
    pub affected_regions: u32,
    pub hit_region: u32,
    pub attacker: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub inflictor: Option<crate::gameplay::entity_lifetime::EntityId>,
    pub ignore_armor: bool,
}
impl Parameters {
    /// 780260 initializes the damage amount from +5c (m_maxDamage).
    pub fn from_declaration(declaration: &crate::assets::damage::Declaration) -> Self {
        Self {
            amount: declaration.maximum,
            damage_type: declaration.damage_type,
            hit_direction: Vec3::ZERO,
            hit_location: None,
            hit_normal: None,
            hit_distance: 0.0,
            affected_regions: 0,
            hit_region: 1,
            attacker: None,
            inflictor: None,
            ignore_armor: false,
        }
    }
    /// 780a80 accepts arbitrary caller damage when max<=0; otherwise the
    /// source's clamp flag limits the override to its authored [min,max].
    pub fn set_amount(
        &mut self,
        declaration: &crate::assets::damage::Declaration,
        amount: f32,
        clamp: bool,
    ) {
        self.amount = if clamp && declaration.maximum > 0.0 {
            if amount < declaration.minimum {
                declaration.minimum
            } else {
                amount.min(declaration.maximum)
            }
        } else {
            amount
        };
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Context {
    pub forward: Vec3,
    pub difficulty_factor: f32,
    pub incoming_scale: f32,
    pub explosion_factor: f32,
    /// Armor owner output from 435200, before region/attribute modifiers.
    pub armor_factor: f32,
    pub health_to_mana: f32,
    pub mana: f32,
    pub minimum_health: f32,
    pub npc: bool,
    pub evolved: bool,
    pub unaware_of_player: bool,
    pub immune: bool,
    pub cannot_lose_health: bool,
    pub started: bool,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            forward: Vec3::X,
            difficulty_factor: 1.0,
            incoming_scale: 1.0,
            explosion_factor: 1.0,
            armor_factor: 1.0,
            health_to_mana: 0.0,
            mana: 0.0,
            minimum_health: 0.0,
            npc: false,
            evolved: false,
            unaware_of_player: false,
            immune: false,
            cannot_lose_health: false,
            started: true,
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct DamageResult {
    pub health_damage: f32,
    pub mana_damage: f32,
    pub incapacitated: bool,
    pub sleep: bool,
    pub accepted: bool,
}
