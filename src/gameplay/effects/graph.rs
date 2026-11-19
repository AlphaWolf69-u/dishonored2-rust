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

#[derive(Clone, Copy, Debug)]
pub enum UsableAction {
    Stage { stage: Option<usize>, instant: bool },
    SetAvailability(bool),
    Remove { attached: bool },
}

pub trait FxGraphWorld {
    fn request_usable_action(&mut self, _entity: crate::gameplay::entity_lifetime::EntityId, _action: UsableAction) -> Result<(), String> {
        Err("graph usable component owner is unavailable".into())
    }
    fn graph_clock(&self, _owner: &str) -> Result<f64, String> {
        Err("graph owner clock is unavailable".into())
    }
    fn managed_entity(&self,_entity:&EntityReference)->Result<Option<crate::gameplay::entity_lifetime::EntityId>,String>{Err("managed entity resolver is not connected".into())}
    fn managed_frame(&self,_entity:crate::gameplay::entity_lifetime::EntityId)->Result<Option<crate::gameplay::reference_frame::ReferenceFrame>,String>{Err("managed frame resolver is not connected".into())}
    fn managed_spawn_tag(&self,_entity:crate::gameplay::entity_lifetime::EntityId)->Result<String,String>{Err("managed spawn tag resolver is not connected".into())}
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
    UsableReset { node: u32 },
    PickupReset { node: u32 },
    BreakableReset { node: u32 },
    Idle,
    Action { schedule: Option<u32> },
}
pub struct FxGraphRuntime {
    graph: Arc<FxGraph>,
    owner: String,
    relevant: HashSet<u32>,
    pending: Vec<(u32, u32)>,
    random: HashMap<u32, f32>,
    pub(super) random_owner: Option<Arc<Mutex<crate::random::MwcRandom>>>,
    pub(super) variables: SharedVariables,
    pub(super) story: Option<Box<super::story_flags::State>>,
    pub(super) children: HashMap<u32, SharedVariables>,
    delays: Option<Box<HashMap<u32, Delay>>>,
    gates: HashMap<u32, GateState>,
    gate_open: HashMap<u32, bool>,
    effects: HashMap<u32, FxNode>,
    entity_effects: HashMap<u32, super::entity_node::EntityFxNode>,
    notifications: Option<Box<HashMap<u32, NotificationState>>>,
    gameflow: Option<Arc<crate::gameplay::gameflow::program::Mailbox>>,
    gameflow_subscriptions: Vec<usize>,
    spawner: Option<Arc<super::spawner::EntitySpawner>>,
    /// 14079dc40's per-node once flag (+0x30) and m_createdEntity.
    spawned: HashMap<u32, Option<EntityReference>>,
    instance_id:Option<u64>,
    spawn_attempt:u64,
    pending_spawns:HashMap<u32,u64>,
}
#[derive(Default)]
struct Delay {
    remaining_ms: i32,
    last_time: Option<f64>,
    running: bool,
}
#[derive(Default)]
struct NotificationState { count: i32, instigator: Option<EntityReference>, step: i32, location: [f32; 3], usable_stage: Option<i32> }
impl GraphInputs for FxGraphRuntime {
    fn boolean_override(&self, node: &Node, name: &str) -> Option<bool> {
        (node.class == "arkKisculeNodeGate" && name == "m_isOpen")
            .then(|| self.gate_open.get(&node.id).copied()).flatten()
    }
    fn graph_nodes(&self) -> &[Node] {
        &self.graph.nodes
    }
    fn graph_owner(&self) -> &str {
        &self.owner
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
        if let Some(kind)=super::story_flags::kind(&node.class) {
            if point.name=="m_value" {
                //9241f0/9043e0 read the actual bound current value; their
                //missing-variable branch returns the executable's zero slot.
                return match self.story.as_ref().and_then(|state|state.bindings.get(&node.id)) {
                    Some(binding)=>binding.get().map(|value|Some(value.unwrap_or(kind))),
                    None=>Ok(Some(kind)),
                };
            }
        }
        if super::story_flags::named_kind(&node.class).is_some()
            && atom(&node.params,"m_variableName").is_some_and(|name|name.contains('/')) {
            //923f00 ->6cb0d0 resolves entity/variable into that entity's
            //existing current variable; it is not a key in this caller's map.
            return match self.story.as_ref().and_then(|state|state.bindings.get(&node.id)) {
                Some(binding)=>binding.get(),
                None=>Err(format!("qualified variable {} awaits its actual live actor binding",
                    atom(&node.params,"m_variableName").unwrap_or(""))),
            };
        }
        if matches!(node.class.as_str(),"arkKisculeNodePickupNotification"|"arkKisculeNodeUsableNotification"|"arkKisculeNodeBreakableNotification"|"arkKisculeNodeGameflowMissionNotification"|"arkKisculeNodeGameflowObjectiveNotification"|"arkKisculeNodeGameflowTaskNotification") && point.name == "m_instigator" {
            return Ok(Some(GraphValue::Entity(self.notifications.as_ref()
                .and_then(|states| states.get(&node.id)).and_then(|state| state.instigator.clone()))));
        }
        if node.class == "arkKisculeNodeEntitySpawnerControl" && point.name == "m_createdEntity" {
            return Ok(Some(GraphValue::Entity(self.spawned.get(&node.id).cloned().flatten())));
        }
        if node.class == "arkKisculeNodeDelay" && point.name == "m_currentDelay" {
            let remaining = self.delays.as_ref().and_then(|delays| delays.get(&node.id)).map_or(0, |delay| delay.remaining_ms);
            return Ok(Some(GraphValue::Time(remaining)));
        }
        if node.class == "arkKisculeNodeUsableNotification" && point.name == "m_stageIndex" {
            let current = self.notifications.as_ref().and_then(|states| states.get(&node.id)).and_then(|state| state.usable_stage);
            return Ok(Some(GraphValue::Int(current.unwrap_or(-1))));
        }
        if node.class == "arkKisculeNodeBreakableNotification" {
            let state = self.notifications.as_ref().and_then(|states| states.get(&node.id));
            match point.name.as_str() {
                "m_stepIndex" => return Ok(Some(GraphValue::Int(state.map_or(0, |state| state.step)))),
                "m_location" => return Ok(Some(GraphValue::Vector(state.map_or([0.; 3], |state| state.location)))),
                _ => {}
            }
        }
        if node.class == "arkKisculeNodeGetGameflowTaskState" {
            let name=self.string_id_input(node,"m_taskName")?;
            let status=self.gameflow.as_ref().and_then(|mailbox|mailbox.status(crate::gameplay::gameflow::program::Domain::Task,name)).unwrap_or(6);
            return Ok(Some(GraphValue::Int(status)));
        }
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
    pub fn new(graph: impl Into<Arc<FxGraph>>, random: HashMap<u32, f32>) -> Self {
        let graph = graph.into();
        let relevant = Self::relevant_nodes(&graph, graph.effects.keys().copied().collect());
        let variables = Arc::new(RwLock::new(GraphVariables::new(&graph.variables)));
        let story=graph.nodes.iter().any(|node|super::story_flags::kind(&node.class).is_some()
            || super::story_flags::named_kind(&node.class).is_some())
            .then(||Box::new(super::story_flags::State::default()));
        let gameflow_subscriptions=graph.nodes.iter().enumerate().filter(|(_,node)|matches!(node.class.as_str(),"arkKisculeNodeGameflowMissionNotification"|"arkKisculeNodeGameflowObjectiveNotification"|"arkKisculeNodeGameflowTaskNotification")).map(|(index,_)|index).collect();
        Self {
            owner: graph.owner.clone(),
            graph,
            relevant,
            random,
            random_owner: None,
            variables,
            story,
            children: HashMap::new(),
            pending: Vec::new(),
            delays: None,
            gates: HashMap::new(),
            gate_open: HashMap::new(),
            effects: HashMap::new(),
            entity_effects: HashMap::new(),
            notifications: None,
            gameflow_subscriptions,
            gameflow: None,
            spawner: None,
            spawned: HashMap::new(),
            instance_id:None,spawn_attempt:0,pending_spawns:HashMap::new(),
        }
    }
    pub(super) fn set_spawner(&mut self, spawner: Option<Arc<super::spawner::EntitySpawner>>) { self.spawner = spawner; }
    /// The host's construction result completes the node's continuation:
    /// Succeeded with m_createdEntity, or Failed.
    pub(super) fn set_instance_id(&mut self,id:Option<u64>){self.instance_id=id;}
    pub(super) fn spawn_result(&mut self, receipt:super::spawner::Receipt, result: Result<crate::gameplay::entity_lifetime::EntityId, String>) -> Result<(), String> {
        let node=receipt.node;
        if self.instance_id!=Some(receipt.instance)||self.pending_spawns.get(&node)!=Some(&receipt.attempt){return Err("entity spawner reply has a stale graph/node attempt".into());}
        self.pending_spawns.remove(&node);
        match result {
            Ok(entity) => {
                self.spawned.insert(node, Some(EntityReference::Managed(entity)));
                self.emit(node, "Succeeded")
            }
            Err(error) => {
                self.spawned.remove(&node); //79dc40 sets its once flag only on success.
                self.emit(node, "Failed")?;
                Err(format!("entity spawner node {node}: {error}"))
            }
        }
    }
    pub(super) fn set_gameflow(&mut self, mailbox: Option<Arc<crate::gameplay::gameflow::program::Mailbox>>) { self.gameflow = mailbox; }
    pub(super) fn set_owner(&mut self, owner: String) { self.owner = owner; }
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
    pub(super) fn shared_definition(&self) -> Arc<FxGraph> { self.graph.clone() }
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
        self.emit_queued(id, name, false).map(|_| ())
    }
    fn emit_queued(&mut self, id: u32, name: &str, count_pruned: bool) -> Result<bool, String> {
        let definition = self.shared_definition();
        let node = definition.node(id)?;
        let links = node
            .ports
            .iter()
            .find(|p| p.name == name && p.kind.starts_with("Output"))
            .map(|p| p.links.as_slice())
            .unwrap_or_default();
        let mut connected = false;
        for &(target, port) in links {
            let relevant = self.relevant.contains(&target);
            if !count_pruned && !relevant { continue; }
            if self.node(target).ok().is_some_and(|n| {
                n.ports
                    .iter()
                    .any(|p| p.id == port && p.kind.starts_with("Input"))
            }) {
                connected = true;
                if relevant { self.pending.push((target, port)); }
            }
        }
        Ok(connected)
    }
    /// 1407ac360 receives the pickedUp event on the subscribed entity. The
    /// local player is its notification instigator; a shadow-grapple pickup
    /// is ignored only when the connected/authored flag requests that filter.
    pub fn picked_up(&mut self, id: u32, successful: bool, from_grapple: bool) -> Result<(), String> {
        let definition = self.shared_definition();
        let node = definition.node(id)?;
        if node.class != "arkKisculeNodePickupNotification" { return Err("node is not a pickup notification".into()); }
        let ignore_grapple = self.boolean_input(node, "m_ignoreShadowGrapple", false)?;
        if from_grapple && ignore_grapple { return Ok(()); }
        let maximum = self.integer_input(node, "m_maxNotificationCount", -1)?;
        self.notifications.get_or_insert_with(Default::default).entry(id).or_default().instigator = Some(EntityReference::Player);
        self.notification_outputs(id, maximum, if successful { "Picked Up" } else { "Picked Up Failed" })
    }
    /// 7986b0: the broken entity, rather than its damage attacker, owns the notification.
    pub fn broken(&mut self, id: u32, entity: EntityReference, step: i32, removed: bool, location: [f32; 3]) -> Result<(), String> {
        let definition = self.shared_definition();
        let node = definition.node(id)?;
        if node.class != "arkKisculeNodeBreakableNotification" { return Err("node is not a breakable notification".into()); }
        let maximum = self.integer_input(node, "m_maxNotificationCount", -1)?;
        let state = self.notifications.get_or_insert_with(Default::default).entry(id).or_default();
        state.instigator = Some(entity); state.step = step; state.location = location;
        for output in [Some("Step changed"), removed.then_some("Removed"), Some("Out")].into_iter().flatten() {
            let count = self.notifications.as_ref().unwrap()[&id].count;
            if (maximum < 0 || count < maximum) && self.emit_queued(id, output, true)? {
                let state = self.notifications.as_mut().unwrap().get_mut(&id).unwrap();
                state.count = state.count.wrapping_add(1);
            }
        }
        Ok(())
    }
    /// 7af990 uses the common notification count, instigator, Used and Out ports.
    pub(super) fn used(&mut self, node: u32, instigator: Option<EntityReference>) -> Result<(), String> {
        let definition = self.shared_definition();
        let source = definition.node(node)?;
        let maximum = self.integer_input(source, "m_maxNotificationCount", -1)?;
        self.notifications.get_or_insert_with(Default::default).entry(node).or_default().instigator = instigator;
        self.notification_outputs(node, maximum, "Used")
    }

    /// 7af810/7af8d0 write the actual stage before firing the phase-specific output.
    pub(super) fn usable_stage(&mut self, id: u32, stage: i32, started: bool, instigator: Option<EntityReference>) -> Result<(), String> {
        let definition = self.shared_definition();
        let maximum = self.integer_input(definition.node(id)?, "m_maxNotificationCount", -1)?;
        let state = self.notifications.get_or_insert_with(Default::default).entry(id).or_default();
        state.usable_stage = Some(stage);
        state.instigator = instigator;
        self.notification_outputs(id, maximum, if started { "Stage changed (Start)" } else { "Stage changed (Finished)" })
    }

    pub fn breakable_subscriptions(&self) -> (Vec<(u32, EntityReference)>, Vec<String>) {
        self.notification_subscriptions("arkKisculeNodeBreakableNotification")
    }
    pub fn breakable_subscription(&self, id: u32) -> Result<Option<EntityReference>, String> {
        self.notification_subscription(id, "arkKisculeNodeBreakableNotification")
    }
    fn notification_outputs(&mut self, id: u32, maximum: i32, first: &str) -> Result<(), String> {
        for output in [first, "Out"] {
            let count = self.notifications.as_ref().unwrap().get(&id).unwrap().count;
            if (maximum < 0 || count < maximum) && self.emit_queued(id, output, true)? {
                let state = self.notifications.as_mut().unwrap().get_mut(&id).unwrap();
                state.count = state.count.wrapping_add(1);
            }
        }
        Ok(())
    }
    /// Reset clears the notification count; the subscription owner handles
    /// its own unsubscribe/re-subscribe and preserves the last instigator.
    /// 63db90/63ddb0/63e000: native Gameflow status notifications use
    /// the same count/instigator/output owner as pickup notifications.
    pub fn gameflow_notification(&mut self,update:crate::gameplay::gameflow::program::Update)->Result<(),String>{
        use crate::gameplay::gameflow::program::Domain;
        if self.gameflow_subscriptions.is_empty(){return Ok(());}
        let definition=self.shared_definition();
        for at in 0..self.gameflow_subscriptions.len() {
            let node=&definition.nodes[self.gameflow_subscriptions[at]];
            let (domain,field,prefix)=match node.class.as_str(){
                "arkKisculeNodeGameflowMissionNotification"=>(Domain::Mission,"m_missionName","Mission"),
                "arkKisculeNodeGameflowObjectiveNotification"=>(Domain::Objective,"m_objectiveName","Objective"),
                "arkKisculeNodeGameflowTaskNotification"=>(Domain::Task,"m_taskName","Task"),
                _=>continue,
            };
            if domain!=update.domain || !self.relevant.contains(&node.id)||self.string_id_input(node,field)?!=update.name||!self.boolean_input(node,"m_enabled",true)?{continue;}
            let listened=self.entity_input(node,"m_listenedEntity",None)?;
            if listened.as_ref().is_some_and(|entity|*entity!=EntityReference::Player){return Err("gameflow callback requires its resolved player entity binding".into());}
            let filter=self.entity_input(node,"m_instigator",None)?;
            if filter.as_ref().is_some_and(|entity|*entity!=EntityReference::Player){continue;}
            let status=match (update.domain,update.status){(_,0)=>"Pending",(_,2)=>"Active",(_,3)=>"Completed",(crate::gameplay::gameflow::program::Domain::Mission,_)=>"Failed",(_,1)=>"Inactive",(_,4)=>"Failed",(_,5)=>"Canceled",_=>continue};
            let maximum=self.integer_input(node,"m_maxNotificationCount",-1)?;
            self.notifications.get_or_insert_with(Default::default).entry(node.id).or_default().instigator=Some(EntityReference::Player);
            self.notification_outputs(node.id,maximum,&format!("{prefix} {status}"))?;
        }
        Ok(())
    }
    pub fn reset_notification_count(&mut self, id: u32) {
        if let Some(state) = self.notifications.as_mut().and_then(|states| states.get_mut(&id)) {
            state.count = 0;
        }
    }
    pub fn pickup_subscriptions(&self) -> (Vec<(u32, EntityReference)>, Vec<String>) {
        self.notification_subscriptions("arkKisculeNodePickupNotification")
    }
    pub fn pickup_subscription(&self, id: u32) -> Result<Option<EntityReference>, String> {
        self.notification_subscription(id, "arkKisculeNodePickupNotification")
    }
    pub(crate) fn notification_subscription(&self, id: u32, class: &str) -> Result<Option<EntityReference>, String> {
        let node = self.node(id)?;
        if node.class != class { return Err(format!("node is not a {class}")); }
        if !self.boolean_input(node, "m_enabled", true)? { return Ok(None); }
        let default = atom(&node.params, "m_listenedEntity")
            .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case("NULL"))
            .map(|name| EntityReference::Named(name.to_owned()));
        Ok(self.entity_input(node, "m_listenedEntity", default)?.filter(|target| *target != EntityReference::Invalid))
    }
    pub(crate) fn notification_subscriptions(&self, class: &str) -> (Vec<(u32, EntityReference)>, Vec<String>) {
        let mut subscriptions = Vec::new();
        let mut errors = Vec::new();
        for node in self.graph.nodes.iter().filter(|node| node.class == class && self.relevant.contains(&node.id)) {
            match self.notification_subscription(node.id, class) {
                Ok(Some(target)) => subscriptions.push((node.id, target)),
                Ok(None) => {},
                Err(error) => errors.push(format!("node {}: {error}", node.id)),
            }
        }
        (subscriptions, errors)
    }
    pub fn loading_end(&mut self) -> Result<(), String> {
        let nodes = self
            .graph
            .nodes
            .iter()
            .filter(|n| n.class == "arkKisculeNodeLoadingEnd")
            .map(|n| n.id)
            .collect::<Vec<_>>();
        for id in nodes { self.loading_end_node(id)?; }
        Ok(())
    }
    pub fn loading_end_node(&mut self, node: u32) -> Result<(), String> {
        if self.node(node)?.class != "arkKisculeNodeLoadingEnd" { return Err("LoadingEnd node changed".into()); }
        self.emit(node, "LevelLoadedFirstTime")
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
        // 1407c0e30 uses the existing definition, with separate execution state.
        let definition = self.shared_definition();
        let node = definition.node(id)?;
        let port = node
            .ports
            .iter()
            .find(|p| p.id == port)
            .ok_or("missing event port")?;
        let mut schedule = None;
        let paired_outputs = match node.class.as_str() {
            "arkKisculeNodeActionGameflowMarkerControl"=>{
                let target=|field:&str|atom(&node.params,field).filter(|name|!name.is_empty()&&*name!="NULL").map(|name|EntityReference::Named(name.to_owned()));
                let entity=self.entity_input(&node,"m_target",target("m_target"))?;
                let replacement=self.entity_input(&node,"m_newTarget",target("m_newTarget"))?;
                let accepted=if let Some(entity)=entity {
                    if let Some(id)=world.managed_entity(&entity)? {
                        let replacement=if let Some(replacement)=replacement{world.managed_entity(&replacement)?}else{None};
                        if port.name=="Replace Target"&&replacement.is_none(){false}else{
                            let target=crate::gameplay::gameflow::MarkerTarget{name:self.string_id_input(&node,"m_targetName")?,entity:EntityReference::Managed(id)};
                            let replacement=crate::gameplay::gameflow::MarkerTarget{name:self.string_id_input(&node,"m_newTargetName")?,entity:replacement.map_or(EntityReference::Invalid,EntityReference::Managed)};
                            self.gameflow.as_ref().map_or(Ok(false),|owner|owner.control_marker(self.string_id_input(&node,"m_taskId")?,&port.name,target,replacement,|reference|world.managed_entity(reference)))?
                        }
                    }else{false}
                }else{false};
                self.emit(id,if accepted{"Update succeeded"}else{"Update failed"})?;
                true
            },
            "arkKisculeNodeActionGameflowMission" | "arkKisculeNodeActionGameflowObjective" | "arkKisculeNodeActionGameflowTask" => {
                use crate::gameplay::gameflow::program::{Domain,Update};
                let (domain,field) = if node.class.ends_with("Mission") {(Domain::Mission,"m_missionName")}
                    else if node.class.ends_with("Objective") {(Domain::Objective,"m_objectiveName")}
                    else {(Domain::Task,"m_taskName")};
                let accepted = if port.name=="Set Portrait Parameters" {
                    let image=self.declaration_input(node,"m_portraitImage","")?;
                    let portrait=crate::gameplay::gameflow::Portrait {
                        show:self.boolean_input(node,"m_showPortrait",true)?,
                        state:self.declaration_input(node,"m_portraitStatus","GAMEFLOW_UI_PORTRAIT_STATE_ASSASSINATED")?,
                        kind:self.declaration_input(node,"m_portraitType","GAMEFLOW_UI_PORTRAIT_TYPE_TARGET")?,
                        female:self.boolean_input(node,"m_portraitFemale",false)?,
                        image:(!image.is_empty()&&image!="NULL").then_some(image),
                        name:self.string_id_input(node,"m_portraitName")?,
                    };
                    self.gameflow.as_ref().map_or(Ok(false),|owner|owner.submit_portrait(self.string_id_input(node,field)?,portrait))?
                } else {
                let status = match port.name.as_str() {
                    "Activate" | "Reactivate" | "Uncomplete" => 2, "Complete" => 3, "Fail" => 4, "Cancel" => 5, "Reset" | "Unactivate" => 0,
                    other => return Err(format!("unsupported gameflow action input {other}")),
                };
                // 530280/530550/530bc0 validate the declared native node class,
                // enqueue its request, and notify acceptance before forwarding.
                let update=Update{domain,name:self.string_id_input(&node,field)?,status};
                self.gameflow.as_ref().map_or(Ok(false),|mailbox|mailbox.submit(update))?
                };
                self.emit(id,if accepted {"Update succeeded"} else {"Update failed"})?;
                true
            }
            "arkKisculeNodeGameflowMissionNotification" | "arkKisculeNodeGameflowObjectiveNotification" | "arkKisculeNodeGameflowTaskNotification" if port.name=="Reset"=>{
                self.reset_notification_count(id);
                self.emit(id,"Reset")?;
                false
            }
            "arkKisculeNodeUsableNotification" if port.name == "Reset" => {
                return Ok(GraphStep::UsableReset { node: id });
            }
            // 792f40 emits 1cf and invokes remove/removeEntityAndAttachedObjects.
            "arkKisculeNodeActionDestroy" if port.name == "In" => {
                if self.declaration_input(node, "m_spawnTag", "SPAWN_TAG_NONE")? != "SPAWN_TAG_NONE" {
                    return Err("tag-wide graph destruction requires its spawn-tag owner".into());
                }
                let default = atom(&node.params, "m_entity").filter(|name| !name.is_empty() && *name != "NULL")
                    .map(|name| EntityReference::Named(name.to_owned()));
                if let Some(reference) = self.entity_input(node, "m_entity", default)? {
                    if let Some(entity) = world.managed_entity(&reference)? {
                        world.request_usable_action(entity, UsableAction::Remove { attached: self.boolean_input(node, "m_attachedObjects", true)? })?;
                    }
                }
                true
            }
            "arkKisculeNodeActionInteractable" if port.name == "Set" => {
                // 52dc30 calls the resolved Interactable's virtual availability setter.
                self.queue_availability(world, node, "m_entity", "m_usable", false)?;
                true
            }
            "arkKisculeNodeActionUsable" if port.name == "Set Usability" => {
                // 797d30 checks UsableWithStateList, then UsableStatic, and calls the same setter.
                self.queue_availability(world, node, "m_usable", "m_usableByPlayer", true)?;
                true
            }
            "arkKisculeNodeActionUsable" if matches!(port.name.as_str(), "Next Stage" | "Set Stage") => {
                let default = atom(&node.params, "m_usable").filter(|name| !name.is_empty() && *name != "NULL")
                    .map(|name| EntityReference::Named(name.to_owned()));
                if let Some(reference) = self.entity_input(node, "m_usable", default)? {
                    if let Some(entity) = world.managed_entity(&reference)? {
                        // 797750 advances with normal timing. 797c40 alone reads instantChangeState.
                        if port.name == "Next Stage" {
                            world.request_usable_action(entity, UsableAction::Stage { stage: None, instant: false })?;
                        } else if let Ok(stage) = usize::try_from(self.integer_input(node, "m_wantedStageIndex", -1)?) {
                            world.request_usable_action(entity, UsableAction::Stage { stage: Some(stage), instant: self.boolean_input(node, "m_instantChangeState", false)? })?;
                        }
                    }
                }
                true
            }
            "arkKisculeNodeBreakableNotification" if port.name == "Reset" => {
                return Ok(GraphStep::BreakableReset { node: id });
            }
            "arkKisculeNodePickupNotification" if port.name == "Reset" => {
                return Ok(GraphStep::PickupReset { node: id });
            }
            "arkKisculeNodeEntitySpawnerControl" if port.name == "Spawn" => {
                // 14079dc40 spawns once per node; later Spawn inputs also
                // repeat Succeeded without constructing another entity.
                if !self.spawned.contains_key(&id) {
                    let spawner = self.spawner.clone().ok_or("entity spawner host is not connected")?;
                    if self.input(&node, "m_entityDef").is_some() || self.input(&node, "m_spawnTag").is_some()
                        || self.input(&node, "m_location").is_some() || self.input(&node, "m_rotation").is_some() {
                        return Err("connected entity spawner inputs are not prepared".into());
                    }
                    let target_default = atom(&node.params, "m_target")
                        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("NULL"))
                        .map(|name| EntityReference::Named(name.to_owned()));
                    let target = self.entity_input(&node, "m_target", target_default)?;
                    let target=target.as_ref().map(|reference|world.managed_entity(reference)).transpose()?.flatten();
                    let target_frame=target.map(|id|world.managed_frame(id)).transpose()?.flatten();
                    let target_spawn_tag=target.map(|id|world.managed_spawn_tag(id)).transpose()?;
                    self.spawn_attempt=self.spawn_attempt.checked_add(1).ok_or("entity spawner attempt identity exhausted")?;
                    let receipt=super::spawner::Receipt{instance:self.instance_id.ok_or("entity spawner graph generation absent")?,node:id,attempt:self.spawn_attempt};
                    let fields = node.params.as_slice();
                    spawner.submit(super::spawner::EntitySpawnRequest {
                        receipt,
                        owner: self.owner.clone(),
                        graph_entity:world.managed_entity(&EntityReference::Named(self.owner.clone()))?,
                        declaration: atom(&node.params, "m_entityDef").unwrap_or("").to_owned(),
                        target:target.map(EntityReference::Managed),target_frame,target_spawn_tag,
                        location: super::variables::read_vector(fields, "m_location")?,
                        rotation: super::variables::read_vector(fields, "m_rotation")?,
                        spawn_tag: atom(&node.params, "m_spawnTag").unwrap_or("").to_owned(),
                    })?;
                    self.spawned.insert(id, None);
                    self.pending_spawns.insert(id,receipt.attempt);
                } else if self.spawned.get(&id).is_some_and(Option::is_some) {
                    self.emit(id, "Succeeded")?;
                }
                true
            }
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
            // The shipping Log handler 7a1750 binds 399620 (return 0).
            // Its regular Log output still forwards through 7c0e30.
            "arkKisculeNodeLog" if port.name == "Log" => true,
            "arkKisculeNodeDelay" => {
                let duration = self.integer_input(node, "m_delay", 1000)?;
                let reset_on_start = self.boolean_input(node, "m_resetOnStart", false)?;
                let delay = self.delays.get_or_insert_with(Default::default).entry(id).or_default();
                let was_running = delay.running;
                match port.name.as_str() {
                    "Start" => {
                        if delay.remaining_ms == 0 || reset_on_start {
                            delay.remaining_ms = duration;
                            delay.last_time = None;
                        }
                        delay.running = true;
                    }
                    "Pause" => delay.running = false,
                    "Resume" if !was_running => { delay.last_time = None; delay.running = true; }
                    "Resume" => {}
                    "Reset" | "Stop" => {
                        delay.remaining_ms = duration;
                        delay.last_time = None;
                        if port.name == "Stop" { delay.running = false; }
                    }
                    input => return Err(format!("unsupported Delay input {input}")),
                }
                if delay.running && !was_running { schedule = Some(id); }
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
            "arkKisculeNodeStoryFlagBool" | "arkKisculeNodeStoryFlagInt" if port.name=="Set" => {
                let name=atom(&node.params,"m_variableName").unwrap_or("");
                let kind=super::story_flags::kind(&node.class).ok_or("story variable type absent")?;
                //926e60/907270 use an explicit Value link first, then the
                //actual parent declaration input; no link preserves current.
                let value=if let Some((source,point))=self.input(&node,"m_value") {
                    Some(kind.read(self,source,point)?)
                } else {world.parent_value(name,&kind)?};
                let accepted=if let Some(value)=value {
                    let binding=self.story.as_ref().and_then(|state|state.bindings.get(&id)).cloned();
                    if let Some(binding)=binding {
                        if binding.set(value)? {
                            self.story.as_mut().ok_or("story state absent")?.updates.push(binding);
                            true
                        } else {false}
                    } else {false}
                } else {false};
                if !accepted {
                    self.emit(id,"Set")?;
                    self.emit(id,"Out")?;
                    return Err(format!("story variable {name}: Set lacks a live binding or Value link"));
                }
                true
            }
            "arkKisculeNodeNamedVariableFloat"
            | "arkKisculeNodeNamedVariableBool"
            | "arkKisculeNodeNamedVariableInt"
            | "arkKisculeNodeNamedVariableVec3"
            | "arkKisculeNodeNamedVariableEntityReference"
                if port.name == "Set" =>
            {
                let name = atom(&node.params, "m_variableName")
                    .unwrap_or("")
                    .to_owned();
                let kind = match node.class.as_str() {
                    "arkKisculeNodeNamedVariableBool" => GraphValue::Bool(false),
                    "arkKisculeNodeNamedVariableInt" => GraphValue::Int(0),
                    "arkKisculeNodeNamedVariableVec3" => GraphValue::Vector([0.; 3]),
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
                    if let Some(binding)=self.story.as_ref().and_then(|state|state.bindings.get(&id)).cloned() {
                        if binding.set(value)? {
                            self.story.as_mut().ok_or("named variable binding state absent")?.updates.push(binding);
                            self.emit(id,"Set")?;
                            self.emit(id,"Out")?;
                            return Ok(GraphStep::Action{schedule});
                        }
                        self.emit(id,"Set")?;
                        self.emit(id,"Out")?;
                        return Err(format!("named variable {name} target has retired"));
                    }
                    if name.contains('/') {
                        self.emit(id,"Set")?;
                        self.emit(id,"Out")?;
                        return Err(format!("qualified variable {name} has no live target"));
                    }
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
    fn queue_availability(&self, world: &mut impl FxGraphWorld, node: &Node, target_field: &str, flag_field: &str, initial: bool) -> Result<(), String> {
        let default = atom(&node.params, target_field).filter(|name| !name.is_empty() && *name != "NULL")
            .map(|name| EntityReference::Named(name.to_owned()));
        if let Some(reference) = self.entity_input(node, target_field, default)? {
            if let Some(entity) = world.managed_entity(&reference)? {
                world.request_usable_action(entity, UsableAction::SetAvailability(self.boolean_input(node, flag_field, initial)?))?;
            }
        }
        Ok(())
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
        if let Some(delay) = self.delays.as_mut().and_then(|delays| delays.get_mut(&node)) {
            if !delay.running { return Ok(false); }
            // 7c42c0 samples the owning entity clock and truncates the remaining
            // signed milliseconds on every tick, including its first baseline.
            let now = world.graph_clock(&self.owner)?;
            if let Some(previous) = delay.last_time {
                delay.remaining_ms = ((delay.remaining_ms as f32 / 1000. - (now - previous) as f32) * 1000.) as i32;
            }
            if delay.remaining_ms < 1 {
                delay.remaining_ms = 0;
                delay.running = false;
                delay.last_time = None;
                self.emit(node, "Finished")?;
                return Ok(false);
            }
            delay.last_time = Some(now);
            return Ok(true);
        }
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
        self.graph.node(self.id)?;
        self.graph.gate_open.insert(self.id, value);
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
            &self.graph.owner,
            self.id,
            &parameters,
            state,
            duration,
        )
    }
    fn stop_effect(&mut self, state: &mut FxNode, handle: u32) -> Result<(), String> {
        if self
            .world
            .stop_effect(&self.graph.owner, self.id, handle)?
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
