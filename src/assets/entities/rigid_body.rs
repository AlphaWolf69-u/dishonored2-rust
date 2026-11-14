//! Reflected rigid-body fields from the resolved component declaration.
//! Missing values follow the original 140806eb0 constructor; malformed
//! authored values are errors rather than silently selecting a default.
use super::{RigidBodySettings, Value};

pub(crate) fn from_resolved(entity: &str, declaration: &str,
    fields: &[(String, Value)]) -> Result<RigidBodySettings, String>
{
    let atom = |key: &str| -> Result<Option<&str>, String> {
        match fields.iter().find(|(name, _)| name == key).map(|(_, value)| value) {
            None => Ok(None),
            Some(Value::Atom(value)) => Ok(Some(value.trim_matches('"'))),
            Some(_) => Err(format!("rigid-body {declaration}: {key} must be an atom")),
        }
    };
    let number = |key: &str| -> Result<Option<f32>, String> {
        atom(key)?.map(|value| value.parse::<f32>()
            .map_err(|_| format!("rigid-body {declaration}: invalid {key}: {value}"))).transpose()
    };
    let flag = |key: &str, default: bool| -> Result<bool, String> {
        match atom(key)? {
            None => Ok(default), Some("true" | "1") => Ok(true),
            Some("false" | "0") => Ok(false),
            Some(value) => Err(format!("rigid-body {declaration}: invalid {key}: {value}")),
        }
    };
    let owned = |key: &str| -> Result<Option<String>, String> {
        Ok(atom(key)?.filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL")).map(str::to_owned))
    };
    let mass_override = number("m_MassOverride")?;
    Ok(RigidBodySettings {
        entity_id: entity.to_owned(), declaration_id: declaration.to_owned(),
        shape_type: Some(owned("m_ShapeType")?.unwrap_or_else(|| "PHYSICS_SHAPE_CONVEX".into())),
        physics_behavior: Some(owned("m_PhysicsBehavior")?.unwrap_or_else(|| "BEHAVIOR_DYNAMIC".into())),
        extra_hd_material: owned("m_materialForExtraHDcollision")?,
        collision_layer: owned("m_CollisionLayer")?, material: owned("m_MaterialOverride")?,
        mass_override, descriptor_mass_override: mass_override.filter(|value| *value > 0.0),
        density_override: number("m_densityOverride")?,
        start_asleep: Some(flag("m_startAsleep", true)?),
        angular_damping: Some(number("m_angularDamping")?.unwrap_or(0.0)),
        wind_speed_factor: Some(number("m_windSpeedFactor")?.unwrap_or(0.0)),
        damage: owned("m_damage")?,
        velocity_damage_threshold: Some(number("m_velocityDamageThreshold")?.unwrap_or(0.1)),
        velocity_damage_threshold_when_dropped: Some(number("m_velocityDamageThresholdWhenDropped")?.unwrap_or(20.0)),
        contact_impulse_max_damage: Some(number("m_contactImpulseMaxDamage")?.unwrap_or(10.0)),
        min_contact_impulse_for_damage: Some(number("m_minContactImpulseForDamage")?.unwrap_or(10.0)),
        max_contact_impulse_for_damage: Some(number("m_maxContactImpulseForDamage")?.unwrap_or(100.0)),
        lock_rotation: [flag("m_lockRotationX", false)?, flag("m_lockRotationY", false)?, flag("m_lockRotationZ", false)?],
        collision: Some(super::physics_material::collision_settings_fields(declaration, fields)?),
        can_touch_trigger: flag("m_canTouchTrigger", false)?,
    })
}

/// Reflected declaration +0xc4 (1416fa750); constructor 806eb0 sets bit 6.
pub(crate) fn declaration_flags(fields: &[(String, Value)]) -> Result<u8, String> {
    let mut flags = 0x40;
    for (bit, key) in ["m_canTouchTrigger", "m_canGenerateObstacle", "m_canGenerateSilhouette", "m_lockRotationX", "m_lockRotationY", "m_lockRotationZ", "m_allowSelfDamage", "m_objectIsCritical"].into_iter().enumerate() {
        let Some((_, value)) = fields.iter().find(|(name, _)| name == key) else { continue };
        let Value::Atom(value) = value else { return Err(format!("rigid-body {key} must be an atom")) };
        match value.trim_matches('"') {
            "true" | "1" => flags |= 1 << bit,
            "false" | "0" => flags &= !(1 << bit),
            _ => return Err(format!("rigid-body {key} is not boolean")),
        }
    }
    Ok(flags)
}
