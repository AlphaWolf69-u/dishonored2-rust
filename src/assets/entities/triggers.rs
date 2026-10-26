//! Trigger declarations and their owning entities, before physics allocation.
use super::{
    components::{atom, entry_class, entry_fields},
    field, scope, vector, Document, Value,
};
use crate::gameplay::trigger_body::{
    trigger_shape_request, TriggerCollisionSettings, TriggerShapeRequest,
};
use std::collections::HashMap;
#[path = "triggers/resources.rs"]
mod resources;
pub use resources::{TriggerAssets, TriggerShapeAsset};

pub struct TriggerBinding {
    pub entity_id: String,
    pub slot: String,
    pub collision: TriggerCollisionSettings,
    pub water_component: bool,
    pub wind_component: bool,
    pub scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling,
}

pub struct TriggerPlacement {
    pub binding: TriggerBinding,
    pub position: [f32; 3],
    pub shape: TriggerShapeRequest,
}

/// Resolve every trigger's authored pose and resource recipe before allocating
/// a runtime body. `map` is the world's map name, without the `maps/` prefix.
pub fn trigger_placements(
    document: &Document,
    map: &str,
    read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<TriggerPlacement>, String> {
    let entities: HashMap<_, _> = document
        .definitions
        .iter()
        .filter(|d| d.kind == "entity")
        .map(|d| (d.id.as_str(), d))
        .collect();
    trigger_bindings(document, read)?
        .into_iter()
        .map(|binding| {
            let entity = entities[binding.entity_id.as_str()];
            let edit = scope(field(&entity.fields, "edit")).unwrap();
            let read_vector =
                |fields: &[(String, Value)], name: &str, default| match field(fields, name) {
                    None => Ok(default),
                    Some(Value::Scope(values)) => vector(values, default)
                        .ok_or_else(|| format!("{}: invalid {name}", entity.id)),
                    Some(_) => Err(format!("{}: expected vector for {name}", entity.id)),
                };
            // idEntity constructor 874c50 calls 18df90: zero position and identity
            // basis. The scale is intentionally absent from 779530's shape recipe.
            let position = read_vector(edit, "spawnPosition", [0.; 3])?;
            let mut basis = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
            if let Some(orientation) = scope(field(edit, "spawnOrientation")) {
                if let Some(matrix) = scope(field(orientation, "mat")) {
                    for (i, axis) in basis.iter_mut().enumerate() {
                        *axis = read_vector(matrix, &format!("mat[{i}]"), *axis)?;
                    }
                }
            }
            let model = scope(field(edit, "renderModelInfo"))
                .and_then(|info| atom(info, "model"))
                .unwrap_or("");
            let shape = trigger_shape_request(model, map, &entity.id, basis);
            Ok(TriggerPlacement {
                binding,
                position,
                shape,
            })
        })
        .collect()
}

pub fn trigger_bindings(
    document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>,
) -> Result<Vec<TriggerBinding>, String> {
    let declarations = document
        .definitions
        .iter()
        .filter(|d| d.kind == "component")
        .map(|d| (d.id.as_str(), d))
        .collect::<HashMap<_, _>>();
    let mut cache = HashMap::new();
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let Some(edit) = scope(field(&entity.fields, "edit")) else {
            continue;
        };
        let Some(components) = scope(field(edit, "m_componentDecls")) else {
            continue;
        };
        let entries: Vec<_> = components
            .iter()
            .filter(|(key, _)| key.starts_with("item_add["))
            .filter_map(|(key, value)| scope(Some(value)).map(|fields| (key, fields)))
            .collect();
        let classes: Vec<_> = entries
            .iter()
            .map(|(_, entry)| entry_class(entry, &declarations))
            .collect::<Result<_, _>>()?;
        let water_component = classes
            .iter()
            .any(|class| class.as_deref() == Some("cpntwater"));
        let wind_component = classes
            .iter()
            .any(|class| class.as_deref() == Some("cpntwind"));
        for (slot, entry) in entries {
            let Some(fields) =
                entry_fields(entry, "cpnttrigger", &declarations, &mut cache, &mut read)?
            else {
                continue;
            };
            let defaults = TriggerCollisionSettings::default();
            let flag = |name, default| -> Result<bool, String> {
                match atom(&fields, name) {
                    None => Ok(default),
                    Some("true") => Ok(true),
                    Some("false") => Ok(false),
                    Some(value) => Err(format!("{}: invalid trigger {name}={value}", entity.id)),
                }
            };
            result.push(TriggerBinding {
                entity_id: entity.id.clone(),
                slot: slot.clone(),
                water_component,
                wind_component,
                scheduling: super::components::scheduling(&fields,
                    crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0))?,
                collision: TriggerCollisionSettings {
                    debris: flag("m_triggeredByDebris", defaults.debris)?,
                    rigid_bodies: flag("m_triggeredByRigidBodies", defaults.rigid_bodies)?,
                    npc_ragdolls: flag("m_triggeredByNpcRagdolls", defaults.npc_ragdolls)?,
                    npc_capsules: flag("m_triggeredByNpcCapsules", defaults.npc_capsules)?,
                    keyframed: flag("m_isKeyframed", defaults.keyframed)?,
                },
            });
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "triggers/tests.rs"]
mod tests;
