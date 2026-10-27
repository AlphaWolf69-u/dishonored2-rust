//! Placed idEntityFx declarations. These persistent scene actors are distinct
//! from the attached entities created by Kiscule NodeFx actions.
use crate::{
    archive::Catalog,
    assets::entities::{
        event_graph::{atom, child},
        Document, Value,
    },
};
use std::collections::HashMap;

#[derive(Clone)]
pub struct ActorDefinition {
    pub owner: String,
    pub parameters: Result<ActorParameters, String>,
}

/// Constructor87e2c0 and reflected members14170e190.
#[derive(Clone, Debug, Default)]
pub struct ActorParameters {
    pub start_off: bool,
    pub cycle_trigger: bool,
    pub action_delay: f32,
    pub update_position: bool,
    pub remove_on_end: bool,
    pub target: Option<String>,
    pub target_tag: String,
    pub stop_on_target_removal: bool,
    pub effect: Option<String>,
}
impl ActorParameters {
    pub fn parse(fields: &[(String, Value)]) -> Result<Self, String> {
        let reference = |name| {
            atom(fields, name)
                .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                .map(str::to_owned)
        };
        let boolean = |name| match atom(fields, name) {
            None | Some("false") => Ok(false),
            Some("true") => Ok(true),
            Some(_) => Err(format!("invalid FX actor boolean {name}")),
        };
        Ok(Self {
            start_off: boolean("startOff")?,
            cycle_trigger: boolean("cycleTrigger")?,
            action_delay: atom(fields, "actionDelay")
                .unwrap_or("0")
                .parse()
                .map_err(|_| "invalid FX actor actionDelay")?,
            update_position: boolean("updateFxPosition")?,
            remove_on_end: boolean("removeEntityOnFxEnd")?,
            target: reference("m_targetEntity"),
            target_tag: atom(fields, "m_targetTagName").unwrap_or("").to_owned(),
            stop_on_target_removal: boolean("m_stopOnTargetRemoval")?,
            effect: reference("fxEffect"),
        })
    }
}

pub fn load(document: &Document, catalog: &Catalog) -> Vec<ActorDefinition> {
    let mut inherited = HashMap::<String, Result<Vec<(String, Value)>, String>>::new();
    document
        .definitions
        .iter()
        .filter(|entity| {
            entity.kind == "entity" && atom(&entity.fields, "class") == Some("idEntityFx")
        })
        .map(|entity| {
            let parameters = (|| {
                let mut fields = if let Some(parent) =
                    atom(&entity.fields, "inherit").filter(|name| !name.is_empty())
                {
                    inherited
                        .entry(parent.to_owned())
                        .or_insert_with(|| {
                            let mut failure = None;
                            let mut read = |path: &str| match catalog.read(path) {
                                Ok(bytes) => bytes,
                                Err(error) => {
                                    failure = Some(format!("{path}: {error}"));
                                    None
                                }
                            };
                            let result = crate::gameplay::options::decl::read_edit(
                                &mut read,
                                "entitydef",
                                parent,
                            );
                            failure.map_or(result, Err)
                        })
                        .clone()?
                } else {
                    Vec::new()
                };
                super::super::components::overlay(&mut fields, child(&entity.fields, "edit"));
                ActorParameters::parse(&fields)
            })()
            .map_err(|error| format!("{}: {error}", entity.id));
            ActorDefinition {
                owner: entity.id.clone(),
                parameters,
            }
        })
        .collect()
}
