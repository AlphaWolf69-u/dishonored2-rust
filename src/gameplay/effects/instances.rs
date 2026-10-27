//! Independently owned declaration instances with one shared action stack.
//! 7bc1d0 constructs children eagerly. 7bb9d0 routes entries/exits while
//! inserting notifications, before ordinary actions can be popped.
use super::graph::FxGraphRuntime;
use crate::{
    assets::entities::{
        effects::{
            declarations::{instance_declaration, FxDeclarations},
            FxGraph,
        },
        event_graph::atom,
    },
    gameplay::event_graph::GraphInputs,
};
use std::{
    collections::HashMap,
    ops::{Index, IndexMut},
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub struct FxGraphInstances {
    runtimes: Vec<Option<FxGraphRuntime>>,
    parents: Vec<Option<(usize, u32)>>,
    effectful: Vec<bool>,
    children: HashMap<(usize, u32), usize>,
    failures: HashMap<(usize, u32), String>,
    pub events: Vec<(usize, u32, u32)>,
}
impl Index<usize> for FxGraphInstances {
    type Output = Option<FxGraphRuntime>;
    fn index(&self, index: usize) -> &Self::Output {
        &self.runtimes[index]
    }
}
impl IndexMut<usize> for FxGraphInstances {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.runtimes[index]
    }
}
impl FxGraphInstances {
    pub fn new(roots: &[FxGraph], declarations: &FxDeclarations, seed: u64) -> Self {
        let random = Arc::new(Mutex::new(crate::random::MwcRandom::event_manager(seed)));
        let mut instances = Self::default();
        for root in roots {
            instances.add(root.clone(), None, declarations, &random, &mut Vec::new());
        }
        instances
    }
    fn add(
        &mut self,
        graph: FxGraph,
        parent: Option<(usize, u32)>,
        declarations: &FxDeclarations,
        random: &Arc<Mutex<crate::random::MwcRandom>>,
        path: &mut Vec<String>,
    ) -> usize {
        let children = graph
            .nodes
            .iter()
            .filter_map(|node| instance_declaration(node).map(|name| (node.id, name.to_owned())))
            .collect::<Vec<_>>();
        let owner = graph.owner.clone();
        let index = self.runtimes.len();
        let mut runtime = FxGraphRuntime::new(graph, HashMap::new());
        runtime.random_owner = Some(random.clone());
        let effectful = runtime.definition().nodes.iter().any(|node| {
            matches!(
                node.class.as_str(),
                "arkKisculeNodeFx" | "arkKisculeNodeEntityFx"
            )
        });
        let mut targets = runtime
            .definition()
            .nodes
            .iter()
            .filter(|node| {
                node.class == "arkKisculeNodeEntityFx"
                    || (parent.is_some() && node.class == "arkKisculeNodeDeclExit")
            })
            .map(|node| node.id)
            .collect::<Vec<_>>();
        self.runtimes.push(Some(runtime));
        self.parents.push(parent);
        self.effectful.push(effectful);
        for (node, name) in children {
            let definition = declarations.get(&name).and_then(|graph| {
                if path.contains(&name) {
                    Err(format!(
                        "recursive declaration instance: {} -> {name}",
                        path.join(" -> ")
                    ))
                } else {
                    Ok(graph.clone())
                }
            });
            match definition {
                Ok(mut graph) => {
                    graph.owner = owner.clone();
                    path.push(name);
                    let child = self.add(graph, Some((index, node)), declarations, random, path);
                    path.pop();
                    let variables = self[child].as_ref().unwrap().variables.clone();
                    self[index]
                        .as_mut()
                        .unwrap()
                        .children
                        .insert(node, variables);
                    self.children.insert((index, node), child);
                    if self.effectful[child] {
                        targets.push(node);
                        self.effectful[index] = true;
                    }
                }
                Err(error) => {
                    self.failures.insert((index, node), error);
                }
            }
        }
        // Only trace actions that lead to effects or a declaration return.
        // Unrelated doors, AI and physics remain with their gameplay owners.
        self[index].as_mut().unwrap().include_targets(targets);
        index
    }
    pub fn owner(&self, index: usize) -> &str {
        self[index].as_ref().expect("FX graph busy").graph_owner()
    }
    pub fn parent_value(
        &self,
        index: usize,
        name: &str,
        kind: &super::variables::GraphValue,
    ) -> Result<Option<super::variables::GraphValue>, String> {
        let Some((parent, instance)) = self.parents[index] else {
            return Ok(None);
        };
        let runtime = self[parent].as_ref().ok_or("parent FX graph busy")?;
        runtime
            .input(runtime.node(instance)?, name)
            .map(|(node, port)| kind.read(runtime, node, port))
            .transpose()
    }
    pub fn loading_end(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        for index in 0..self.runtimes.len() {
            if let Err(error) = self[index].as_mut().unwrap().loading_end() {
                errors.push(format!("{}: {error}", self.owner(index)));
            }
            errors.extend(self.collect(index));
        }
        errors
    }
    pub fn collect(&mut self, index: usize) -> Vec<String> {
        let mut errors = Vec::new();
        let pending = self[index].as_mut().unwrap().take_pending();
        for (node, port) in pending {
            if let Err(error) = self.insert(index, node, port) {
                errors.push(format!("{} node {node}: {error}", self.owner(index)));
            }
        }
        errors
    }
    fn insert(&mut self, index: usize, id: u32, port: u32) -> Result<(), String> {
        let node = self[index]
            .as_ref()
            .ok_or("FX graph busy")?
            .node(id)?
            .clone();
        if node.class == "arkKisculeNodeDeclInstance" {
            if let Some(error) = self.failures.get(&(index, id)) {
                return Err(error.clone());
            }
            let Some(child) = self.children.get(&(index, id)).copied() else {
                // An empty declaration has no child: 7bb9d0 falls through to
                // ordinary dispatch instead of inventing an auto-exit.
                self.events.push((index, id, port));
                return Ok(());
            };
            let name = &node
                .ports
                .iter()
                .find(|point| point.id == port)
                .ok_or("declaration input port absent")?
                .name;
            let entries = self[child]
                .as_ref()
                .unwrap()
                .definition()
                .nodes
                .iter()
                .filter(|node| {
                    node.class == "arkKisculeNodeDeclEntry"
                        && atom(&node.params, "m_entryName").unwrap_or("") == name
                })
                .cloned()
                .collect::<Vec<_>>();
            let mut auto_exit = false;
            let mut refreshed = false;
            for entry in &entries {
                let runtime = self[child].as_ref().unwrap();
                auto_exit |= runtime.boolean_parameter(entry, "m_autoExit", true)?;
                if !refreshed
                    && runtime.boolean_parameter(entry, "m_refreshGlobalVariables", true)?
                {
                    let notify =
                        runtime.boolean_parameter(entry, "m_sendUpdateNotifications", true)?;
                    let variables = runtime.definition().variables.clone();
                    for variable in variables {
                        let mut runtime = self[child].take().unwrap();
                        let result =
                            runtime.refresh_variable(self[index].as_ref().unwrap(), id, &variable);
                        self[child] = Some(runtime);
                        result?;
                        if notify {
                            self[child].as_mut().unwrap().variable_updated(
                                atom(&variable.parameters, "m_name").unwrap_or(""),
                            )?;
                            self.flush(child)?;
                        }
                    }
                    refreshed = true;
                }
            }
            for entry in entries {
                self[child].as_mut().unwrap().emit(entry.id, "Decl Entry")?;
                self.flush(child)?;
            }
            if auto_exit {
                self[index].as_mut().unwrap().emit(id, name)?;
                self.flush(index)?;
            }
        } else if node.class == "arkKisculeNodeDeclExit" {
            if let Some((parent, instance)) = self.parents[index] {
                self[parent]
                    .as_mut()
                    .ok_or("parent FX graph busy")?
                    .emit(instance, atom(&node.params, "m_exitName").unwrap_or(""))?;
                self.flush(parent)?;
            }
        } else {
            self.events.push((index, id, port));
        }
        Ok(())
    }
    fn flush(&mut self, index: usize) -> Result<(), String> {
        for (node, port) in self[index].as_mut().unwrap().take_pending() {
            self.insert(index, node, port)?;
        }
        Ok(())
    }
}
