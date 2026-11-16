//! Immutable Throwable declaration projection. Constructor4c6690,
//! reflection1416fa2c0. Uses the existing resolved-declaration/material owners.
use crate::{
    entities::{self, Value},
    gameplay::{component_state::declaration::DeclarationScheduling, throwable::Settings},
};
use std::sync::Arc;
#[derive(Debug)]
pub(crate) struct Definition {
    pub rigid: Arc<entities::rigid_body::Definition>,
    pub settings: Settings,
    /// The base physics damage declaration is acquired with its owner, never
    /// read or parsed from inside a live contact callback (807170/8071e0).
    pub damage: Option<Arc<crate::assets::damage::Declaration>>,
}
impl Definition {
    pub(crate) fn from_resolved(
        catalog: &crate::archive::Catalog,
        entity: &str,
        declaration: &str,
        fields: &[(String, Value)],
        scheduling: Arc<DeclarationScheduling>,
    ) -> Result<Arc<Self>, String> {
        let flag = |name: &str| -> Result<bool, String> {
            match entities::scene_motion::atom(fields, name) {
                None | Some("false" | "0") => Ok(false),
                Some("true" | "1") => Ok(true),
                Some(value) => Err(format!("{declaration}: invalid{name}: {value}")),
            }
        };
        let number = |name: &str, default: f32| -> Result<f32, String> {
            match entities::scene_motion::atom(fields, name) {
                None => Ok(default),
                Some(value) => value
                    .parse::<f32>()
                    .ok()
                    .filter(|v| v.is_finite())
                    .ok_or_else(|| format!("{declaration}: invalid{name}: {value}")),
            }
        };
        let attached = entities::scene_motion::child(fields, "m_collisionSettingsAttached");
        // Reuse the canonical reflection/version converter under its expected
        // field name; this temporary declaration projection is not retained.
        let attached_collision = entities::physics_material::collision_settings_fields(
            declaration,
            &[(
                "m_collisionSettings".into(),
                Value::Scope(attached.to_vec()),
            )],
        )?;
        let damage = entities::scene_motion::atom(fields, "m_stickDamageInHead")
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
            .map(|name| crate::assets::damage::Declaration::acquire(catalog, name))
            .transpose()?;
        let contact_damage = entities::scene_motion::atom(fields, "m_damage")
            .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
            .map(|name| crate::assets::damage::Declaration::acquire(catalog, name))
            .transpose()?;
        Ok(Arc::new(Self {
            damage: contact_damage,
            rigid: Arc::new(entities::rigid_body::Definition {
                settings: entities::rigid_body::from_resolved(entity, declaration, fields)?,
                declaration_flags: entities::rigid_body::declaration_flags(fields)?,
                scheduling,
            }),
            settings: Settings {
                high_quality: flag("m_highQualityPhysics")?,
                sticky: flag("m_sticky")?,
                notify_hit: flag("m_notifyHit")?,
                player_damage_using_raycast: flag("m_playerDamageUsingRaycast")?,
                attached_collision,
                stick_distance: number("m_stickDistFromContactPos", 0.)?,
                head_damage: damage,
                damping_after_first_hit: number("m_angularDampingAfterFirstHit", 0.)?,
                dropped_reset_distance: number("m_minDistanceToResetDroppedFlag", 2.)?,
            },
        }))
    }
}
