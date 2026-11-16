//! Reflected rigid-body fields from the resolved component declaration.
//! Missing values follow the original 140806eb0 constructor; malformed
//! authored values are errors rather than silently selecting a default.
use super::{RigidBodySettings, Value};

/// One immutable decoded declaration shared by admitted components and their
/// readable bindings. Per-instance body and lifecycle state stay in the app.
#[derive(Debug)]
pub(crate) struct Definition {
    pub settings: RigidBodySettings,
    pub declaration_flags: u8,
    pub scheduling:
        std::sync::Arc<crate::gameplay::component_state::declaration::DeclarationScheduling>,
}

pub(crate) enum BodyDefinition {
    Rigid(std::sync::Arc<Definition>),
    Throwable(std::sync::Arc<crate::assets::throwable::Definition>),
}
impl BodyDefinition {
    pub(crate) fn rigid(&self) -> &std::sync::Arc<Definition> {
        match self {
            Self::Rigid(value) => value,
            Self::Throwable(value) => &value.rigid,
        }
    }
}
pub(crate) struct Binding {
    pub entity: String,
    pub definition: BodyDefinition,
}

/// All concrete placed rigid-family declarations. Component identity and
/// inherited/inline overlays use the same resolver as the other factories.
pub(crate) fn bindings(
    catalog: &crate::archive::Catalog,
    document: &super::Document,
) -> Result<Vec<Binding>, String> {
    use super::{components, field, scope};
    use std::{collections::HashMap, sync::Arc};
    let declarations = document
        .definitions
        .iter()
        .filter(|d| d.kind == "component")
        .map(|d| (d.id.as_str(), d))
        .collect::<HashMap<_, _>>();
    let mut read = |path: &str| {
        catalog
            .read(path)
            .map_err(|e| e.to_string())
            .map(|bytes| bytes.map(super::owned_declaration_text))
    };
    let mut fields_cache = HashMap::new();
    let mut rigids: HashMap<String, Arc<Definition>> = HashMap::new();
    let mut throwables: HashMap<String, Arc<crate::assets::throwable::Definition>> = HashMap::new();
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let entries = scope(field(&entity.fields, "edit"))
            .and_then(|fields| scope(field(fields, "m_componentDecls")))
            .unwrap_or(&[]);
        let mut admitted = false;
        for (slot, value) in entries
            .iter()
            .filter(|(slot, _)| slot.starts_with("item_add["))
        {
            let Some(entry) = scope(Some(value)) else {
                continue;
            };
            let kind = components::entry_class(entry, &declarations)?;
            let Some(kind @ ("cpntrigidbody" | "cpntthrowable")) = kind.as_deref() else {
                continue;
            };
            if admitted {
                return Err(format!("{}: multiple rigid-family components", entity.id));
            }
            admitted = true;
            let identity = components::atom(entry, "mapComponentDecl")
                .map(|v| format!("map:{v}"))
                .or_else(|| {
                    field(entry, "inlineDecl")
                        .is_none()
                        .then(|| {
                            components::atom(entry, "inheritedDecl").map(|v| {
                                format!("inherited:{}", v.replace('\\', "/").to_ascii_lowercase())
                            })
                        })
                        .flatten()
                });
            let name = components::atom(entry, "mapComponentDecl")
                .or_else(|| components::atom(entry, "inheritedDecl"))
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{}:{slot}", entity.id));
            let fields =
                components::entry_fields(entry, kind, &declarations, &mut fields_cache, &mut read)?
                    .ok_or("rigid component class changed during resolution")?;
            let scheduling = Arc::new(components::scheduling(&fields,
                crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0))?);
            let definition = if kind == "cpntthrowable" {
                let value =
                    if let Some(shared) = identity.as_ref().and_then(|key| throwables.get(key)) {
                        shared.clone()
                    } else {
                        let value = crate::assets::throwable::Definition::from_resolved(
                            catalog, "", &name, &fields, scheduling,
                        )?;
                        if !catalog.has_overlays() {
                            if let Some(key) = identity {
                                throwables.insert(key, value.clone());
                            }
                        }
                        value
                    };
                BodyDefinition::Throwable(value)
            } else {
                let value = if let Some(shared) = identity.as_ref().and_then(|key| rigids.get(key))
                {
                    shared.clone()
                } else {
                    let value = Arc::new(Definition {
                        settings: from_resolved("", &name, &fields)?,
                        declaration_flags: declaration_flags(&fields)?,
                        scheduling,
                    });
                    if !catalog.has_overlays() {
                        if let Some(key) = identity {
                            rigids.insert(key, value.clone());
                        }
                    }
                    value
                };
                BodyDefinition::Rigid(value)
            };
            result.push(Binding {
                entity: entity.id.clone(),
                definition,
            });
        }
    }
    Ok(result)
}

pub(crate) fn from_resolved(
    entity: &str,
    declaration: &str,
    fields: &[(String, Value)],
) -> Result<RigidBodySettings, String> {
    let atom = |key: &str| -> Result<Option<&str>, String> {
        match fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
        {
            None => Ok(None),
            Some(Value::Atom(value)) => Ok(Some(value.trim_matches('"'))),
            Some(_) => Err(format!("rigid-body {declaration}: {key} must be an atom")),
        }
    };
    let number = |key: &str| -> Result<Option<f32>, String> {
        atom(key)?
            .map(|value| {
                value
                    .parse::<f32>()
                    .map_err(|_| format!("rigid-body {declaration}: invalid {key}: {value}"))
            })
            .transpose()
    };
    let flag = |key: &str, default: bool| -> Result<bool, String> {
        match atom(key)? {
            None => Ok(default),
            Some("true" | "1") => Ok(true),
            Some("false" | "0") => Ok(false),
            Some(value) => Err(format!("rigid-body {declaration}: invalid {key}: {value}")),
        }
    };
    let owned = |key: &str| -> Result<Option<String>, String> {
        Ok(atom(key)?
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
            .map(str::to_owned))
    };
    let mass_override = number("m_MassOverride")?;
    Ok(RigidBodySettings {
        entity_id: entity.to_owned(),
        declaration_id: declaration.to_owned(),
        shape_type: Some(owned("m_ShapeType")?.unwrap_or_else(|| "PHYSICS_SHAPE_CONVEX".into())),
        physics_behavior: Some(
            owned("m_PhysicsBehavior")?.unwrap_or_else(|| "BEHAVIOR_DYNAMIC".into()),
        ),
        extra_hd_material: owned("m_materialForExtraHDcollision")?,
        collision_layer: owned("m_CollisionLayer")?,
        material: owned("m_MaterialOverride")?,
        mass_override,
        descriptor_mass_override: mass_override.filter(|value| *value > 0.0),
        density_override: number("m_densityOverride")?,
        start_asleep: Some(flag("m_startAsleep", true)?),
        angular_damping: Some(number("m_angularDamping")?.unwrap_or(0.0)),
        wind_speed_factor: Some(number("m_windSpeedFactor")?.unwrap_or(0.0)),
        damage: owned("m_damage")?,
        velocity_damage_threshold: Some(number("m_velocityDamageThreshold")?.unwrap_or(0.1)),
        velocity_damage_threshold_when_dropped: Some(
            number("m_velocityDamageThresholdWhenDropped")?.unwrap_or(20.0),
        ),
        contact_impulse_max_damage: Some(number("m_contactImpulseMaxDamage")?.unwrap_or(10.0)),
        min_contact_impulse_for_damage: Some(
            number("m_minContactImpulseForDamage")?.unwrap_or(10.0),
        ),
        max_contact_impulse_for_damage: Some(
            number("m_maxContactImpulseForDamage")?.unwrap_or(100.0),
        ),
        lock_rotation: [
            flag("m_lockRotationX", false)?,
            flag("m_lockRotationY", false)?,
            flag("m_lockRotationZ", false)?,
        ],
        collision: Some(super::physics_material::collision_settings_fields(
            declaration,
            fields,
        )?),
        can_touch_trigger: flag("m_canTouchTrigger", false)?,
    })
}

/// Reflected declaration +0xc4 (1416fa750); constructor 806eb0 sets bit 6.
pub(crate) fn declaration_flags(fields: &[(String, Value)]) -> Result<u8, String> {
    let mut flags = 0x40;
    for (bit, key) in [
        "m_canTouchTrigger",
        "m_canGenerateObstacle",
        "m_canGenerateSilhouette",
        "m_lockRotationX",
        "m_lockRotationY",
        "m_lockRotationZ",
        "m_allowSelfDamage",
        "m_objectIsCritical",
    ]
    .into_iter()
    .enumerate()
    {
        let Some((_, value)) = fields.iter().find(|(name, _)| name == key) else {
            continue;
        };
        let Value::Atom(value) = value else {
            return Err(format!("rigid-body {key} must be an atom"));
        };
        match value.trim_matches('"') {
            "true" | "1" => flags |= 1 << bit,
            "false" | "0" => flags &= !(1 << bit),
            _ => return Err(format!("rigid-body {key} is not boolean")),
        }
    }
    Ok(flags)
}
