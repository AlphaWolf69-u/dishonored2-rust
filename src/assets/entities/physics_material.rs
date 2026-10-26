//! arkPhysicsMaterial declarations (`generated/decls/physicsmaterial/*.decl`).
//! Defaults come from the constructor FUN_14008b640; a declaration starts
//! from its `inherit` parent's resolved values and applies its `edit` block.
use super::components::atom;
use super::{field, scope};

/// arkPhysicsMaterial flag byte +0x80. Bit order is the reflection order;
/// each getter (FUN_140990040..1409900b0) reads one bit of that byte.
pub mod flags {
    pub const ALLOW_MANTLE: u8 = 1 << 0;
    pub const ALLOW_VAULT: u8 = 1 << 1;
    pub const CAN_BLINK_THROUGH: u8 = 1 << 2;
    pub const CAN_GRAPPLE_THROUGH: u8 = 1 << 3;
    pub const CAN_WINDBLAST_THROUGH: u8 = 1 << 4;
    /// FUN_14008b6e0 turns this into an hknpMaterial trigger.
    pub const TRIGGER_MATERIAL: u8 = 1 << 5;
    pub const TRIGGER_TYPE_CONTACT_SOLVER: u8 = 1 << 6;
    pub const IS_STAIRS: u8 = 1 << 7;
}

const FLAG_FIELDS: [(&str, u8); 8] = [
    ("m_allowMantle", flags::ALLOW_MANTLE),
    ("m_allowVault", flags::ALLOW_VAULT),
    ("m_canBlinkThrough", flags::CAN_BLINK_THROUGH),
    ("m_canGrappleThrough", flags::CAN_GRAPPLE_THROUGH),
    ("m_canWindblastThrough", flags::CAN_WINDBLAST_THROUGH),
    ("m_triggerMaterial", flags::TRIGGER_MATERIAL),
    (
        "m_triggerTypeContactSolver",
        flags::TRIGGER_TYPE_CONTACT_SOLVER,
    ),
    ("m_isStairs", flags::IS_STAIRS),
];

#[derive(Clone, Debug, PartialEq)]
pub struct PhysicsMaterial {
    /// Declaration name below `physicsmaterial/`, as the physics world lists it.
    pub name: String,
    /// m_Density +0x58.
    pub density: f32,
    /// m_DynamicFriction +0x5c.
    pub dynamic_friction: f32,
    /// m_StaticFriction +0x60.
    pub static_friction: f32,
    /// m_Restitution +0x64.
    pub restitution: f32,
    /// Flag byte +0x80 (`flags::*`).
    pub flags: u8,
    pub collision: CollisionSettings,
}

impl PhysicsMaterial {
    /// FUN_14008b640 constructor values: density 1000, both frictions 0.5,
    /// restitution 0, flag byte 3 (mantle and vault allowed).
    pub fn constructor_default(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            density: 1000.,
            dynamic_friction: 0.5,
            static_friction: 0.5,
            restitution: 0.,
            flags: flags::ALLOW_MANTLE | flags::ALLOW_VAULT,
            collision: CollisionSettings::default(),
        }
    }
    /// Mantle sensor surface test (FUN_1403bafe0 reads bit 0).
    pub fn allow_mantle(&self) -> bool {
        self.flags & flags::ALLOW_MANTLE != 0
    }
    /// Vault sensor surface test (FUN_1403cfd30 reads bit 1).
    pub fn allow_vault(&self) -> bool {
        self.flags & flags::ALLOW_VAULT != 0
    }
    pub fn can_blink_through(&self) -> bool {
        self.flags & flags::CAN_BLINK_THROUGH != 0
    }
    pub fn can_grapple_through(&self) -> bool {
        self.flags & flags::CAN_GRAPPLE_THROUGH != 0
    }
    pub fn can_windblast_through(&self) -> bool {
        self.flags & flags::CAN_WINDBLAST_THROUGH != 0
    }
    pub fn is_stairs(&self) -> bool {
        self.flags & flags::IS_STAIRS != 0
    }
}

/// Decl float atoms may carry a C `f` suffix (installed `character_body`
/// writes `0.0f`).
fn decl_float(text: &str) -> Option<f32> {
    text.strip_suffix('f')
        .unwrap_or(text)
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
}

/// Resolve `generated/decls/physicsmaterial/{name}.decl` with inheritance.
pub fn physics_material(
    name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<PhysicsMaterial, String> {
    let fields = component_fields("physicsmaterial", name, 0, &mut HashMap::new(), &mut read)?;
    let mut material = PhysicsMaterial::constructor_default(name);
    let path = format!("generated/decls/physicsmaterial/{name}.decl");
    for (key, slot) in [
        ("m_Density", &mut material.density),
        ("m_DynamicFriction", &mut material.dynamic_friction),
        ("m_StaticFriction", &mut material.static_friction),
        ("m_Restitution", &mut material.restitution),
    ] {
        if let Some(text) = atom(&fields, key) {
            *slot = decl_float(text).ok_or_else(|| format!("invalid {key} in {path}"))?;
        }
    }
    for (key, bit) in FLAG_FIELDS {
        match atom(&fields, key) {
            Some("true") => material.flags |= bit,
            Some("false") => material.flags &= !bit,
            Some(other) => return Err(format!("invalid {key} = {other} in {path}")),
            None => {}
        }
    }
    material.collision = collision_settings_fields(name, &fields)?;
    Ok(material)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &str) -> Result<Option<String>, String> {
        Ok(match path {
            "generated/decls/physicsmaterial/contactsystem/environment.decl" => {
                Some("{ edit = { m_Restitution = 0.1000000015; } }".into())
            }
            "generated/decls/physicsmaterial/contactsystem/env.metal.grid.decl" => Some(
                r#"{ inherit = "contactsystem/environment";
                     edit = { m_canGrappleThrough = true; m_canWindblastThrough = true; } }"#
                    .into(),
            ),
            "generated/decls/physicsmaterial/contactsystem/env.metal.grid.mantleoff.decl" => Some(
                r#"{ inherit = "contactsystem/env.metal.grid";
                     edit = { m_allowMantle = false; m_allowVault = false; } }"#
                    .into(),
            ),
            "generated/decls/physicsmaterial/character_body.decl" => Some(
                "{ edit = { m_DynamicFriction = 0.0f; m_StaticFriction = 0.0f;
                   m_allowMantle = false; m_allowVault = false; } }"
                    .into(),
            ),
            _ => None,
        })
    }

    #[test]
    fn inherits_constructor_defaults_and_parent_flags() {
        let grid = physics_material("contactsystem/env.metal.grid", read).unwrap();
        assert!(grid.allow_mantle() && grid.allow_vault());
        assert!(grid.can_grapple_through() && grid.can_windblast_through());
        assert!(!grid.can_blink_through() && !grid.is_stairs());
        assert_eq!(grid.restitution, 0.1000000015);
        assert_eq!((grid.density, grid.static_friction), (1000., 0.5));

        let off = physics_material("contactsystem/env.metal.grid.mantleoff", read).unwrap();
        assert!(!off.allow_mantle() && !off.allow_vault());
        assert!(off.can_grapple_through());
        assert_eq!(off.name, "contactsystem/env.metal.grid.mantleoff");
    }

    #[test]
    fn reads_suffixed_floats_and_rejects_missing_declarations() {
        let body = physics_material("character_body", read).unwrap();
        assert_eq!((body.dynamic_friction, body.static_friction), (0., 0.));
        assert_eq!(body.flags, 0);
        assert!(physics_material("missing", read).is_err());
    }
}

use super::components::component_fields;
use crate::gameplay::collision_settings::{CollisionBlock, CollisionSettings};
use std::collections::HashMap;
/// Resolve the complete inherited edit before the settings post-parse conversion
/// (98aeb0). Runtime material IDs and material maps belong to the world owner.
pub fn collision_settings(
    name: &str,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<CollisionSettings, String> {
    Ok(physics_material(name, &mut read)?.collision)
}

fn collision_settings_fields(
    name: &str,
    fields: &[(String, super::Value)],
) -> Result<CollisionSettings, String> {
    let mut result = CollisionSettings::default();
    if let Some(value) = field(&fields, "m_collisionSettings") {
        let settings =
            scope(Some(value)).ok_or("physics material collision settings must be a scope")?;
        if let Some(flags) = atom(settings, "m_blockFlags") {
            result.flags = 0;
            for flag in flags.split_whitespace() {
                // Values from the executable's named enum metadata, caa8 onward.
                result.flags |= match flag {
                    "eBLOCK_NONE" => 0,
                    "eBLOCK_NPC" => 1,
                    "eBLOCK_PLAYER" => 2,
                    "eBLOCK_WEAPON" => 4,
                    "eBLOCK_VISION" => 8,
                    "eBLOCK_DYNAMIC" => 16,
                    "eBLOCKED_BY_STATIC" => 32,
                    "eBLOCK_DEBRIS_AND_PARTICLES" => 64,
                    "eBLOCK_RAGDOLL" => 128,
                    "eBLOCK_DEFAULT" => 127,
                    "eBLOCK_ALL" => 65535,
                    _ => return Err(format!("unknown collision block flag {flag} in {name}")),
                };
            }
        }
        if let Some(version) = atom(settings, "m_version") {
            result.version = version
                .parse()
                .map_err(|_| format!("invalid collision settings version in {name}"))?;
        }
        if let Some(blocker) = atom(settings, "m_blocker") {
            result.blocker = match blocker {
                "BLOCK_DEFAULT" => 0,
                "BLOCK_NONE" => 1,
                "BLOCK_CUSTOM" => 2,
                "BLOCK_ALL" => 3,
                "BLOCK_INVALID" => 4,
                _ => return Err(format!("unknown collision blocker {blocker} in {name}")),
            };
        }
        for (key, block) in [
            ("m_blockNPC", CollisionBlock::Npc),
            ("m_blockPlayer", CollisionBlock::Player),
            ("m_blockWeapon", CollisionBlock::Weapon),
            ("m_blockVision", CollisionBlock::Vision),
            ("m_blockDynamic", CollisionBlock::Dynamic),
            ("m_blockedByStatic", CollisionBlock::BlockedByStatic),
            (
                "m_blockDebrisAndParticles",
                CollisionBlock::DebrisAndParticles,
            ),
            ("m_blockRagdoll", CollisionBlock::Ragdoll),
        ] {
            if let Some(value) = atom(settings, key) {
                let enabled = match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err(format!("invalid {key} in {name}")),
                };
                result.set_legacy_blocking(block, enabled);
            }
        }
    }
    result.upgrade();
    Ok(result)
}

#[cfg(test)]
#[path = "physics_material/tests.rs"]
mod collision_tests;
