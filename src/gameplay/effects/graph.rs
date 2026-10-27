//! World particle FX graph execution. Links append in authored order and
//! ordinary actions pop newest-first (7beee0/7c10b0). The world owner supplies
//! real effect allocation, entity clocks and inter-graph queue phases.
use super::variables::{GraphValue, GraphVariables, SharedVariables};
use super::world_node::{CreatedEffect, FxNode, FxNodeWorld};
use crate::{
    assets::entities::{
        effects::{FxGraph, FxNodeParameters},
        event_graph::{atom, Node},
        Value,
    },
    gameplay::{
        event_gate::{GateContext, GateOutput, GateState},
        event_graph::{EntityReference, GraphInputs},
    },
};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, RwLock},
};

pub type FxRuntimeParameters = FxNodeParameters<EntityReference>;

pub trait FxGraphWorld {
    fn entity_effect_ready(&self, _entity: &EntityReference) -> Result<bool, String> {
        Err("scene FX actor playback is not connected".into())
    }
    fn start_entity_effect(
        &mut self,
        _node: u32,
        _entity: &EntityReference,
        _target: Option<&EntityReference>,
        _target_tag: &str,
    ) -> Result<Option<u32>, String> {
        Err("scene FX actor playback is not connected".into())
    }
    fn stop_entity_effect(
        &mut self,
        _node: u32,
        _entity: &EntityReference,
    ) -> Result<Option<u32>, String> {
        Err("scene FX actor playback is not connected".into())
    }
    fn parent_value(&self, _name: &str, _kind: &GraphValue) -> Result<Option<GraphValue>, String> {
        Ok(None)
    }
    fn effect_ready(&self, effect: &str) -> Result<bool, String>;
    fn entity_exists(&self, entity: &EntityReference) -> Result<bool, String>;
    fn spawn_effect(
        &mut self,
        owner: &str,
        node: u32,
        parameters: &FxRuntimeParameters,
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
    pub(super) random_owner: Option<Arc<Mutex<crate::random::MwcRandom>>>,
    pub(super) variables: SharedVariables,
    pub(super) children: HashMap<u32, SharedVariables>,
    gates: HashMap<u32, GateState>,
    effects: HashMap<u32, FxNode>,
    entity_effects: HashMap<u32, super::entity_node::EntityFxNode>,
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
        let value = self.variables
            .read()
            .map_err(|_| "graph variable owner poisoned")?
            .get(name)?
            .ok_or_else(|| format!("world FX named variable {name} awaits its actual binding"))?;
        value.number().map(Some)
    }
    fn variable_value(
        &self,
        node: &Node,
        port: u32,
        depth: usize,
    ) -> Result<Option<GraphValue>, String> {
        let point = node
            .ports
            .iter()
            .find(|point| point.id == port)
            .ok_or("graph variable port absent")?;
        if let Some(value) = super::values::evaluate(self, node, &point.name, depth)? {
            return Ok(Some(value));
        }
        if node.class == "arkKisculeNodeRandomFloat" {
            if let Some(random) = &self.random_owner {
                // 522a10 invokes the producer's +c0 getter on every read;
                // 8bcce0 then advances the shared event-manager generator.
                let min = self.number_input(node, "m_min", 0., depth + 1)?;
                let max = self.number_input(node, "m_max", 1., depth + 1)?;
                return Ok(Some(GraphValue::Float(
                    random
                        .lock()
                        .map_err(|_| "graph random owner poisoned")?
                        .between(min, max),
                )));
            }
        }
        let (variables, name) = if node.class.starts_with("arkKisculeNodeNamedVariable") {
            (
                &self.variables,
                atom(&node.params, "m_variableName").unwrap_or(""),
            )
        } else if node.class == "arkKisculeNodeDeclInstance" {
            let variables = self
                .children
                .get(&node.id)
                .ok_or("declaration variable instance is not available")?;
            let port = node
                .ports
                .iter()
                .find(|point| point.id == port)
                .ok_or("declaration variable port is absent")?;
            (variables, port.name.as_str())
        } else {
            return Ok(None);
        };
        variables
            .read()
            .map_err(|_| "graph variable owner poisoned")?
            .get(name)
    }
}
impl FxGraphRuntime {
    /// Random inputs come from the graph manager's existing RNG preparation.
    /// Constructing this domain owner does not seed another random generator.
    pub fn new(graph: FxGraph, random: HashMap<u32, f32>) -> Self {
        let relevant = Self::relevant_nodes(&graph, graph.effects.keys().copied().collect());
        let variables = Arc::new(RwLock::new(GraphVariables::new(&graph.variables)));
        Self {
            graph,
            relevant,
            random,
            random_owner: None,
            variables,
            children: HashMap::new(),
            pending: Vec::new(),
            gates: HashMap::new(),
            effects: HashMap::new(),
            entity_effects: HashMap::new(),
        }
    }
    fn relevant_nodes(graph: &FxGraph, mut relevant: HashSet<u32>) -> HashSet<u32> {
        loop {
            let previous = relevant.len();
            for node in &graph.nodes {
                if relevant.contains(&node.id) {
                    for port in node.ports.iter().filter(|p| p.kind.starts_with("Input")) {
                        relevant.extend(port.links.iter().map(|(node, _)| *node));
                    }
                    if node.class.starts_with("arkKisculeNodeNamedVariable") {
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
        relevant
    }
    pub(super) fn include_targets(&mut self, targets: impl IntoIterator<Item = u32>) {
        self.relevant.extend(targets);
        self.relevant = Self::relevant_nodes(&self.graph, std::mem::take(&mut self.relevant));
    }
    pub fn set_named_value(&mut self, name: String, value: f32) {
        self.variables
            .write()
            .expect("graph variable owner poisoned")
            .set(name, GraphValue::Float(value));
    }
    pub(super) fn definition(&self) -> &FxGraph {
        &self.graph
    }
    pub(super) fn refresh_variable(
        &mut self,
        parent: &Self,
        instance: u32,
        definition: &crate::assets::entities::effects::declarations::GraphVariable,
    ) -> Result<(), String> {
        let node = parent.node(instance)?;
        let name = atom(&definition.parameters, "m_name").unwrap_or("");
        if let Some((id, port)) = parent.input(node, name) {
            let current = self
                .variables
                .read()
                .map_err(|_| "graph variable owner poisoned")?
                .get(name)?
                .ok_or("graph variable absent")?;
            let value = current.read(parent, id, port)?;
            self.variables
                .write()
                .map_err(|_| "graph variable owner poisoned")?
                .set(name.to_owned(), value);
        }
        Ok(())
    }
    pub(super) fn variable_updated(&mut self, name: &str) -> Result<(), String> {
        let listeners = self
            .graph
            .nodes
            .iter()
            .filter(|node| {
                node.class.starts_with("arkKisculeNodeNamedVariable")
                    && atom(&node.params, "m_variableName") == Some(name)
            })
            .map(|node| node.id)
            .collect::<Vec<_>>();
        for listener in listeners {
            self.emit(listener, "Updated")?;
        }
        Ok(())
    }
    /// The shared manager owns inter-graph ordering. Export append order so it
    /// can process the resulting global stack, rather than drain each graph.
    pub fn take_pending(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.pending)
    }
    pub(super) fn emit(&mut self, id: u32, name: &str) -> Result<(), String> {
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
            "arkKisculeNodeEntityFx" => {
                let reference = |name| {
                    atom(&node.params, name)
                        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                        .map(|name| EntityReference::Named(name.to_owned()))
                };
                let entity = self.entity_input(&node, "m_entity", reference("m_entity"))?;
                if let Some(entity) = entity {
                    match port.name.as_str() {
                        "Start" => {
                            if world.entity_effect_ready(&entity)? {
                                let target =
                                    self.entity_input(&node, "m_target", reference("m_target"))?;
                                if self.input(&node, "m_targetTagName").is_some() {
                                    return Err(
                                        "connected scene FX target tag producer is not prepared"
                                            .into(),
                                    );
                                }
                                let tag = atom(&node.params, "m_targetTagName").unwrap_or("");
                                if let Some(actor) =
                                    world.start_entity_effect(id, &entity, target.as_ref(), tag)?
                                {
                                    self.entity_effects.entry(id).or_default().started(actor);
                                }
                            }
                        }
                        "Stop" => {
                            if let Some(actor) = world.stop_entity_effect(id, &entity)? {
                                self.entity_effects.entry(id).or_default().stopped(actor);
                            }
                        }
                        _ => return Err(format!("unsupported scene FX input {}", port.name)),
                    }
                }
                true
            }
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
            "arkKisculeNodeNamedVariableFloat"
            | "arkKisculeNodeNamedVariableBool"
            | "arkKisculeNodeNamedVariableInt"
            | "arkKisculeNodeNamedVariableEntityReference"
                if port.name == "Set" =>
            {
                let name = atom(&node.params, "m_variableName")
                    .unwrap_or("")
                    .to_owned();
                let kind = match node.class.as_str() {
                    "arkKisculeNodeNamedVariableBool" => GraphValue::Bool(false),
                    "arkKisculeNodeNamedVariableInt" => GraphValue::Int(0),
                    "arkKisculeNodeNamedVariableEntityReference" => GraphValue::Entity(None),
                    _ => GraphValue::Float(0.),
                };
                // 926e60 reads an explicit Value link first, then the parent
                // declaration input with the variable's own name. An absent
                // link leaves the current value untouched.
                let value = if let Some((id, port)) = self.input(&node, "m_value") {
                    Some(kind.read(self, id, port)?)
                } else {
                    world.parent_value(&name, &kind)?
                };
                if let Some(value) = value {
                    let bound = self
                        .variables
                        .read()
                        .map_err(|_| "graph variable owner poisoned")?
                        .get(&name)?
                        .is_some();
                    if !bound {
                        self.emit(id, "Set")?;
                        self.emit(id, "Out")?;
                        return Err(format!("named variable {name} is not bound"));
                    }
                    self.variables
                        .write()
                        .map_err(|_| "graph variable owner poisoned")?
                        .set(name.clone(), value);
                    self.variable_updated(&name)?;
                } else {
                    // The native warning does not suppress regular forwarding.
                    self.emit(id, "Set")?;
                    self.emit(id, "Out")?;
                    return Err(format!("named variable {name}: Set has no Value link"));
                }
                true
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
    pub fn entity_effect_completed(&mut self, node: u32, actor: u32) -> Result<(), String> {
        self.entity_effects.entry(node).or_default().finished(actor);
        self.emit(node, "Finished")
    }
    fn entity_input(
        &self,
        node: &Node,
        name: &str,
        default: Option<EntityReference>,
    ) -> Result<Option<EntityReference>, String> {
        if let Some((id, port)) = self.input(node, name) {
            let mut entities = self.entity_references(id, port)?;
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
            .clone()
            .map_entities(EntityReference::Named);
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
