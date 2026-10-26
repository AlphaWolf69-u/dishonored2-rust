//! World particle FX graph execution. Links append in authored order and
//! ordinary actions pop newest-first (7beee0/7c10b0). The world owner supplies
//! real effect allocation, entity clocks and inter-graph queue phases.
use super::world_node::{CreatedEffect, FxNode, FxNodeWorld};
use crate::{
    assets::entities::{
        effects::{FxGraph, FxNodeParameters},
        event_graph::{atom, Node},
        Value,
    },
    gameplay::{
        event_gate::{GateContext, GateOutput, GateState},
        event_graph::GraphInputs,
    },
};
use std::collections::{HashMap, HashSet};

pub trait FxGraphWorld {
    fn effect_ready(&self, effect: &str) -> Result<bool, String>;
    fn entity_exists(&self, entity: &str) -> Result<bool, String>;
    fn spawn_effect(
        &mut self,
        owner: &str,
        node: u32,
        parameters: &FxNodeParameters,
        state: &mut FxNode,
        duration: f32,
    ) -> Result<Option<CreatedEffect>, String>;
    /// True only when the actual completion observer fires during this call.
    fn stop_effect(&mut self, owner: &str, node: u32, handle: u32) -> Result<bool, String>;
    fn entity_clock(&mut self, entity: u32) -> Result<Option<f64>, String>;
}
pub enum GraphStep {
    Idle,
    Action { schedule: Option<u32> },
}
pub struct FxGraphRuntime {
    graph: FxGraph,
    relevant: HashSet<u32>,
    pending: Vec<(u32, u32)>,
    random: HashMap<u32, f32>,
    named: HashMap<String, f32>,
    gates: HashMap<u32, GateState>,
    effects: HashMap<u32, FxNode>,
}
impl GraphInputs for FxGraphRuntime {
    fn graph_nodes(&self) -> &[Node] {
        &self.graph.nodes
    }
    fn graph_owner(&self) -> &str {
        &self.graph.owner
    }
    fn random_sample(&self, node: u32) -> Option<f32> {
        self.random.get(&node).copied()
    }
    fn named_value(&self, name: &str) -> Result<Option<f32>, String> {
        self.named
            .get(name)
            .copied()
            .map(Some)
            .ok_or_else(|| format!("world FX named variable {name} awaits its actual binding"))
    }
}
impl FxGraphRuntime {
    /// Random inputs come from the graph manager's existing RNG preparation.
    /// Constructing this domain owner does not seed another random generator.
    pub fn new(graph: FxGraph, random: HashMap<u32, f32>) -> Self {
        let mut relevant = graph.effects.keys().copied().collect::<HashSet<_>>();
        loop {
            let previous = relevant.len();
            for node in &graph.nodes {
                if relevant.contains(&node.id) {
                    for port in node.ports.iter().filter(|p| p.kind.starts_with("Input")) {
                        relevant.extend(port.links.iter().map(|(node, _)| *node));
                    }
                    if matches!(
                        node.class.as_str(),
                        "arkKisculeNodeNamedVariableFloat" | "arkKisculeNodeNamedVariableBool"
                    ) {
                        for writer in &graph.nodes {
                            if writer.class == node.class
                                && atom(&writer.params, "m_variableName")
                                    == atom(&node.params, "m_variableName")
                            {
                                relevant.insert(writer.id);
                            }
                        }
                    }
                }
            }
            if previous == relevant.len() {
                break;
            }
        }
        Self {
            graph,
            relevant,
            random,
            named: HashMap::new(),
            pending: Vec::new(),
            gates: HashMap::new(),
            effects: HashMap::new(),
        }
    }
    pub fn set_named_value(&mut self, name: String, value: f32) {
        self.named.insert(name, value);
    }
    fn emit(&mut self, id: u32, name: &str) -> Result<(), String> {
        let node = self.node(id)?;
        let links = node
            .ports
            .iter()
            .find(|p| p.name == name && p.kind.starts_with("Output"))
            .map(|p| p.links.clone())
            .unwrap_or_default();
        for (target, port) in links {
            if !self.relevant.contains(&target) {
                continue;
            }
            if self.node(target).ok().is_some_and(|n| {
                n.ports
                    .iter()
                    .any(|p| p.id == port && p.kind.starts_with("Input"))
            }) {
                self.pending.push((target, port));
            }
        }
        Ok(())
    }
    pub fn loading_end(&mut self) -> Result<(), String> {
        let nodes = self
            .graph
            .nodes
            .iter()
            .filter(|n| n.class == "arkKisculeNodeLoadingEnd")
            .map(|n| n.id)
            .collect::<Vec<_>>();
        for id in nodes {
            self.emit(id, "LevelLoadedFirstTime")?;
        }
        Ok(())
    }
    pub fn notify(&mut self, node: u32, port: u32) {
        if self.relevant.contains(&node) {
            self.pending.push((node, port));
        }
    }
    /// One queued action, without an invented per-frame event-count limit.
    /// The manager retains ownership of its queue phases and work budget.
    pub fn step(&mut self, world: &mut impl FxGraphWorld) -> Result<GraphStep, String> {
        let Some((id, port)) = self.pending.pop() else {
            return Ok(GraphStep::Idle);
        };
        let node = self.node(id)?.clone();
        let port = node
            .ports
            .iter()
            .find(|p| p.id == port)
            .ok_or("missing event port")?;
        let mut schedule = None;
        let paired_outputs = match node.class.as_str() {
            "arkKisculeNodeFx" => {
                let operation = match port.name.as_str() {
                    "Start" => FxOperation::Start,
                    "Stop" => FxOperation::Stop,
                    _ => return Err(format!("unsupported FX node input {}", port.name)),
                };
                let requested = self.invoke_fx(id, operation, world)?;
                if port.name == "Start" && requested {
                    schedule = Some(id);
                }
                true
            }
            "arkKisculeNodeGate" => {
                let mut gate = self.gates.remove(&id).unwrap_or_default();
                let mut context = GateInvocation { graph: self, id };
                let result = match port.name.as_str() {
                    "Test" => gate.pass(&mut context),
                    "Open" => gate.open(&mut context),
                    "Close" => gate.close(&mut context),
                    "Toggle" => gate.toggle(&mut context),
                    _ => Err(format!("unsupported gate input {}", port.name)),
                };
                self.gates.insert(id, gate);
                result?;
                true
            }
            // These routes use the existing scene event runtime's handlers.
            "arkKisculeNodeRelay" => {
                self.emit(id, "Out")?;
                false
            }
            "arkKisculeNodeActionTestBool" => {
                let value = self.boolean_input(&node, "m_value", false)?;
                self.emit(id, if value { "True" } else { "False" })?;
                false
            }
            "arkKisculeNodeNamedVariableFloat" if port.name == "Set" => {
                let value = self.number_input(&node, "m_value", 0., 0)?;
                let name = atom(&node.params, "m_variableName")
                    .unwrap_or("")
                    .to_owned();
                self.named.insert(name.clone(), value);
                let listeners = self
                    .graph
                    .nodes
                    .iter()
                    .filter(|n| {
                        n.class == node.class
                            && atom(&n.params, "m_variableName") == Some(name.as_str())
                    })
                    .map(|n| n.id)
                    .collect::<Vec<_>>();
                for listener in listeners {
                    self.emit(listener, "Updated")?;
                }
                false
            }
            _ => {
                return Err(format!(
                    "world FX event requires unsupported node {}",
                    node.class
                ))
            }
        };
        // 7c0e30 emits the corresponding regular output then Out, regardless
        // of whether the handler returns a task continuation (gates return0).
        if paired_outputs {
            self.emit(id, &port.name)?;
            self.emit(id, "Out")?;
        }
        Ok(GraphStep::Action { schedule })
    }
    fn invoke_fx(
        &mut self,
        id: u32,
        operation: FxOperation,
        world: &mut impl FxGraphWorld,
    ) -> Result<bool, String> {
        if !self.graph.effects.contains_key(&id) {
            return Err("FX node definition absent".into());
        }
        let mut state = self.effects.remove(&id).unwrap_or_default();
        let mut context = FxInvocation {
            graph: self,
            world,
            id,
        };
        let result = match operation {
            FxOperation::Start => state.start(&mut context).map(|_| state.scheduled()),
            FxOperation::Stop => state.stop(&mut context).map(|_| false),
            FxOperation::Update => state.update(&mut context),
            FxOperation::Completed(handle) => {
                state.effect_completed(handle, &mut context).map(|_| false)
            }
        };
        self.effects.insert(id, state);
        result
    }
    pub fn update_timer(
        &mut self,
        node: u32,
        world: &mut impl FxGraphWorld,
    ) -> Result<bool, String> {
        self.invoke_fx(node, FxOperation::Update, world)
    }
    pub fn effect_completed(
        &mut self,
        node: u32,
        handle: u32,
        world: &mut impl FxGraphWorld,
    ) -> Result<(), String> {
        self.invoke_fx(node, FxOperation::Completed(handle), world)
            .map(|_| ())
    }
    pub fn effect_started(&mut self, node: u32, handle: u32) -> Result<(), String> {
        if !self.graph.effects.contains_key(&node) {
            return Err("FX node definition absent".into());
        }
        self.effects.entry(node).or_default().effect_started(handle);
        Ok(())
    }
    fn entity_input(
        &self,
        node: &Node,
        name: &str,
        default: Option<String>,
    ) -> Result<Option<String>, String> {
        if let Some((id, port)) = self.input(node, name) {
            let mut entities = self.entities(id, port)?;
            if entities.len() > 1 {
                return Err(format!("FX {name} requires a single entity"));
            }
            Ok(entities.pop())
        } else {
            Ok(default)
        }
    }
}
enum FxOperation {
    Start,
    Stop,
    Update,
    Completed(u32),
}
struct GateInvocation<'a> {
    graph: &'a mut FxGraphRuntime,
    id: u32,
}
impl GateContext for GateInvocation<'_> {
    fn is_open(&mut self) -> Result<bool, String> {
        self.graph
            .boolean_input(self.graph.node(self.id)?, "m_isOpen", true)
    }
    fn auto_close_count(&mut self) -> Result<i32, String> {
        self.graph
            .integer_input(self.graph.node(self.id)?, "m_autoCloseCount", -1)
    }
    fn set_open(&mut self, value: bool) -> Result<(), String> {
        let node = self
            .graph
            .graph
            .nodes
            .iter_mut()
            .find(|n| n.id == self.id)
            .ok_or("gate node absent")?;
        let value = Value::Atom(if value { "true" } else { "false" }.into());
        if let Some((_, current)) = node.params.iter_mut().find(|(key, _)| key == "m_isOpen") {
            *current = value;
        } else {
            node.params.push(("m_isOpen".into(), value));
        }
        Ok(())
    }
    fn emit(&mut self, output: GateOutput) -> Result<(), String> {
        self.graph.emit(
            self.id,
            match output {
                GateOutput::Open => "Gate is Open",
                GateOutput::Closed => "Gate is Closed",
            },
        )
    }
}
struct FxInvocation<'a, W> {
    graph: &'a mut FxGraphRuntime,
    world: &'a mut W,
    id: u32,
}
impl<W: FxGraphWorld> FxNodeWorld for FxInvocation<'_, W> {
    fn duration_seconds(&mut self) -> Result<f32, String> {
        let node = self.graph.node(self.id)?;
        Ok(self.graph.integer_input(node, "m_duration", -1)? as f32 / 1000.)
    }
    fn spawn(
        &mut self,
        state: &mut FxNode,
        duration: f32,
    ) -> Result<Option<CreatedEffect>, String> {
        let node = self.graph.node(self.id)?;
        let mut parameters = self
            .graph
            .graph
            .effects
            .get(&self.id)
            .ok_or("FX parameters absent")?
            .clone();
        if self.graph.input(node, "m_fx").is_some() {
            return Err("connected FX declaration producer is not prepared".into());
        }
        parameters.location = self
            .graph
            .entity_input(node, "m_location", parameters.location)?;
        let effect_valid = parameters
            .effect
            .as_ref()
            .map(|p| self.world.effect_ready(p))
            .transpose()?
            .unwrap_or(false);
        let location_valid = parameters
            .location
            .as_ref()
            .map(|p| self.world.entity_exists(p))
            .transpose()?
            .unwrap_or(false);
        if !effect_valid || !location_valid || duration == 0. {
            return Ok(None);
        }
        for name in ["m_tagName", "m_targetTagName"] {
            if self.graph.input(node, name).is_some() {
                return Err(format!("connected FX {name} producer is not prepared"));
            }
        }
        parameters.target = self
            .graph
            .entity_input(node, "m_target", parameters.target)?;
        parameters.update_position =
            self.graph
                .boolean_input(node, "m_updatePosition", parameters.update_position)?;
        parameters.dont_spawn_if_culled = self.graph.boolean_input(
            node,
            "m_dontSpawnIfCulled",
            parameters.dont_spawn_if_culled,
        )?;
        parameters.stop_on_target_removal = self.graph.boolean_input(
            node,
            "m_stopOnTargetRemoval",
            parameters.stop_on_target_removal,
        )?;
        parameters.stop_on_parent_removal = self.graph.boolean_input(
            node,
            "m_stopOnParentRemoval",
            parameters.stop_on_parent_removal,
        )?;
        self.world.spawn_effect(
            &self.graph.graph.owner,
            self.id,
            &parameters,
            state,
            duration,
        )
    }
    fn stop_effect(&mut self, state: &mut FxNode, handle: u32) -> Result<(), String> {
        if self
            .world
            .stop_effect(&self.graph.graph.owner, self.id, handle)?
        {
            state.effect_completed(handle, self)?;
        }
        Ok(())
    }
    fn emit_completion(&mut self, _: &mut FxNode) -> Result<(), String> {
        self.graph.emit(self.id, "Finished")
    }
    fn entity_clock(&mut self, entity: u32) -> Result<Option<f64>, String> {
        self.world.entity_clock(entity)
    }
}
