//! StoryFlag scalar nodes bind the world's existing declaration variable.
//! Bool924050/9241f0/926e60 and Int9042e0/9043e0/907270 use the same
//! qualified lookup5134e0 and notification7c07b0/7c0b50 owners. Bindings are
//! weak: a node cannot keep a retired graph or its current values resident.
use super::variables::{GraphValue, GraphVariables, SharedVariables};
use std::sync::{Arc, RwLock, Weak};

#[derive(Default)]
pub(super) struct State {
    pub bindings: std::collections::HashMap<u32,Binding>,
    pub updates: Vec<Binding>,
    pub ready: bool,
    pub named_ready: bool,
}

#[derive(Clone)]
pub(super) struct Binding {
    pub variables: Weak<RwLock<GraphVariables>>,
    pub name: Arc<str>,
    pub qualified: Arc<str>,
    pub target: u64,
    pub global: bool,
}
impl Binding {
    pub fn new(variables: &SharedVariables, name: &str, qualified: &str, target:u64, global:bool) -> Self {
        Self { variables: Arc::downgrade(variables), name: Arc::from(name), qualified: Arc::from(qualified), target, global }
    }
    pub fn get(&self) -> Result<Option<GraphValue>, String> {
        let Some(variables)=self.variables.upgrade() else {return Ok(None)};
        let value=variables.read().map_err(|_|"story variable owner poisoned")?.get(&self.name)?;
        Ok(value)
    }
    pub fn set(&self, value:GraphValue) -> Result<bool,String> {
        let Some(variables)=self.variables.upgrade() else {return Ok(false)};
        let mut variables=variables.write().map_err(|_|"story variable owner poisoned")?;
        let Some(previous)=variables.get(&self.name)? else {return Ok(false)};
        if std::mem::discriminant(&previous)!=std::mem::discriminant(&value) {
            return Err(format!("story variable {} type differs",self.name));
        }
        variables.set(self.name.to_string(),value);
        Ok(true)
    }
}

/// Concrete scalar types present in the admitted EotW startup graph. Other
/// StoryFlag value families keep their explicit unsupported-node boundary.
pub(super) fn kind(class:&str)->Option<GraphValue> {
    match class {
        "arkKisculeNodeStoryFlagBool"=>Some(GraphValue::Bool(false)),
        "arkKisculeNodeStoryFlagInt"=>Some(GraphValue::Int(0)),
        _=>None,
    }
}

pub(super) fn named_kind(class:&str)->Option<GraphValue> {
    match class {
        "arkKisculeNodeNamedVariableBool"=>Some(GraphValue::Bool(false)),
        "arkKisculeNodeNamedVariableInt"=>Some(GraphValue::Int(0)),
        "arkKisculeNodeNamedVariableFloat"=>Some(GraphValue::Float(0.)),
        "arkKisculeNodeNamedVariableVec3"=>Some(GraphValue::Vector([0.;3])),
        "arkKisculeNodeNamedVariableEntityReference"=>Some(GraphValue::Entity(None)),
        _=>None,
    }
}
