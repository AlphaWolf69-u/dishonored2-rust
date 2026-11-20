//! Per-instance Kiscule variables. Setup copies authored defaults into current
//! values (7bc710); entry refresh only replaces connected parent inputs.
use crate::{
    assets::entities::{effects::declarations::GraphVariable, event_graph::{Node, atom}},
    gameplay::event_graph::{EntityReference, GraphInputs},
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
    Vector([f32; 3]),
    Entity(Option<EntityReference>),
    Entities(Vec<EntityReference>),
}
impl GraphValue {
    pub(super) fn read_input(
        &self,
        inputs: &impl GraphInputs,
        node: &Node,
        name: &str,
    ) -> Result<Option<Self>, String> {
        if matches!(self, Self::Entities(_)) {
            // 524300 collects all connected list and singleton producers, in port order.
            let links = node.ports.iter()
                .filter(|port| port.kind == "InputVariable" && port.name == name)
                .flat_map(|port| port.links.iter().copied());
            let mut values = None;
            for (source, port) in links {
                values.get_or_insert_with(Vec::new).extend(inputs.entity_references(source, port)?);
            }
            return Ok(values.map(Self::Entities));
        }
        inputs.input(node, name).map(|(source, port)| self.read(inputs, source, port)).transpose()
    }

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
            Self::Vector(_) => Self::Vector(inputs.vector(node, port, 0)?),
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
    pub fn vector(&self) -> Result<[f32; 3], String> {
        match self {
            Self::Vector(value) => Ok(*value),
            _ => Err("non-vector graph variable used as a vector".into()),
        }
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
            Self::Entity(value) => Ok(vec![value.clone().unwrap_or(EntityReference::Invalid)]),
            Self::Entities(values) => Ok(values.clone()),
            _ => Err("non-entity graph variable used as an entity".into()),
        }
    }
}

///909030/90bfa0 initialize both Vec3 storage slots to the executable's
///1432cdf40 zero vector. Authored fields retain the reflected x/y/z layout.
pub(crate) fn read_vector(fields: &[(String, crate::entities::Value)], name: &str) -> Result<[f32; 3], String> {
    let values = match crate::entities::field(fields, name) {
        None => &[][..],
        Some(value) => crate::entities::scope(Some(value)).ok_or_else(|| format!("invalid vector {name}"))?,
    };
    Ok([
        crate::gameplay::damage::number(values, "x", 0.)?,
        crate::gameplay::damage::number(values, "y", 0.)?,
        crate::gameplay::damage::number(values, "z", 0.)?,
    ])
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
        "arkKisculeGraphVariableVec3" => read_vector(fields, "m_defaultValue").map(GraphValue::Vector),
        "arkKisculeGraphVariableEntityReference" => Ok(GraphValue::Entity(
            value
                .filter(|value| !value.is_empty())
                .map(|name| EntityReference::Named(name.to_owned())),
        )),
        "arkKisculeGraphVariableListEntityReference" => {
            // List initialization/reset uses the same installed list layout as
            // literal-node parameters (908fa0/90fb00 and90da00/91a1a0).
            crate::assets::entities::event_graph::entity_reference_names(fields, "m_defaultValue")
                .map(|values| {
                    GraphValue::Entities(
                        values
                            .into_iter()
                            .map(|value| {
                                value.map_or(EntityReference::Invalid, EntityReference::Named)
                            })
                            .collect(),
                    )
                })
        }
        class => Err(format!("graph variable type {class} is not connected")),
    }
}
