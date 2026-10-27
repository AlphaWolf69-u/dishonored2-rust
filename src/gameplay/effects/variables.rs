//! Per-instance Kiscule variables. Setup copies authored defaults into current
//! values (7bc710); entry refresh only replaces connected parent inputs.
use crate::{
    assets::entities::{effects::declarations::GraphVariable, event_graph::atom},
    gameplay::event_graph::EntityReference,
};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Clone, Debug)]
pub enum GraphValue {
    Bool(bool),
    Float(f32),
    Int(i32),
    Time(i32),
    Entity(Option<EntityReference>),
    Entities(Vec<EntityReference>),
}
impl GraphValue {
    pub fn read(
        &self,
        inputs: &impl crate::gameplay::event_graph::GraphInputs,
        node: u32,
        port: u32,
    ) -> Result<Self, String> {
        Ok(match self {
            Self::Bool(_) => Self::Bool(inputs.boolean(node, port, 0)?),
            Self::Float(_) => Self::Float(inputs.number(node, port, 0)?),
            Self::Int(_) => Self::Int(inputs.integer(node, port, 0)?),
            Self::Time(_) => Self::Time(inputs.integer(node, port, 0)?),
            Self::Entity(_) => {
                let mut entities = inputs.entity_references(node, port)?;
                if entities.len() > 1 {
                    return Err("graph variable requires one entity".into());
                }
                Self::Entity(entities.pop())
            }
            Self::Entities(_) => Self::Entities(inputs.entity_references(node, port)?),
        })
    }
    pub fn number(&self) -> Result<f32, String> {
        match self {
            Self::Bool(value) => Ok(f32::from(*value)),
            Self::Float(value) => Ok(*value),
            Self::Int(value) | Self::Time(value) => Ok(*value as f32),
            _ => Err("entity variable used as a number".into()),
        }
    }
    pub fn integer(&self) -> Result<i32, String> {
        match self {
            Self::Int(value) | Self::Time(value) => Ok(*value),
            _ => Err("non-integer graph variable used as an integer".into()),
        }
    }
    pub fn entities(&self) -> Result<Vec<EntityReference>, String> {
        match self {
            Self::Entity(value) => Ok(value.iter().cloned().collect()),
            Self::Entities(values) => Ok(values.clone()),
            _ => Err("non-entity graph variable used as an entity".into()),
        }
    }
}

pub type SharedVariables = Arc<RwLock<GraphVariables>>;
#[derive(Default)]
pub struct GraphVariables {
    values: HashMap<String, Result<GraphValue, String>>,
}
impl GraphVariables {
    pub fn new(definitions: &[GraphVariable]) -> Self {
        let mut values = HashMap::new();
        for definition in definitions {
            let name = atom(&definition.parameters, "m_name")
                .unwrap_or("")
                .to_owned();
            values.insert(name, default_value(definition));
        }
        Self { values }
    }
    pub fn get(&self, name: &str) -> Result<Option<GraphValue>, String> {
        self.values.get(name).cloned().transpose()
    }
    pub fn set(&mut self, name: String, value: GraphValue) {
        self.values.insert(name, Ok(value));
    }
}

fn default_value(definition: &GraphVariable) -> Result<GraphValue, String> {
    let fields = &definition.parameters;
    let value = atom(fields, "m_defaultValue");
    let invalid = || format!("invalid {} default", definition.class);
    match definition.class.as_str() {
        "arkKisculeGraphVariableBool" => match value.unwrap_or("false") {
            "true" => Ok(GraphValue::Bool(true)),
            "false" => Ok(GraphValue::Bool(false)),
            _ => Err(invalid()),
        },
        "arkKisculeGraphVariableFloat" => value
            .unwrap_or("0")
            .parse()
            .map(GraphValue::Float)
            .map_err(|_| invalid()),
        "arkKisculeGraphVariableInt" => value
            .unwrap_or("0")
            .parse()
            .map(GraphValue::Int)
            .map_err(|_| invalid()),
        "arkKisculeGraphVariableTime" => value
            .unwrap_or("0")
            .parse()
            .map(GraphValue::Time)
            .map_err(|_| invalid()),
        "arkKisculeGraphVariableEntityReference" => Ok(GraphValue::Entity(
            value
                .filter(|value| !value.is_empty())
                .map(|name| EntityReference::Named(name.to_owned())),
        )),
        "arkKisculeGraphVariableListEntityReference" => {
            // List initialization/reset uses the same installed list layout as
            // literal-node parameters (908fa0/90fb00 and90da00/91a1a0).
            crate::assets::entities::event_graph::entity_names(fields, "m_defaultValue").map(
                |values| {
                    GraphValue::Entities(values.into_iter().map(EntityReference::Named).collect())
                },
            )
        }
        class => Err(format!("graph variable type {class} is not connected")),
    }
}
