//! Shared variable and entity inputs for authored event graphs. Domain owners
//! retain node state and decide which actions to execute.
use crate::assets::entities::event_graph::{atom, Node};

/// Runtime entity identity, separate from serialized map names. Player resolves
/// through the active player owner (7a8500), never a fabricated map entity name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EntityReference {
    Named(String),
    Player,
}

pub trait GraphInputs {
    fn graph_nodes(&self) -> &[Node];
    fn graph_owner(&self) -> &str;
    fn random_sample(&self, node: u32) -> Option<f32>;
    fn named_value(&self, name: &str) -> Result<Option<f32>, String>;
    /// Instance owners expose typed named and declaration outputs here. Other
    /// graph domains retain their existing variable readers.
    fn variable_value(
        &self,
        _node: &Node,
        _port: u32,
        _depth: usize,
    ) -> Result<Option<super::effects::variables::GraphValue>, String> {
        Ok(None)
    }
    fn node(&self, id: u32) -> Result<&Node, String> {
        self.graph_nodes()
            .iter()
            .find(|n| n.id == id)
            .ok_or_else(|| format!("missing node {id}"))
    }
    fn input(&self, node: &Node, name: &str) -> Option<(u32, u32)> {
        node.ports
            .iter()
            .find(|p| p.name == name && p.kind == "InputVariable")
            .and_then(|p| p.links.first())
            .copied()
    }
    fn number(&self, id: u32, port: u32, depth: usize) -> Result<f32, String> {
        if depth > 64 {
            return Err("cyclic variable graph".into());
        }
        let n = self.node(id)?;
        let p = n
            .ports
            .iter()
            .find(|p| p.id == port)
            .ok_or("missing variable port")?;
        if p.kind == "InputVariable" {
            return if let Some((n, p)) = p.links.first() {
                self.number(*n, *p, depth + 1)
            } else {
                atom(&n.params, &p.name)
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| format!("invalid number {}", p.name))
            };
        }
        if let Some(value) = self.variable_value(n, port, depth)? {
            return value.number();
        }
        match n.class.as_str() {
            "arkKisculeNodeRandomFloat" => self
                .random_sample(id)
                .ok_or("random node not initialized".into()),
            "arkKisculeNodeFloat" | "arkKisculeNodeInt" | "arkKisculeNodeTime" => {
                atom(&n.params, "m_value")
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| "invalid constant".into())
            }
            "arkKisculeNodeMultiplyFloat" => Ok(self.number_input(n, "m_a", 0., depth + 1)?
                * self.number_input(n, "m_b", 0., depth + 1)?),
            "arkKisculeNodeNamedVariableFloat" => Ok(self
                .named_value(atom(&n.params, "m_variableName").unwrap_or(""))?
                .unwrap_or(0.)),
            "arkKisculeNodeNamedVariableBool" => Ok(self
                .named_value(atom(&n.params, "m_variableName").unwrap_or(""))?
                .unwrap_or(0.)),
            "arkKisculeNodeBool" => Ok(f32::from(atom(&n.params, "m_value") == Some("true"))),
            _ => Err(format!("unsupported variable node {}", n.class)),
        }
    }
    fn number_input(
        &self,
        n: &Node,
        name: &str,
        default: f32,
        depth: usize,
    ) -> Result<f32, String> {
        if let Some((n, p)) = self.input(n, name) {
            self.number(n, p, depth + 1)
        } else {
            atom(&n.params, name).map_or(Ok(default), |v| {
                v.parse().map_err(|_| format!("invalid {name}"))
            })
        }
    }
    fn entity_references(&self, id: u32, port: u32) -> Result<Vec<EntityReference>, String> {
        self.entity_references_inner(id, port, 0)
    }
    fn entity_references_inner(
        &self,
        id: u32,
        port: u32,
        depth: usize,
    ) -> Result<Vec<EntityReference>, String> {
        if depth > 64 {
            return Err("cyclic variable graph".into());
        }
        let node = self.node(id)?;
        let input = node
            .ports
            .iter()
            .find(|point| point.id == port)
            .ok_or("missing entity variable port")?;
        if input.kind == "InputVariable" {
            if let Some((id, port)) = input.links.first() {
                return self.entity_references_inner(*id, *port, depth + 1);
            }
        }
        if let Some(value) = self.variable_value(node, port, depth)? {
            return value.entities();
        }
        if node.class == "arkKisculeNodePlayer" {
            return Ok(vec![EntityReference::Player]);
        }
        self.entities(id, port)
            .map(|names| names.into_iter().map(EntityReference::Named).collect())
    }
    fn entities(&self, id: u32, port: u32) -> Result<Vec<String>, String> {
        let n = self.node(id)?;
        match n.class.as_str() {
            "arkKisculeNodeListEntityReference" => {
                crate::assets::entities::event_graph::entity_names(&n.params, "m_value")
            }
            "arkKisculeNodeEntityReference" => Ok(atom(&n.params, "m_value")
                .filter(|v| !v.is_empty())
                .map(|v| vec![v.to_owned()])
                .unwrap_or_default()),
            "arkKisculeNodeOwner" => Ok(vec![self.graph_owner().to_owned()]),
            other => Err(format!("unsupported entity binding {other}, port {port}")),
        }
    }
    fn boolean_parameter(&self, node: &Node, name: &str, default: bool) -> Result<bool, String> {
        match atom(&node.params, name) {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(_) => Err(format!("invalid boolean {name}")),
        }
    }
    fn boolean(&self, id: u32, port: u32, depth: usize) -> Result<bool, String> {
        if depth > 64 {
            return Err("cyclic variable graph".into());
        }
        let node = self.node(id)?;
        let port = node
            .ports
            .iter()
            .find(|p| p.id == port)
            .ok_or("missing variable port")?;
        if port.kind == "InputVariable" {
            if let Some((id, port)) = port.links.first() {
                return self.boolean(*id, *port, depth + 1);
            }
        }
        if node.class == "arkKisculeNodeGate" && port.name == "m_isOpen" {
            return self.boolean_parameter(node, "m_isOpen", true);
        }
        Ok(self.number(id, port.id, depth)? != 0.)
    }
    fn boolean_input(&self, node: &Node, name: &str, default: bool) -> Result<bool, String> {
        if let Some((id, port)) = self.input(node, name) {
            self.boolean(id, port, 0)
        } else {
            self.boolean_parameter(node, name, default)
        }
    }
    fn integer(&self, id: u32, port: u32, depth: usize) -> Result<i32, String> {
        if depth > 64 {
            return Err("cyclic variable graph".into());
        }
        let node = self.node(id)?;
        let port = node
            .ports
            .iter()
            .find(|p| p.id == port)
            .ok_or("missing variable port")?;
        if port.kind == "InputVariable" {
            if let Some((id, port)) = port.links.first() {
                return self.integer(*id, *port, depth + 1);
            }
            return atom(&node.params, &port.name)
                .ok_or("integer input default is not prepared")?
                .parse()
                .map_err(|_| format!("invalid integer {}", port.name));
        }
        if let Some(value) = self.variable_value(node, port.id, depth)? {
            return value.integer();
        }
        match node.class.as_str() {
            "arkKisculeNodeInt" | "arkKisculeNodeTime" => atom(&node.params, "m_value")
                .ok_or("integer literal default is not prepared")?
                .parse()
                .map_err(|_| "invalid integer literal".into()),
            "arkKisculeNodeGate" if port.name == "m_autoCloseCount" => {
                atom(&node.params, "m_autoCloseCount")
                    .unwrap_or("-1")
                    .parse()
                    .map_err(|_| "invalid gate count".into())
            }
            _ => Err(format!("unsupported integer binding {}", node.class)),
        }
    }
    fn integer_input(&self, node: &Node, name: &str, default: i32) -> Result<i32, String> {
        if let Some((id, port)) = self.input(node, name) {
            self.integer(id, port, 0)
        } else {
            atom(&node.params, name).map_or(Ok(default), |value| {
                value.parse().map_err(|_| format!("invalid integer {name}"))
            })
        }
    }
}
