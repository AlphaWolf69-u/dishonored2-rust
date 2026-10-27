//! Shared serialized event-graph nodes. Parsing preserves graph order and
//! links; each gameplay owner decides which events and actions it executes.
use super::{field, scope, Value};

#[derive(Clone, Debug)]
pub struct Port {
    pub id: u32,
    pub name: String,
    pub kind: String,
    pub links: Vec<(u32, u32)>,
}
#[derive(Clone, Debug)]
pub struct Node {
    pub id: u32,
    pub class: String,
    pub params: Vec<(String, Value)>,
    pub ports: Vec<Port>,
}
pub fn child<'a>(fields: &'a [(String, Value)], name: &str) -> &'a [(String, Value)] {
    scope(field(fields, name)).unwrap_or(&[])
}
pub fn atom<'a>(fields: &'a [(String, Value)], name: &str) -> Option<&'a str> {
    match field(fields, name)? {
        Value::Atom(value) => Some(value.trim_matches('"')),
        _ => None,
    }
}
pub fn items(fields: &[(String, Value)]) -> impl Iterator<Item = &[(String, Value)]> {
    fields.iter().filter_map(|(name, value)| {
        name.starts_with("item[")
            .then(|| scope(Some(value)))
            .flatten()
    })
}
/// Serialized managed-entity lists retain index order, including omitted slots.
/// This consumer cannot turn a null element into a named entity or drop it.
pub fn entity_names(fields: &[(String, Value)], name: &str) -> Result<Vec<String>, String> {
    let fields = child(fields, name);
    let count = atom(fields, "num")
        .unwrap_or("0")
        .parse::<usize>()
        .map_err(|_| format!("invalid entity list {name} length"))?;
    let mut values = vec![None; count];
    for (key, value) in fields {
        let Some(index) = key
            .strip_prefix("item[")
            .and_then(|key| key.strip_suffix(']'))
        else {
            continue;
        };
        let index = index
            .parse::<usize>()
            .map_err(|_| "invalid entity list index")?;
        let slot = values
            .get_mut(index)
            .ok_or("entity list index exceeds length")?;
        let Value::Atom(value) = value else {
            return Err("entity list element is not a reference".into());
        };
        let value = value.trim_matches('"');
        *slot = (!value.is_empty()).then(|| value.to_owned());
    }
    values
        .into_iter()
        .map(|value| {
            value.ok_or_else(|| format!("entity list {name} contains a null managed reference"))
        })
        .collect()
}
pub fn parse_node(fields: &[(String, Value)]) -> Result<Node, String> {
    let ports = items(child(fields, "m_connectionPoints"))
        .map(|port| {
            let links = items(child(port, "m_connections"))
                .filter_map(|link| {
                    Some((
                        atom(link, "m_endNodeID")?.parse().ok()?,
                        atom(link, "m_endConnectionPointID")?.parse().ok()?,
                    ))
                })
                .collect();
            Ok(Port {
                id: atom(port, "m_id")
                    .and_then(|s| s.parse().ok())
                    .ok_or("invalid port ID")?,
                name: atom(port, "m_name").unwrap_or("").into(),
                kind: atom(port, "m_type").unwrap_or("").into(),
                links,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Node {
        id: atom(fields, "m_id")
            .and_then(|s| s.parse().ok())
            .ok_or("invalid node ID")?,
        class: atom(fields, "m_name").unwrap_or("").into(),
        params: child(child(fields, "m_parameters"), "m_object").to_vec(),
        ports,
    })
}
