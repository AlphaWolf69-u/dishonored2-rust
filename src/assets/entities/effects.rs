//! World FX event-graph declarations. Loading a graph does not activate its
//! effects: the event runtime must reach the authored Start input first.
use super::{
    event_graph::{parse_node, Node},
    field, scope, Document, Value,
};
use std::collections::HashMap;
pub mod declarations;

/// 79e100/17debb0: arkKisculeNodeFxParameters, before graph-input overrides.
#[derive(Clone, Debug)]
pub struct FxNodeParameters<E = String> {
    pub effect: Option<String>,
    pub location: Option<E>,
    pub tag: String,
    pub target: Option<E>,
    pub target_tag: String,
    pub duration_ms: i32,
    pub update_position: bool,
    pub dont_spawn_if_culled: bool,
    pub stop_on_target_removal: bool,
    pub stop_on_parent_removal: bool,
}
impl<E> Default for FxNodeParameters<E> {
    fn default() -> Self {
        Self {
            effect: None,
            location: None,
            tag: String::new(),
            target: None,
            target_tag: String::new(),
            duration_ms: -1,
            update_position: false,
            dont_spawn_if_culled: false,
            stop_on_target_removal: false,
            stop_on_parent_removal: true,
        }
    }
}
impl<E> FxNodeParameters<E> {
    pub fn map_entities<U>(self, mut map: impl FnMut(E) -> U) -> FxNodeParameters<U> {
        FxNodeParameters {
            effect: self.effect, location: self.location.map(&mut map), tag: self.tag,
            target: self.target.map(map), target_tag: self.target_tag,
            duration_ms: self.duration_ms, update_position: self.update_position,
            dont_spawn_if_culled: self.dont_spawn_if_culled,
            stop_on_target_removal: self.stop_on_target_removal,
            stop_on_parent_removal: self.stop_on_parent_removal,
        }
    }
}
impl FxNodeParameters {
    pub fn parse(fields: &[(String, Value)]) -> Result<Self, String> {
        let mut result = Self::default();
        for (name, value) in fields {
            let Value::Atom(value) = value else {
                return Err(format!("FX node parameter {name} requires an atom"));
            };
            let value = value.trim_matches('"');
            let reference = || {
                (!value.is_empty() && !value.eq_ignore_ascii_case("NULL")).then(|| value.to_owned())
            };
            let boolean = || match value {
                "true" => Ok(true),
                "false" => Ok(false),
                _ => Err(format!("invalid FX node boolean {name}")),
            };
            match name.as_str() {
                "m_fx" => result.effect = reference(),
                "m_location" => result.location = reference(),
                "m_tagName" => result.tag = value.to_owned(),
                "m_target" => result.target = reference(),
                "m_targetTagName" => result.target_tag = value.to_owned(),
                "m_duration" => {
                    result.duration_ms = value
                        .parse()
                        .map_err(|e| format!("FX node duration: {e}"))?
                }
                "m_updatePosition" => result.update_position = boolean()?,
                "m_dontSpawnIfCulled" => result.dont_spawn_if_culled = boolean()?,
                "m_stopOnTargetRemoval" => result.stop_on_target_removal = boolean()?,
                "m_stopOnParentRemoval" => result.stop_on_parent_removal = boolean()?,
                _ => return Err(format!("unknown FX node parameter {name}")),
            }
        }
        Ok(result)
    }
}

#[derive(Clone)]
pub struct FxGraph {
    pub owner: String,
    pub nodes: Vec<Node>,
    pub effects: HashMap<u32, FxNodeParameters>,
    pub variables: Vec<declarations::GraphVariable>,
}
fn child<'a>(fields: &'a [(String, Value)], name: &str) -> &'a [(String, Value)] {
    scope(field(fields, name)).unwrap_or(&[])
}
pub(super) fn parse_graph(owner: String, fields: &[(String, Value)]) -> Result<FxGraph, String> {
    let nodes = child(fields, "m_nodes").iter()
        .filter(|(name, _)| name.starts_with("item["))
        .filter_map(|(_, value)| scope(Some(value)))
        .map(parse_node).collect::<Result<Vec<_>, _>>()?;
    let effects = nodes.iter().filter(|node| node.class == "arkKisculeNodeFx")
        .map(|node| FxNodeParameters::parse(&node.params).map(|parameters| (node.id, parameters)))
        .collect::<Result<HashMap<_, _>, _>>()?;
    let variables = super::event_graph::items(child(fields, "m_variables"))
        .map(|variable| declarations::GraphVariable {
            class: super::event_graph::atom(variable, "m_class").unwrap_or("").to_owned(),
            parameters: child(variable, "m_object").to_vec(),
        }).collect();
    Ok(FxGraph { owner, nodes, effects, variables })
}

pub fn load(document: &Document) -> Result<Vec<FxGraph>, String> {
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let fields = child(child(&entity.fields, "edit"), "m_kiscule");
        let relevant = super::event_graph::items(child(fields, "m_nodes")).any(|node|
            matches!(super::event_graph::atom(node, "m_name"),
                Some("arkKisculeNodeFx" | "arkKisculeNodeDeclInstance")));
        if relevant {
            result.push(parse_graph(entity.id.clone(), fields)?);
        }
    }
    Ok(result)
}
