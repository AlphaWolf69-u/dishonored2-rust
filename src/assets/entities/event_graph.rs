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
