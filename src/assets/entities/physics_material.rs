//! arkPhysicsMaterial declarations (`generated/decls/physicsmaterial/*.decl`).
//! Defaults come from the constructor FUN_14008b640; a declaration starts
//! from its `inherit` parent's resolved values and applies its `edit` block.
use super::components::atom;
use super::{field, parse_declaration, scope};

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
    fn resolve(
        name: &str,
        depth: usize,
        read: &mut impl FnMut(&str) -> Result<Option<String>, String>,
    ) -> Result<PhysicsMaterial, String> {
        if depth > 32 {
            return Err(format!("physics material inheritance exceeds 32: {name}"));
        }
        let path = format!("generated/decls/physicsmaterial/{name}.decl");
        let source = read(&path)?.ok_or_else(|| format!("missing physics material {path}"))?;
        let fields = parse_declaration(&source)?;
        let mut material = match atom(&fields, "inherit") {
            Some(parent) => resolve(parent, depth + 1, read)?,
            None => PhysicsMaterial::constructor_default(name),
        };
        material.name = name.to_owned();
        let edit = scope(field(&fields, "edit")).unwrap_or(&fields);
        for (key, slot) in [
            ("m_Density", &mut material.density),
            ("m_DynamicFriction", &mut material.dynamic_friction),
            ("m_StaticFriction", &mut material.static_friction),
            ("m_Restitution", &mut material.restitution),
        ] {
            if let Some(text) = atom(edit, key) {
                *slot = decl_float(text).ok_or_else(|| format!("invalid {key} in {path}"))?;
            }
        }
        for (key, bit) in FLAG_FIELDS {
            match atom(edit, key) {
                Some("true") => material.flags |= bit,
                Some("false") => material.flags &= !bit,
                Some(other) => return Err(format!("invalid {key} = {other} in {path}")),
                None => {}
            }
        }
        Ok(material)
    }
    resolve(name, 0, &mut read)
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
