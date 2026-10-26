//! World FX event-graph declarations. Loading a graph does not activate its
//! effects: the event runtime must reach the authored Start input first.
use super::{
    event_graph::{parse_node, Node},
    field, scope, Document, Value,
};
use std::collections::HashMap;

/// 79e100/17debb0: arkKisculeNodeFxParameters, before graph-input overrides.
#[derive(Clone, Debug)]
pub struct FxNodeParameters {
    pub effect: Option<String>,
    pub location: Option<String>,
    pub tag: String,
    pub target: Option<String>,
    pub target_tag: String,
    pub duration_ms: i32,
    pub update_position: bool,
    pub dont_spawn_if_culled: bool,
    pub stop_on_target_removal: bool,
    pub stop_on_parent_removal: bool,
}
impl Default for FxNodeParameters {
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
}
fn child<'a>(fields: &'a [(String, Value)], name: &str) -> &'a [(String, Value)] {
    scope(field(fields, name)).unwrap_or(&[])
}
pub fn load(document: &Document) -> Result<Vec<FxGraph>, String> {
    let mut result = Vec::new();
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let nodes = child(child(child(&entity.fields, "edit"), "m_kiscule"), "m_nodes");
        let fields = nodes
            .iter()
            .filter(|(name, _)| name.starts_with("item["))
            .filter_map(|(_, value)| scope(Some(value)))
            .collect::<Vec<_>>();
        let is_fx = |fields: &&[(String, Value)]| {
            matches!(field(fields, "m_name"), Some(Value::Atom(name))
                if name.trim_matches('"') == "arkKisculeNodeFx")
        };
        if !fields.iter().any(is_fx) {
            continue;
        }
        let nodes = fields
            .into_iter()
            .map(parse_node)
            .collect::<Result<Vec<_>, _>>()?;
        let effects = nodes
            .iter()
            .filter(|node| node.class == "arkKisculeNodeFx")
            .map(|node| {
                FxNodeParameters::parse(&node.params).map(|parameters| (node.id, parameters))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;
        result.push(FxGraph {
            owner: entity.id.clone(),
            nodes,
            effects,
        });
    }
    Ok(result)
}
