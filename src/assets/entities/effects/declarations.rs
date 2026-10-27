//! Installed Kiscule declarations shared by independently owned instances.
use super::{child, parse_graph, FxGraph};
use crate::{
    archive::Catalog,
    assets::entities::{
        event_graph::{atom, Node},
        Value,
    },
};
use std::collections::HashMap;

/// Keep the reflected variable record intact until its runtime type consumes
/// it. Entity lists and other values must not be coerced into scalar defaults.
#[derive(Clone)]
pub struct GraphVariable {
    pub class: String,
    pub parameters: Vec<(String, Value)>,
}

#[derive(Default)]
pub struct FxDeclarations {
    /// First-reference order, including dependencies of nested declarations.
    pub declarations: Vec<Result<FxGraph, String>>,
    indices: HashMap<String, usize>,
}

pub fn instance_declaration(node: &Node) -> Option<&str> {
    (node.class == "arkKisculeNodeDeclInstance")
        .then(|| atom(&node.params, "m_declKiscule"))
        .flatten()
        .filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL"))
}

impl FxDeclarations {
    pub fn load(roots: &[FxGraph], catalog: &Catalog) -> Self {
        let mut declarations = Self::default();
        for graph in roots {
            declarations.load_dependencies(graph, catalog);
        }
        declarations
    }

    pub fn get(&self, name: &str) -> Result<&FxGraph, String> {
        let index = self
            .indices
            .get(name)
            .ok_or_else(|| format!("Kiscule declaration was not prepared: {name}"))?;
        self.declarations[*index].as_ref().map_err(Clone::clone)
    }

    fn load_dependencies(&mut self, graph: &FxGraph, catalog: &Catalog) {
        for name in graph.nodes.iter().filter_map(instance_declaration) {
            if self.indices.contains_key(name) {
                continue;
            }
            let index = self.declarations.len();
            // Reserve identity before reading dependencies. Repeated references
            // share immutable data, including recursive declaration references;
            // runtime instances retain their own node/variable/queue state.
            self.indices.insert(name.to_owned(), index);
            self.declarations.push(Err(format!(
                "Kiscule declaration is being prepared: {name}"
            )));
            let parsed = read_declaration(catalog, name);
            if let Ok(declaration) = &parsed {
                self.load_dependencies(declaration, catalog);
            }
            self.declarations[index] = parsed.map_err(|error| format!("{name}: {error}"));
        }
    }
}

fn read_declaration(catalog: &Catalog, name: &str) -> Result<FxGraph, String> {
    let mut failure = None;
    let mut read = |path: &str| match catalog.read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            failure = Some(format!("{path}: {error}"));
            None
        }
    };
    // 14138ca50 registers the kiscule declaration family. Reuse the ordinary
    // installed declaration reader, including its inherited partial edits.
    let fields = crate::gameplay::options::decl::read_edit(&mut read, "kiscule", name);
    if let Some(failure) = failure {
        return Err(failure);
    }
    let fields = fields?;
    let graph = child(&fields, "m_kiscule");
    parse_graph(name.to_owned(), graph)
}
