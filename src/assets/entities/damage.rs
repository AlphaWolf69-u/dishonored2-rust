//! Resolved world health bindings. Component identity and authored slot stay
//! attached to their actual owning entity; no collider implies health.
use super::{components::{entry_class, entry_fields, scheduling}, field, scope, Document};
use crate::gameplay::{component_state::declaration::DeclarationScheduling, damage::Receiver};
use std::collections::HashMap;

pub struct HealthBinding {
    pub entity_id: String,
    pub slot: String,
    pub class: String,
    pub maximum: f32,
    pub asleep_health: f32,
    pub receiver: Receiver,
    pub scheduling: DeclarationScheduling,
}
/// Common entity component resolution owns inheritance and inline overrides.
/// 717580/718ae0 constructor defaults, reflected 14165df80/14165db50.
pub fn health_bindings(document: &Document,
    mut read: impl FnMut(&str) -> Result<Option<String>, String>) -> Result<Vec<HealthBinding>, String> {
    let declarations = document.definitions.iter().filter(|definition| definition.kind == "component").map(|definition| (definition.id.as_str(), definition)).collect::<HashMap<_, _>>();
    let mut cache = HashMap::new();
    let mut bindings = Vec::new();
    for entity in document.definitions.iter().filter(|definition| definition.kind == "entity") {
        let Some(entries) = scope(field(&entity.fields, "edit")).and_then(|edit| scope(field(edit, "m_componentDecls"))) else { continue };
        for (slot, value) in entries.iter().filter(|(key, _)| key.starts_with("item_add[")) {
            let Some(entry) = scope(Some(value)) else { continue };
            let Some(class) = entry_class(entry, &declarations)? else { continue };
            if !matches!(class.as_str(), "cpnthealth" | "cpnthealthevolved" | "cpnthealthdoor") { continue; }
            let Some(fields) = entry_fields(entry, &class, &declarations, &mut cache, &mut read)? else { continue };
            let number = |key, default| crate::gameplay::damage::number(&fields, key, default);
            bindings.push(HealthBinding { entity_id: entity.id.clone(), slot: slot.clone(), class,
                maximum: number("m_health", 1.0)?, asleep_health: number("m_asleepHealth", 1.0)?,
                receiver: Receiver::read(&fields)?,
                scheduling: scheduling(&fields, DeclarationScheduling::from_base_flags(0))? });
        }
    }
    Ok(bindings)
}
