//! arkDeclCpntWhaleOilTank741980; reflection141779340.
use crate::{
    entities::Value,
    gameplay::damage::{DamageType, atom, number},
};
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct Definition {
    pub maximum_energy: i32,
    pub starting_energy: i32,
    pub clip_smooth: f32,
    pub pusher: Option<Arc<str>>,
    pub maximum_offset: f32,
    pub low_offset: f32,
    pub delayed_types: DamageType,
    pub delayed_seconds: f32,
    pub window_tag: Arc<str>,
}

pub(crate) struct Binding {
    pub entity: String,
    pub definition: Arc<Definition>,
    pub scheduling: crate::gameplay::component_state::declaration::DeclarationScheduling,
}
pub(crate) fn bindings(
    catalog: &crate::archive::Catalog,
    document: &crate::entities::Document,
) -> Result<Vec<Binding>, String> {
    use crate::entities::{
        components,
        event_graph::{atom, child},
        scope,
    };
    let declarations = document
        .definitions
        .iter()
        .filter(|entry| entry.kind == "component")
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let mut cache = std::collections::HashMap::new();
    let mut typed = std::collections::HashMap::<String, Arc<Definition>>::new();
    let mut result = Vec::new();
    let mut read =
        |path: &str| crate::assets::declaration_texts::read_common_or_catalog(catalog, path);
    for entity in document
        .definitions
        .iter()
        .filter(|entry| entry.kind == "entity")
    {
        for (slot, value) in child(child(&entity.fields, "edit"), "m_componentDecls") {
            if !slot.starts_with("item_add[") {
                continue;
            }
            let Some(entry) = scope(Some(value)) else {
                continue;
            };
            let Some(fields) = components::entry_fields(
                entry,
                "cpntwhaleoiltank",
                &declarations,
                &mut cache,
                &mut read,
            )?
            else {
                continue;
            };
            let key = if let Some(name) = atom(entry, "mapComponentDecl") {
                format!("map:{name}")
            } else if crate::entities::field(entry, "inlineDecl").is_none() {
                format!("decl:{}", atom(entry, "inheritedDecl").unwrap_or(""))
            } else {
                format!("{}:{slot}", entity.id)
            };
            let definition = if let Some(definition) = typed.get(&key) {
                definition.clone()
            } else {
                let definition = Arc::new(
                    Definition::read(&fields)
                        .map_err(|error| format!("{} {slot}: {error}", entity.id))?,
                );
                typed.insert(key, definition.clone());
                definition
            };
            result.push(Binding{entity:entity.id.clone(),definition,scheduling:components::scheduling(&fields,
                crate::gameplay::component_state::declaration::DeclarationScheduling::from_base_flags(0))?});
        }
    }
    Ok(result)
}
impl Definition {
    pub(crate) fn read(fields: &[(String, Value)]) -> Result<Self, String> {
        let integer = |key, default| {
            atom(fields, key).map_or(Ok(default), |value| {
                value
                    .parse::<i32>()
                    .map_err(|_| format!("invalid WhaleOilTank {key}: {value}"))
            })
        };
        let timer = fields
            .iter()
            .find(|(key, _)| key == "m_delayedExplosionTimer")
            .map(|(_, value)| value);
        let delayed_seconds = match timer {
            Some(Value::Scope(fields)) => number(fields, "m_seconds", 1.)?,
            Some(Value::Atom(value)) => value
                .trim_end_matches('s')
                .parse::<f32>()
                .map_err(|_| "invalid WhaleOilTank explosion timer")?,
            None => 1.,
        };
        if !delayed_seconds.is_finite() {
            return Err("nonfinite WhaleOilTank timer".into());
        }
        Ok(Self {
            maximum_energy: integer("m_maximumEnergy", 100)?,
            starting_energy: integer("m_startingEnergy", 100)?,
            clip_smooth: number(fields, "m_renderParamClipSmoothValue", 0.05)?,
            pusher: atom(fields, "m_pusherDef")
                .filter(|value| !value.is_empty() && *value != "NULL")
                .map(Arc::from),
            maximum_offset: number(fields, "m_maxEnergyOffset", 0.)?,
            low_offset: number(fields, "m_lowEnergyOffset", -0.3)?,
            delayed_types: DamageType::read(
                atom(fields, "m_delayedExplosionDamagesTypes").unwrap_or("0"),
            )?,
            delayed_seconds,
            window_tag: Arc::from(atom(fields, "m_windowObservableTag").unwrap_or("")),
        })
    }
}
