//! Installed Kiscule declarations shared by independently owned instances.
use super::{child, parse_graph, FxGraph};
use crate::{
    archive::Catalog,
    assets::entities::{
        event_graph::{atom, Node},
        Value,
    },
};
use std::{collections::{HashMap, HashSet}, sync::Arc};

/// Keep the reflected variable record intact until its runtime type consumes
/// it. Entity lists and other values must not be coerced into scalar defaults.
#[derive(Clone)]
pub struct GraphVariable {
    pub class: String,
    pub parameters: Vec<(String, Value)>,
}

#[derive(Clone, Default)]
pub struct FxDeclarations {
    /// First-reference order, including dependencies of nested declarations.
    pub declarations: Vec<Result<Arc<FxGraph>, String>>,
    indices: HashMap<String, usize>,
}

pub fn instance_declaration(node: &Node) -> Option<&str> {
    (node.class == "arkKisculeNodeDeclInstance")
        .then(|| atom(&node.params, "m_declKiscule"))
        .flatten()
        .filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL"))
}

impl FxDeclarations {
    pub fn load(roots: &[Arc<FxGraph>], catalog: &Catalog) -> Self {
        let mut declarations = Self::default();
        let mut bootstrap_dependencies = HashSet::new();
        for graph in roots {
            declarations.load_dependencies(graph, catalog, 0, &mut bootstrap_dependencies);
        }
        declarations
    }

    pub fn get(&self, name: &str) -> Result<&Arc<FxGraph>, String> {
        let index = self
            .indices
            .get(name)
            .ok_or_else(|| format!("Kiscule declaration was not prepared: {name}"))?;
        self.declarations[*index].as_ref().map_err(Clone::clone)
    }

    fn load_dependencies(&mut self, graph: &FxGraph, catalog: &Catalog, inherited_scope: u8,
        bootstrap_dependencies: &mut HashSet<String>) {
        for name in graph.nodes.iter().filter_map(instance_declaration) {
            if self.indices.contains_key(name) {
                // A graph reached later through a deferred bootstrap parent
                // must propagate that earlier phase too. Reserve the visit
                // before recursing, including when declarations form a cycle.
                if inherited_scope == 2 && bootstrap_dependencies.insert(name.to_owned()) {
                    if let Ok((declaration, scope)) = acquire_declaration(catalog, name, 2) {
                        self.load_dependencies(&declaration, catalog, scope, bootstrap_dependencies);
                    }
                }
                continue;
            }
            let index = self.declarations.len();
            self.indices.insert(name.to_owned(), index);
            self.declarations.push(Err(format!("Kiscule declaration is being prepared: {name}")));
            let parsed = acquire_declaration(catalog, name, inherited_scope).map(|(declaration, scope)| {
                if scope == 2 { bootstrap_dependencies.insert(name.to_owned()); }
                self.load_dependencies(&declaration, catalog, scope, bootstrap_dependencies);
                declaration
            });
            self.declarations[index] = parsed.map_err(|error| format!("{name}: {error}"));
        }
    }
}

fn resource_path(name: &str) -> String {
    format!("generated/decls/kiscule/{}.decl", name.replace('\\', "/").to_ascii_lowercase())
}
fn acquire_declaration(catalog: &Catalog, name: &str, inherited_scope: u8)
    -> Result<(Arc<FxGraph>, u8), String> {
    let path = resource_path(name);
    let shared = !catalog.has_overlays()
        && std::env::var("DH2_DEBUG_SEPARATE_GRAPH_RESOURCES").as_deref() != Ok("1");
    let trace = std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1");
    let mut registry = catalog.kiscule_resources.get_or_init(Default::default).lock()
        .map_err(|_| "Kiscule resource registry is poisoned")?;
    let mut decoded = false;
    let value = registry.acquire_with_sharing(&path, inherited_scope, shared, |_| {
        decoded = true;
        read_declaration(catalog, name).map(Arc::new)
    });
    let scope = registry.producer_scope(&path);
    if trace { eprintln!("FX_GRAPH_ACQUIRE path={path} decoded={decoded} shared={shared} scope={scope}"); }
    value.map(|graph| (graph, scope))
}

/// Borrow the canonical map-scoped definition for acquisition-time projection;
/// the same registry serves later runtime declaration instances.
pub(crate) fn acquire_projection(catalog: &Catalog, name: &str) -> Result<Arc<FxGraph>, String> {
    acquire_declaration(catalog, name, 0).map(|(graph, _)| graph)
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8, persistent: bool) -> Result<(), String> {
    let path = path.replace('\\', "/").to_ascii_lowercase();
    catalog.kiscule_resources.get_or_init(Default::default).lock()
        .map_err(|_| "Kiscule resource registry is poisoned")?.register(&path, marker, persistent);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(registry) = catalog.kiscule_resources.get() {
        registry.lock().map_err(|_| "Kiscule resource registry is poisoned")?.promote_bootstrap();
    }
    Ok(())
}
pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(registry) = catalog.kiscule_resources.get() {
        registry.lock().unwrap_or_else(|error| error.into_inner()).release(catalog.identity(), "kiscule", marker);
    }
}

fn read_declaration(catalog: &Catalog, name: &str) -> Result<FxGraph, String> {
    let _tag = crate::platform::allocation_tags::tag("FX graph definitions");
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
