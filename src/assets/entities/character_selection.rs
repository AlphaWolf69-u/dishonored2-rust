//! Host: fold deterministic character-dependent startup destruction before
//! acquiring placements/components. The original executes these authored
//! LoadingEnd -> declaration -> TestBool -> Destroy paths after loading.
//! No entity/item names or declaration names are used as selection rules.
use super::{event_graph::{atom, child, items, parse_node, Node}, Document};
use crate::archive::Catalog;
use std::collections::{HashMap, HashSet};

struct Projection<'a> {
    catalog: &'a Catalog,
    player: String,
    active_declarations: HashSet<String>,
    removed: HashMap<String, bool>,
}

fn input<'a>(nodes: &'a [Node], node: &Node, name: &str) -> Option<(&'a Node, u32)> {
    let (id, port) = node.ports.iter().find(|p| p.name == name && p.kind == "InputVariable")?.links.first()?;
    Some((nodes.iter().find(|node| node.id == *id)?, *port))
}

fn constant(node: &Node) -> bool {
    !node.ports.iter().any(|p| p.name == "Set" && p.kind == "InputRegular" && !p.links.is_empty())
}

impl Projection<'_> {
    // 79bc20 compares canonical declaration identities; a null operand yields
    // false for both equality and inequality. Unknown inputs are not defaults.
    fn condition(&self, nodes: &[Node], node: &Node) -> Option<(bool, bool)> {
        let (value, port) = input(nodes, node, "m_value")?;
        if value.class != "arkKisculeNodeCompareEntityDef" { return None; }
        let name = &value.ports.iter().find(|p| p.id == port)?.name;
        let (a, ap) = input(nodes, value, "m_a")?;
        let (b, bp) = input(nodes, value, "m_b")?;
        let (a, from_player_a) = self.entity_def(nodes, a, ap)?;
        let (b, from_player_b) = self.entity_def(nodes, b, bp)?;
        let equal = a.eq_ignore_ascii_case(&b);
        Some((match name.as_str() { "A == B" => equal, "A != B" => !equal, _ => return None },
            from_player_a || from_player_b))
    }

    fn entity_def(&self, nodes: &[Node], node: &Node, port: u32) -> Option<(String, bool)> {
        let point = node.ports.iter().find(|p| p.id == port)?;
        match node.class.as_str() {
            "arkKisculeNodeEntityDef" if point.name == "m_value" => {
                // A connected/mutable value cannot be folded as a constant.
                if !constant(node) || input(nodes, node, "m_value").is_some() { return None; }
                let name = atom(&node.params, "m_value")?;
                if name.is_empty() || name == "NULL" { return None; }
                Some((name.replace('\\', "/"), false))
            }
            "arkKisculeNodeEntityToEntityDef" if point.name == "EntityDef" => {
                let (source, port) = input(nodes, node, "m_entity")?;
                (source.class == "arkKisculeNodePlayer"
                    && source.ports.iter().any(|p| p.id == port && p.name == "Player"))
                    .then(|| (self.player.clone(), true))
            }
            _ => None,
        }
    }

    fn emit(nodes: &[Node], node: &Node, output: &str, dependent: bool, pending: &mut Vec<(u32, u32, bool)>) {
        for port in node.ports.iter().filter(|p| p.name == output && p.kind.starts_with("Output")) {
            pending.extend(port.links.iter().filter(|(id, _)| nodes.iter().any(|node| node.id == *id))
                .map(|(id, port)| (*id, *port, dependent)));
        }
    }

    fn run(&mut self, nodes: &[Node], entry: Option<(&str, bool)>) -> Result<Vec<(String, bool)>, String> {
        let mut pending = Vec::new();
        let mut exits = Vec::new();
        for node in nodes {
            match entry {
                None if node.class == "arkKisculeNodeLoadingEnd" =>
                    Self::emit(nodes, node, "LevelLoadedFirstTime", false, &mut pending),
                Some((name, dependent)) if node.class == "arkKisculeNodeDeclEntry"
                    && atom(&node.params, "m_entryName") == Some(name) => {
                        Self::emit(nodes, node, "Decl Entry", dependent, &mut pending);
                        // 7bb9d0's auto-exit defaults true, independently of
                        // explicit declaration exits and their notifications.
                        if atom(&node.params, "m_autoExit") != Some("false") {
                            exits.push((name.to_owned(), dependent));
                        }
                    }
                _ => {}
            }
        }
        // This projection admits only stateless nodes. Repeated identical
        // inputs therefore have identical destruction sets; cycles need no TTL.
        let mut visited = HashSet::new();
        while let Some((id, port, dependent)) = pending.pop() {
            if !visited.insert((id, port, dependent)) { continue; }
            let Some(node) = nodes.iter().find(|node| node.id == id) else { continue; };
            let Some(point) = node.ports.iter().find(|p| p.id == port) else { continue; };
            match node.class.as_str() {
                "arkKisculeNodeDeclInstance" => {
                    let Some(name) = super::effects::declarations::instance_declaration(node) else { continue; };
                    let key = name.replace('\\', "/").to_ascii_lowercase();
                    if !self.active_declarations.insert(key.clone()) { continue; }
                    let graph = super::effects::declarations::acquire_projection(self.catalog, name)?;
                    let child_exits = self.run(&graph.nodes, Some((&point.name, dependent)))?;
                    self.active_declarations.remove(&key);
                    for (name, dependent) in child_exits {
                        Self::emit(nodes, node, &name, dependent, &mut pending);
                    }
                }
                "arkKisculeNodeDeclExit" => {
                    if let Some(name) = atom(&node.params, "m_exitName") { exits.push((name.to_owned(), dependent)); }
                }
                "arkKisculeNodeActionTestBool" => {
                    if let Some((value, character)) = self.condition(nodes, node) {
                        Self::emit(nodes, node, if value { "True" } else { "False" }, dependent || character, &mut pending);
                        // 7c0e30 also emits the paired input and Out; their
                        // reachability does not depend on the tested value.
                        Self::emit(nodes, node, &point.name, dependent, &mut pending);
                        Self::emit(nodes, node, "Out", dependent, &mut pending);
                    }
                }
                "arkKisculeNodeActionDestroy" if dependent && point.name == "In" => {
                    // 792f40: a non-null spawn tag is a different lookup;
                    // do not pretend it is a constant entity reference.
                    if atom(&node.params, "m_spawnTag").is_some_and(|tag| tag != "SPAWN_TAG_NONE") { continue; }
                    let name = if let Some((value, port)) = input(nodes, node, "m_entity") {
                        if value.class != "arkKisculeNodeEntityReference" || !constant(value)
                            || !value.ports.iter().any(|p| p.id == port && p.name == "m_value")
                            || input(nodes, value, "m_value").is_some() { continue; }
                        atom(&value.params, "m_value")
                    } else { atom(&node.params, "m_entity") };
                    if let Some(name) = name.filter(|name| !name.is_empty() && *name != "NULL") {
                        let attached = atom(&node.params, "m_attachedObjects") != Some("false");
                        *self.removed.entry(name.to_owned()).or_default() |= attached;
                    }
                    Self::emit(nodes, node, &point.name, dependent, &mut pending);
                    Self::emit(nodes, node, "Out", dependent, &mut pending);
                }
                // Do not guess quest state, mutable variables, arbitrary
                // actions, timers or later gameplay events during acquisition.
                _ => {}
            }
        }
        Ok(exits)
    }
}

/// Fold only authored, player-dependent startup destruction. The temporary
/// projection and source leases die here; the surviving document is the sole
/// input to placements, interaction/collision admission and FX preparation.
pub fn prune(catalog: &Catalog, document: &mut Document, character: &str) -> Result<Vec<String>, String> {
    let mut projection = Projection { catalog, player: format!("models/characters/player/{character}/body.def"),
        active_declarations: HashSet::new(), removed: HashMap::new() };
    for entity in document.definitions.iter().filter(|d| d.kind == "entity") {
        let fields = child(child(&entity.fields, "edit"), "m_kiscule");
        if !items(child(fields, "m_nodes")).any(|node| atom(node, "m_name") == Some("arkKisculeNodeLoadingEnd")) { continue; }
        let nodes = items(child(fields, "m_nodes")).map(parse_node).collect::<Result<Vec<_>, _>>()?;
        projection.run(&nodes, None)?;
    }
    let binds = super::resolved_entity_binds(document);
    // Destroy-without-attachments requires runtime detachment. Leave that
    // case to its runtime owner rather than dropping a child's parent here.
    projection.removed.retain(|name, attached| *attached || !binds.iter().any(|bind| bind.parent == *name));
    loop {
        let children = binds.iter().filter(|bind| projection.removed.get(&bind.parent) == Some(&true)
            && !projection.removed.contains_key(&bind.entity_id)).map(|bind| bind.entity_id.clone()).collect::<Vec<_>>();
        if children.is_empty() { break; }
        for child in children { projection.removed.insert(child, true); }
    }
    let mut removed = Vec::new();
    document.definitions.retain(|definition| {
        let discard = definition.kind == "entity" && projection.removed.contains_key(&definition.id);
        if discard { removed.push(definition.id.clone()); }
        !discard
    });
    removed.sort();
    Ok(removed)
}
