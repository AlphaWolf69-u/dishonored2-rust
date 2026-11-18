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
    sync::{Arc, Mutex, atomic::{AtomicU64, Ordering}},
};

// Subscription callbacks must never alias a graph from an earlier level.
static NEXT_GRAPH_GENERATION: AtomicU64 = AtomicU64::new(1);

/// One883d10 receiver snapshot, owned by the actual fallible pickup action.
/// Graph generations stay in the canonical manager; no definition is copied.
#[derive(Default)]
pub(crate) struct PickupReceipt {
    pub(crate) arguments: Option<(crate::gameplay::entity_lifetime::EntityId,bool,bool)>,
    pub(crate) receivers: Vec<(u64,u32)>,
    pub(crate) next: usize,
    pub(crate) errors: Vec<String>,
    pub(crate) failure: Option<String>,
}

pub enum GraphEvent {
    Action { graph: usize, node: u32, port: u32 },
    LoadingEnd { graph: usize, node: u32 },
}
impl GraphEvent {
    pub fn graph(&self) -> usize {
        match *self { Self::Action { graph, .. } | Self::LoadingEnd { graph, .. } => graph }
    }
}

#[derive(Default)]
pub struct FxGraphInstances {
    runtimes: Vec<Option<FxGraphRuntime>>,
    parents: Vec<Option<(usize, u32)>>,
    effectful: Vec<bool>,
    children: HashMap<(usize, u32), usize>,
    failures: HashMap<(usize, u32), String>,
    pub events: crate::gameplay::event_graph::ManagerWork<GraphEvent>,
    generations: Vec<u32>,
    live: Vec<bool>,
    free: Vec<usize>,
    random: Option<Arc<Mutex<crate::random::MwcRandom>>>,
    gameflow: Option<Arc<crate::gameplay::gameflow::program::Mailbox>>,
    gameflow_notifications: Vec<crate::gameplay::gameflow::program::Update>,
    story_root: Option<u64>,
    story_listeners: HashMap<String,Vec<(u64,u32)>>,
    spawner: Option<Arc<super::spawner::EntitySpawner>>,
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
    ///5134e0 locates the current bool in a named story declaration instance;
    ///6cbf20 uses the engine's zero-initialized bool only for a null lookup.
    ///Borrow the same variable owner that receives graph writes, not a second
    ///snapshot of declaration defaults. Missing graph/variable remains None.
    fn story_index(&self, root: u64, qualified: &str) -> Result<Option<usize>, String> {
        let root = self.instance_index(root).ok_or("story worldspawn graph generation stale")?;
        let (path, _) = qualified.rsplit_once('/').ok_or("story variable lacks declaration/name")?;
        //Follow the same already-owned authored child order as7bc1d0. Do
        //not scan unrelated actors' graph instances on every rule condition.
        fn find(owner: &FxGraphInstances, index: usize, path: &str) -> Result<Option<usize>, String> {
            if !owner.live[index] { return Ok(None); }
            let runtime = owner.runtimes[index].as_ref().ok_or("story graph is executing")?;
            if runtime.definition().owner.eq_ignore_ascii_case(path) { return Ok(Some(index)); }
            for node in &runtime.definition().nodes {
                if let Some(&child) = owner.children.get(&(index,node.id)) {
                    if let Some(found) = find(owner,child,path)? { return Ok(Some(found)); }
                }
            }
            Ok(None)
        }
        find(self, root, path)
    }
    fn story_binding(&self,root:u64,qualified:&str)->Result<Option<super::story_flags::Binding>,String>{
        let Some(index)=self.story_index(root,qualified)? else{return Ok(None)};
        let (_,name)=qualified.rsplit_once('/').ok_or("story variable lacks declaration/name")?;
        self.variable_binding(index,name,qualified)
    }
    fn variable_binding(&self,index:usize,name:&str,qualified:&str)->Result<Option<super::story_flags::Binding>,String>{
        let runtime=self.runtimes[index].as_ref().ok_or("variable graph is executing")?;
        if runtime.variables.read().map_err(|_|"story variable owner poisoned")?.get(name)?.is_none() {
            return Ok(None);
        }
        let variable=runtime.definition().variables.iter().find(|variable|atom(&variable.parameters,"m_name")==Some(name))
            .ok_or("story variable declaration absent")?;
        //8f0f30 initializes GraphVariableBase+10 to zero before authored fields.
        let global=match atom(&variable.parameters,"m_global").unwrap_or("false") {
            "true"=>true,"false"=>false,_=>return Err("invalid story variable global flag".into()),
        };
        let external;
        let qualified=if global && !qualified.contains('/') {
            external=format!("{}/{name}",runtime.graph_owner());external.as_str()
        } else {qualified};
        Ok(Some(super::story_flags::Binding::new(&runtime.variables,name,qualified,
            self.instance_id(index).ok_or("story target generation absent")?,global)))
    }
    pub(crate) fn story_boolean(&self,root:u64,qualified:&str)->Result<Option<bool>,String>{
        let Some(index)=self.story_index(root,qualified)? else{return Ok(None)};
        let (_,name)=qualified.rsplit_once('/').ok_or("story variable lacks declaration/name")?;
        let runtime=self.runtimes[index].as_ref().ok_or("story graph is executing")?;
        let value=runtime.variables.read().map_err(|_|"story variable owner poisoned")?.get(name)?;
        match value {
            None=>Ok(None),Some(super::variables::GraphValue::Bool(value))=>Ok(Some(value)),
            Some(_)=>Err(format!("story variable {qualified} is not bool")),
        }
    }
    ///7bf3e0 selects the actual worldspawn program owner; declaration lookup
    ///5134e0 and current values remain in that same graph manager.
    pub(crate) fn bind_story_root(&mut self,root:u64)->Result<(),String>{
        self.instance_index(root).ok_or("story worldspawn graph generation stale")?;
        if self.story_root.is_some_and(|previous|previous!=root){return Err("story worldspawn already bound".into());}
        self.story_root=Some(root);
        Ok(())
    }
    pub(crate) fn prepare_story_bindings(&mut self,index:usize)->Result<(),String>{
        let runtime=self.runtimes.get(index).and_then(Option::as_ref).ok_or("story caller graph busy")?;
        let Some(state)=runtime.story.as_ref() else{return Ok(())};
        if state.ready{return Ok(())}
        //924050/9042e0 diagnose a null LocateVariable result at the actual
        //getter/Set call. They do not make an otherwise empty world fail load.
        let Some(root)=self.story_root else{return Ok(())};
        let definition=runtime.shared_definition();
        let mut ready=true;
        for node in &definition.nodes {
            let Some(kind)=super::story_flags::kind(&node.class) else{continue};
            if self.runtimes[index].as_ref().and_then(|runtime|runtime.story.as_ref())
                .is_some_and(|state|state.bindings.get(&node.id).is_some_and(|binding|binding.variables.strong_count()!=0)){continue;}
            let name=atom(&node.params,"m_variableName").ok_or("StoryFlag variable name absent")?;
            if let Some(binding)=self.story_binding(root,name)? {
                if let Some(value)=binding.get()? {
                    if std::mem::discriminant(&value)!=std::mem::discriminant(&kind) {
                        return Err(format!("StoryFlag {name} type differs from {}",node.class));
                    }
                }
                self.runtimes[index].as_mut().and_then(|runtime|runtime.story.as_mut())
                    .ok_or("story caller state absent")?.bindings.insert(node.id,binding);
            } else {ready=false;}
        }
        self.runtimes[index].as_mut().and_then(|runtime|runtime.story.as_mut())
            .ok_or("story caller state absent")?.ready=ready;
        Ok(())
    }
    ///923f00 ->6cb0d0: qualified NamedVariable references select a real
    ///entity's current root-program variables through the existing actor map.
    pub(crate) fn prepare_named_bindings(&mut self,index:usize,
        resolve:&mut impl FnMut(&str)->Option<u64>)->Result<(),String>{
        let runtime=self.runtimes.get(index).and_then(Option::as_ref).ok_or("named variable caller busy")?;
        let Some(state)=runtime.story.as_ref() else{return Ok(())};
        if state.named_ready{return Ok(())}
        let definition=runtime.shared_definition();
        let mut ready=true;
        for node in &definition.nodes {
            let Some(kind)=super::story_flags::named_kind(&node.class) else{continue};
            let Some(qualified)=atom(&node.params,"m_variableName") else{continue};
            if self.runtimes[index].as_ref().and_then(|runtime|runtime.story.as_ref())
                .is_some_and(|state|state.bindings.get(&node.id).is_some_and(|binding|binding.variables.strong_count()!=0)){continue;}
            let (target,name)=if let Some((entity,name))=qualified.split_once('/') {
                (resolve(entity).and_then(|root|self.instance_index(root)),name)
            } else {(Some(index),qualified)};
            let binding=target.map(|target|self.variable_binding(target,name,qualified)).transpose()?.flatten();
            if let Some(binding)=binding {
                if let Some(value)=binding.get()? {
                    if std::mem::discriminant(&value)!=std::mem::discriminant(&kind) {
                        return Err(format!("qualified variable {qualified} type differs from {}",node.class));
                    }
                }
                self.runtimes[index].as_mut().and_then(|runtime|runtime.story.as_mut())
                    .ok_or("named variable caller state absent")?.bindings.insert(node.id,binding);
            } else {ready=false;}
        }
        self.runtimes[index].as_mut().and_then(|runtime|runtime.story.as_mut())
            .ok_or("named variable caller state absent")?.named_ready=ready;
        Ok(())
    }
    pub(crate) fn prepare_named_all(&mut self,mut resolve:impl FnMut(&str)->Option<u64>)->Result<(),String>{
        for index in 0..self.runtimes.len(){
            if self.live[index]{self.prepare_named_bindings(index,&mut resolve)?;}
        }
        Ok(())
    }
    ///926e60/907270 publish7c07b0 before regular Set/Out forwarding. These
    ///are references to canonical variables, never copied declaration values.
    fn story_updated(&mut self,source:usize,update:&super::story_flags::Binding)->Result<(),String>{
        let target=if update.global {self.instance_index(update.target)} else {Some(source)};
        if let Some(target)=target {
            self.runtimes[target].as_mut().ok_or("story notification graph busy")?.variable_updated(&update.name)?;
            self.flush(target)?;
        }
        //7c0b50 has a real name-to-node observer roster; no node-vector scan
        //or per-write allocation is needed. Generations cannot cross reuse.
        let key=if update.global {update.qualified.as_ref()} else {update.name.as_ref()};
        let count=self.story_listeners.get(key).map_or(0,Vec::len);
        for at in 0..count {
            let (graph,node)=self.story_listeners[key][at];
            let Some(index)=self.instance_index(graph) else{continue};
            if !update.global && index!=source {continue;}
            self.runtimes[index].as_mut().ok_or("story notification graph busy")?.emit(node,"Updated")?;
            self.flush(index)?;
        }
        Ok(())
    }
    pub fn poll_gameflow(&mut self)->Vec<String>{
        let mut errors=Vec::new();
        let Some(mailbox)=self.gameflow.as_ref() else{return errors};
        if let Err(error)=mailbox.take_notifications(&mut self.gameflow_notifications){errors.push(error);return errors;}
        for at in 0..self.gameflow_notifications.len(){
            let update=self.gameflow_notifications[at];
            for index in 0..self.runtimes.len(){
                if !self.live[index]{continue;}
                if let Some(runtime)=self.runtimes[index].as_mut(){if let Err(error)=runtime.gameflow_notification(update){errors.push(error);}}
                errors.extend(self.collect(index));
            }
        }
        errors
    }
    pub fn set_spawner(&mut self, spawner: Arc<super::spawner::EntitySpawner>) {
        for runtime in self.runtimes.iter_mut().flatten() {runtime.set_spawner(Some(spawner.clone()));}
        self.spawner=Some(spawner);
    }
    /// Delivers the host's spawn results to the requesting graphs' nodes.
    pub fn poll_spawns(&mut self)->Vec<String>{
        let mut errors=Vec::new();
        let Some(spawner)=self.spawner.clone() else{return errors};
        let results=match spawner.take_results(){Ok(results)=>results,Err(error)=>{errors.push(error);return errors;}};
        for reply in results{
            let Some(index)=self.instance_index(reply.receipt.instance)else{continue;};
            if let Err(error)=self.runtimes[index].as_mut().unwrap().spawn_result(reply.receipt,reply.result){errors.push(error);}
            errors.extend(self.collect(index));
        }
        errors
    }
    pub fn set_gameflow(&mut self, mailbox: Arc<crate::gameplay::gameflow::program::Mailbox>) {
        for runtime in self.runtimes.iter_mut().flatten() {runtime.set_gameflow(Some(mailbox.clone()));}
        self.gameflow=Some(mailbox);
    }

    /// Runtime references versus distinct immutable definitions (diagnostics).
    pub fn definition_counts(&self) -> (usize, usize) {
        let pointers: std::collections::HashSet<_> = self.runtimes.iter().flatten()
            .map(|runtime| runtime.definition() as *const FxGraph).collect();
        (self.runtimes.iter().flatten().count(), pointers.len())
    }
    pub fn new(roots: &[Arc<FxGraph>], declarations: &FxDeclarations, seed: u64) -> Self {
        Self::with_admission(roots, declarations, seed, false)
    }
    /// A live entity graph may route to gameplay, motion and FX through the
    /// same queue; it must not inherit the render-only FX relevance filter.
    pub fn new_level(roots: &[Arc<FxGraph>], declarations: &FxDeclarations, seed: u64) -> Self {
        Self::with_admission(roots, declarations, seed, true)
    }
    fn with_admission(roots: &[Arc<FxGraph>], declarations: &FxDeclarations, seed: u64, all_nodes: bool) -> Self {
        let _tag = crate::platform::allocation_tags::tag("FX graph instances");
        let random = Arc::new(Mutex::new(crate::random::MwcRandom::event_manager(seed)));
        let mut instances = Self::default();
        instances.random = Some(random.clone());
        for root in roots {
            instances.add(root.clone(), root.owner.clone(), None, declarations, &random, &mut Vec::new(), all_nodes);
        }
        instances
    }
    fn add(
        &mut self,
        graph: Arc<FxGraph>,
        owner: String,
        parent: Option<(usize, u32)>,
        declarations: &FxDeclarations,
        random: &Arc<Mutex<crate::random::MwcRandom>>,
        path: &mut Vec<String>,
        all_nodes: bool,
    ) -> usize {
        // Diagnostic A/B retains the former per-instance deep-copy cost.
        let graph = if std::env::var("DH2_DEBUG_SHARED_FX_GRAPHS").as_deref() == Ok("0") {
            Arc::new((*graph).clone())
        } else { graph };
        let children = graph
            .nodes
            .iter()
            .filter_map(|node| instance_declaration(node).map(|name| (node.id, name.to_owned())))
            .collect::<Vec<_>>();

        let mut runtime = FxGraphRuntime::new(graph, HashMap::new());
        runtime.set_owner(owner.clone());
        runtime.set_gameflow(self.gameflow.clone());
        runtime.set_spawner(self.spawner.clone());
        runtime.random_owner = Some(Arc::clone(random));
        let effectful = runtime.definition().nodes.iter().any(|node|
            matches!(node.class.as_str(), "arkKisculeNodeFx" | "arkKisculeNodeEntityFx") || gameflow_action(&node.class)
                || super::story_flags::kind(&node.class).is_some());
        let mut targets = runtime.definition().nodes.iter().filter(|node|
            all_nodes || node.class == "arkKisculeNodeEntityFx" || gameflow_action(&node.class)
                || super::story_flags::kind(&node.class).is_some()
                || (parent.is_some() && node.class == "arkKisculeNodeDeclExit"))
            .map(|node| node.id).collect::<Vec<_>>();

        let index = if let Some(index) = self.free.pop() {
            self.runtimes[index] = Some(runtime);
            self.parents[index] = parent;
            self.effectful[index] = effectful;
            self.live[index] = true;
            index
        } else {
            let index = self.runtimes.len();
            self.runtimes.push(Some(runtime));
            self.parents.push(parent);
            self.effectful.push(effectful);
            self.generations.push(0);
            self.live.push(true);
            index
        };
        // Zero is reserved for exhaustion; never wrap and alias old work.
        self.generations[index] = u32::try_from(NEXT_GRAPH_GENERATION.fetch_add(1, Ordering::Relaxed)).unwrap_or(0);
        let generation=self.instance_id(index);
        self.runtimes[index].as_mut().unwrap().set_instance_id(generation);
        if let Some(generation)=generation {
            let definition=self[index].as_ref().unwrap().shared_definition();
            for node in &definition.nodes {
                if super::story_flags::kind(&node.class).is_some()
                    || super::story_flags::named_kind(&node.class).is_some() {
                    if let Some(name)=atom(&node.params,"m_variableName").filter(|name|name.contains('/')) {
                        self.story_listeners.entry(name.to_owned()).or_default().push((generation,node.id));
                    }
                }
            }
        }
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
                Ok(graph) => {
                    path.push(name);
                    let child = self.add(graph, owner.clone(), Some((index, node)), declarations, random, path, all_nodes);
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
        // Host roots also trace actions into the real gameflow owner. Their
        // declaration callers must stay relevant even without a particle effect.
        // Only trace actions that lead to owned effects or a declaration return.
        // Unrelated doors, AI and physics remain with their gameplay owners.
        self[index].as_mut().unwrap().include_targets(targets);
        index
    }
    pub fn has_root_owner(&self, owner: &str) -> bool {
        self.runtimes.iter().enumerate().any(|(index, runtime)|
            self.live[index] && self.parents[index].is_none()
                && runtime.as_ref().is_some_and(|runtime| runtime.graph_owner().eq_ignore_ascii_case(owner)))
    }
    pub fn root_owners(&self) -> impl Iterator<Item = &str> {
        self.runtimes.iter().enumerate().filter_map(|(index, runtime)| {
            (self.live[index] && self.parents[index].is_none())
                .then(|| runtime.as_ref().map(|runtime| runtime.graph_owner())).flatten()
        })
    }
    /// Stable subscription identity: a reused slot always receives a new
    /// generation, so delayed notifications cannot target its next owner.
    pub fn instance_id(&self, index: usize) -> Option<u64> {
        if !*self.live.get(index)? { return None; }
        let slot = u32::try_from(index).ok()?;
        let generation = *self.generations.get(index)?;
        (generation != 0).then_some((u64::from(generation) << 32) | u64::from(slot))
    }
    pub fn instance_index(&self, id: u64) -> Option<usize> {
        let index = id as u32 as usize;
        (self.instance_id(index)? == id).then_some(index)
    }
    pub fn admit_root(&mut self, graph: Arc<FxGraph>, declarations: &FxDeclarations) -> Result<u64, String> {
        let random = self.random.clone().ok_or("graph manager has not been initialized")?;
        // An explicitly admitted live entity graph is a gameplay graph even
        // when the manager also still hosts legacy FX-filtered roots.
        let index = self.add(graph.clone(), graph.owner.clone(), None, declarations, &random, &mut Vec::new(), true);
        self.instance_id(index).ok_or_else(|| "graph instance index exceeds the handle range".into())
    }
    /// Restore an authored render-only root using the original FX relevance
    /// filter. It shares this manager and its declaration/random owners, but
    /// does not assert that the corresponding gameplay entity has spawned.
    pub fn admit_render_root(&mut self, graph: Arc<FxGraph>, declarations: &FxDeclarations) -> Result<u64, String> {
        let random = self.random.clone().ok_or("graph manager has not been initialized")?;
        let index = self.add(graph.clone(), graph.owner.clone(), None, declarations, &random, &mut Vec::new(), false);
        self.instance_id(index).ok_or_else(|| "graph instance index exceeds the handle range".into())
    }
    /// Queue the admitted root's startup nodes on the shared manager stack.
    /// 14063fa90 defers them until the game-local manager readiness flag.
    pub fn queue_loading_end(&mut self, root: u64) -> Result<(), String> {
        let indices = self.retirement_subtree(root).ok_or("loading graph subtree is unavailable")?;
        for index in indices {
            let nodes = self.runtimes[index].as_ref().ok_or("loading graph is busy")?.definition().nodes.iter()
                .filter(|node| node.class == "arkKisculeNodeLoadingEnd").map(|node| node.id).collect::<Vec<_>>();
            for node in nodes { self.events.push(GraphEvent::LoadingEnd { graph: index, node }); }
        }
        Ok(())
    }
    /// Identify the complete declaration subtree before the entity owner
    /// detaches its subscriptions and FX callbacks.
    pub fn live_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.live.iter().enumerate().filter_map(|(index, &live)| live.then_some(index))
    }
    pub fn retirement_subtree(&self, id: u64) -> Option<Vec<usize>> {
        let root = self.instance_index(id)?;
        if self.parents[root].is_some() { return None; }
        let mut result = vec![root];
        let mut cursor = 0;
        while cursor < result.len() {
            let parent = result[cursor];
            for (index, binding) in self.parents.iter().enumerate() {
                if self.live[index]
                    && binding.is_some_and(|(owner, _)| owner == parent) {
                    result.push(index);
                }
            }
            cursor += 1;
        }
        if result.iter().any(|&index| self.runtimes[index].is_none()) { return None; }
        Some(result)
    }
    /// Receiver teardown must run first, while the instance IDs are valid.
    pub fn retire_detached(&mut self, indices: &[usize]) -> Result<(), String> {
        if indices.iter().any(|&index| self.live.get(index) == Some(&true)
            && self.runtimes.get(index).is_some_and(Option::is_none)) {
            return Err("graph receiver is executing; defer teardown until its action returns".into());
        }
        if self.story_root.and_then(|root|self.instance_index(root)).is_some_and(|root|indices.contains(&root)) {
            self.story_root=None;
            for runtime in self.runtimes.iter_mut().flatten() {
                if let Some(state)=runtime.story.as_mut(){state.bindings.clear();state.ready=false;state.named_ready=false;}
            }
        }
        for runtime in self.runtimes.iter_mut().flatten() {
            if let Some(state)=runtime.story.as_mut(){
                let before=state.bindings.len();
                state.bindings.retain(|_,binding|!indices.contains(&(binding.target as u32 as usize)));
                if before!=state.bindings.len(){state.ready=false;state.named_ready=false;}
            }
        }
        self.story_listeners.retain(|_,listeners|{
            listeners.retain(|(graph,_)|!indices.contains(&(*graph as u32 as usize)));
            !listeners.is_empty()
        });
        self.events.retain(|event| !indices.contains(&event.graph()));
        self.children.retain(|(parent, _), child| !indices.contains(parent) && !indices.contains(child));
        self.failures.retain(|(parent, _), _| !indices.contains(parent));
        for &index in indices.iter().rev() {
            if let Some(runtime) = self.runtimes.get_mut(index) {
                if runtime.take().is_some() {
                    self.parents[index] = None;
                    self.effectful[index] = false;
                    self.live[index] = false;
                    self.free.push(index);
                }
            }
        }
        Ok(())
    }
    pub fn broken(&mut self, id: u64, node: u32, entity: crate::gameplay::event_graph::EntityReference, step: i32, removed: bool, location: [f32; 3]) -> Result<Vec<String>, String> {
        let index = self.instance_index(id).ok_or("breakable graph retired")?;
        self.runtimes[index].as_mut().ok_or("breakable graph busy")?.broken(node, entity, step, removed, location)?;
        Ok(self.collect(index))
    }
    pub fn breakable_subscriptions(&self, graph: u64) -> Result<(Vec<(u32, crate::gameplay::event_graph::EntityReference)>, Vec<String>), String> {
        let index = self.instance_index(graph).ok_or("breakable graph retired")?;
        Ok(self.runtimes[index].as_ref().ok_or("breakable graph busy")?.breakable_subscriptions())
    }
    pub fn breakable_subscription(&self, graph: u64, node: u32) -> Result<Option<crate::gameplay::event_graph::EntityReference>, String> {
        let index = self.instance_index(graph).ok_or("breakable graph retired")?;
        self.runtimes[index].as_ref().ok_or("breakable graph busy")?.breakable_subscription(node)
    }
    pub fn picked_up(&mut self, id: u64, node: u32, successful: bool, from_grapple: bool)
        -> Result<Vec<String>, String> {
        let index = self.instance_index(id).ok_or("pickup notification graph has retired")?;
        self.runtimes.get_mut(index).and_then(Option::as_mut).ok_or("graph instance absent or busy")?
            .picked_up(node, successful, from_grapple)?;
        Ok(self.collect(index))
    }
    pub fn pickup_subscriptions(&self, graph: u64)
        -> Result<(Vec<(u32, crate::gameplay::event_graph::EntityReference)>, Vec<String>), String> {
        let index = self.instance_index(graph).ok_or("pickup graph has retired")?;
        let runtime = self.runtimes.get(index).and_then(Option::as_ref).ok_or("pickup graph is busy")?;
        Ok(runtime.pickup_subscriptions())
    }
    pub fn pickup_subscription(&self, graph: u64, node: u32) -> Result<Option<crate::gameplay::event_graph::EntityReference>, String> {
        let index = self.instance_index(graph).ok_or("pickup graph retired")?;
        self.runtimes[index].as_ref().ok_or("pickup graph busy")?.pickup_subscription(node)
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
        let roots = self.runtimes.iter().enumerate().filter_map(|(index, _)|
            self.parents[index].is_none().then(|| self.instance_id(index)).flatten()).collect::<Vec<_>>();
        let mut errors = Vec::new();
        for root in roots { if let Err(error) = self.queue_loading_end(root) { errors.push(error); } }
        errors.extend(self.startup());
        errors
    }
    /// Drain only the native startup callback list. Their emitted ordinary
    /// actions stay in the manager's alternating update queues until gameplay.
    pub fn startup(&mut self) -> Vec<String> {
        self.events.start();
        let mut errors = Vec::new();
        while let Some(event) = self.events.pop_startup() {
            match event {
                GraphEvent::LoadingEnd { graph, node } => {
                    if !self.live[graph] { continue; }
                    self.events.startup_callback(true);
                    if let Err(error) = self.runtimes[graph].as_mut().ok_or_else(|| "startup graph busy".to_owned())
                        .and_then(|runtime| runtime.loading_end_node(node)) { errors.push(error.to_owned()); }
                    errors.extend(self.collect(graph));
                    self.events.startup_callback(false);
                }
                GraphEvent::Action { .. } => { errors.push("ordinary action entered the startup callback list".into()); break; }
            }
        }
        errors
    }
    pub fn collect(&mut self, index: usize) -> Vec<String> {
        let mut errors = Vec::new();
        let pending = self[index].as_mut().unwrap().take_pending();
        let updates=self[index].as_mut().and_then(|runtime|runtime.story.as_mut())
            .map(|state|std::mem::take(&mut state.updates)).unwrap_or_default();
        for update in &updates {
            if let Err(error)=self.story_updated(index,update){errors.push(error);}
        }
        if let Some(state)=self[index].as_mut().and_then(|runtime|runtime.story.as_mut()) {
            let mut scratch=updates;scratch.clear();state.updates=scratch;
        }
        for (node, port) in pending {
            if let Err(error) = self.insert(index, node, port) {
                errors.push(format!("{} node {node}: {error}", self.owner(index)));
            }
        }
        errors
    }
    fn insert(&mut self, index: usize, id: u32, port: u32) -> Result<(), String> {
        let definition = self[index].as_ref().ok_or("FX graph busy")?.shared_definition();
        let node = definition.node(id)?;
        if node.class == "arkKisculeNodeDeclInstance" {
            if let Some(error) = self.failures.get(&(index, id)) {
                return Err(error.clone());
            }
            let Some(child) = self.children.get(&(index, id)).copied() else {
                // An empty declaration has no child: 7bb9d0 falls through to
                // ordinary dispatch instead of inventing an auto-exit.
                self.events.push(GraphEvent::Action { graph: index, node: id, port });
                return Ok(());
            };
            let name = &node
                .ports
                .iter()
                .find(|point| point.id == port)
                .ok_or("declaration input port absent")?
                .name;
            let child_definition = self[child].as_ref().unwrap().shared_definition();
            let entries = child_definition.nodes.iter()
                .filter(|node| {
                    node.class == "arkKisculeNodeDeclEntry"
                        && atom(&node.params, "m_entryName").unwrap_or("") == name
                })
                ;
            let mut auto_exit = false;
            let mut refreshed = false;
            for entry in entries.clone() {
                let runtime = self[child].as_ref().unwrap();
                auto_exit |= runtime.boolean_parameter(entry, "m_autoExit", true)?;
                if !refreshed
                    && runtime.boolean_parameter(entry, "m_refreshGlobalVariables", true)?
                {
                    let notify =
                        runtime.boolean_parameter(entry, "m_sendUpdateNotifications", true)?;
                    let definition = runtime.shared_definition();
                    for variable in &definition.variables {
                        let mut runtime = self[child].take().unwrap();
                        let result =
                            runtime.refresh_variable(self[index].as_ref().unwrap(), id, variable);
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
            self.events.push(GraphEvent::Action { graph: index, node: id, port });
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

fn gameflow_action(class: &str) -> bool {
    matches!(class, "arkKisculeNodeActionGameflowMission" | "arkKisculeNodeActionGameflowObjective" | "arkKisculeNodeActionGameflowTask" | "arkKisculeNodeActionGameflowMarkerControl")
}
