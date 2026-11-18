//! Shared variable and entity inputs for authored event graphs. Domain owners
//! retain node state and decide which actions to execute.
use crate::assets::entities::event_graph::{atom, Node, Port};

/// The Kiscule manager owns a startup stack and two alternating update
/// stacks (7bf290 / 7c10b0). Ordinary callbacks enqueue into the current
/// stack only while the manager is processing; external callbacks enqueue
/// into the next stack. Work emitted by a startup callback enters the current
/// update stack, which is not the startup stack being drained.
pub struct ManagerWork<T> {
    startup: Vec<T>,
    current: Vec<T>,
    deferred: Vec<T>,
    started: bool,
    processing: bool,
    draining_startup: bool,
    startup_callback: bool,
}
impl<T> Default for ManagerWork<T> {
    fn default() -> Self {
        Self { startup: Vec::new(), current: Vec::new(), deferred: Vec::new(),
            started: false, processing: false, draining_startup: false, startup_callback: false }
    }
}
impl<T> ManagerWork<T> {
    pub fn push(&mut self, item: T) {
        if self.startup_callback || (self.started && self.processing) { self.current.push(item); }
        else if self.started { self.deferred.push(item); }
        else { self.startup.push(item); }
    }
    /// A callback returning the same action kind is deferred (7c1202).
    pub fn push_deferred(&mut self, item: T) {
        if self.started { self.deferred.push(item); } else { self.startup.push(item); }
    }
    /// Used for the already-evaluated LoadingEnd startup callbacks until the
    /// entity manager supplies individual startup-node dispatch.
    pub fn startup_callback(&mut self, enabled: bool) { self.startup_callback = enabled; }
    /// Called by the manager's startup owner, after its actual readiness gate.
    pub fn start(&mut self) {
        if self.started { return; }
        self.started = true;
        self.processing = true;
        self.draining_startup = true;
    }
    pub fn begin_update(&mut self) {
        if !self.started { return; }
        // An owner error may have interrupted the previous drain. Resume that
        // same list rather than swapping away its remaining work.
        if self.processing { return; }
        self.processing = true;
        std::mem::swap(&mut self.current, &mut self.deferred);
    }
    /// Drain startup callbacks without consuming the first ordinary action.
    /// 7bf290 crosses this phase boundary before7c10b0 updates the action list.
    pub fn pop_startup(&mut self) -> Option<T> {
        if !self.processing || !self.draining_startup { return None; }
        if let Some(item)=self.startup.pop(){return Some(item);}
        self.draining_startup=false;
        self.startup=Vec::new();
        std::mem::swap(&mut self.current,&mut self.deferred);
        None
    }
    pub fn pop(&mut self) -> Option<T> {
        if !self.processing { return None; }
        if self.draining_startup {
            if let Some(item)=self.pop_startup(){return Some(item);}
        }
        match self.current.pop() {
            Some(item) => Some(item),
            None => { self.processing = false; None }
        }
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        self.startup.retain(&mut keep);
        self.current.retain(&mut keep);
        self.deferred.retain(keep);
    }
    /// Borrow the retained queues for resource diagnostics. This does not
    /// enter an execution phase or alter the manager's dispatch order.
    pub fn pending(&self) -> impl Iterator<Item = &T> {
        self.startup.iter().chain(&self.current).chain(&self.deferred)
    }
}

/// Runtime entity identity, separate from serialized map names. Player resolves
/// through the active player owner (7a8500), never a fabricated map entity name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EntityReference {
    Named(String),
    Player,
    ///Actual weak managed pointer returned by runtime graph actions.
    ///A retired generation never resolves through a reused name or slot.
    Managed(super::entity_lifetime::EntityId),
    /// The game's invalid managed-entity handle (0x3fff0010). Lists retain
    /// these elements; dropping them would change indices and loop counts.
    Invalid,
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
    ///7be020 searches incoming variable connections by name when a
    ///mirrored output is read; outgoing notification links are not inputs.
    fn mirrored_variable_input<'a>(&self,node:&'a Node,point:&Port)->Option<&'a Port>{
        if point.kind!="OutputVariable"{return None;}
        node.ports.iter().find(|input|input.kind=="InputVariable"&&input.name==point.name)
    }
    /// Typed vector outputs include BreakableNotification's actual owner location (7986b0).
    fn vector(&self, id: u32, port: u32, depth: usize) -> Result<[f32; 3], String> {
        if depth > 64 { return Err("cyclic variable graph".into()); }
        let node = self.node(id)?;
        let point = node.ports.iter().find(|point| point.id == port).ok_or("missing vector port")?;
        if point.kind == "InputVariable" {
            if let Some(&(node, port)) = point.links.first() { return self.vector(node, port, depth + 1); }
            return super::effects::variables::read_vector(&node.params, &point.name);
        }
        if let Some(value) = self.variable_value(node, port, depth)? { return value.vector(); }
        if let Some(input)=self.mirrored_variable_input(node,point){
            if let Some(&(id,port))=input.links.first(){return self.vector(id,port,depth+1);}
            if node.params.iter().any(|(name,_)|name==&point.name){return super::effects::variables::read_vector(&node.params,&point.name);}
        }
        if node.class == "arkKisculeNodeVec3" { return super::effects::variables::read_vector(&node.params, "m_value"); }
        Err(format!("unsupported vector output {}.{}", node.class, point.name))
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
        if let Some(input)=self.mirrored_variable_input(n,p){
            if let Some(&(id,port))=input.links.first(){return self.number(id,port,depth+1);}
            if let Some(value)=atom(&n.params,&p.name){
                return match value{"true"=>Ok(1.),"false"=>Ok(0.),_=>value.parse().map_err(|_|format!("invalid number {}",p.name))};
            }
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
        //528040 ->7be020 resolves the connection by reflected variable NAME,
        //even when a caller requested the node's mirrored output port. Real
        //instance getters above retain priority over ordinary parameter reads.
        let mirrored=self.mirrored_variable_input(node,input);
        if let Some(source)=mirrored{
            if let Some(&(id,port))=source.links.first(){return self.entity_references_inner(id,port,depth+1);}
        }
        //524190 returns the actual reflected parameter field. An explicitly
        //serialized null is ONE invalid managed reference, not an empty list.
        //Do not fabricate defaults when an unknown class omitted the field.
        if input.kind=="InputVariable"||mirrored.is_some(){
            if let Some(value)=atom(&node.params,&input.name){
                return Ok(vec![if value.is_empty(){EntityReference::Invalid}else{EntityReference::Named(value.to_owned())}]);
            }
        }
        if node.class == "arkKisculeNodeListEntityReference" {
            return crate::assets::entities::event_graph::entity_reference_names(
                &node.params,
                "m_value",
            )
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| value.map_or(EntityReference::Invalid, EntityReference::Named))
                    .collect()
            });
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
    /// Mutable node values belong to the instance, not its shared definition.
    fn boolean_override(&self, _node: &Node, _name: &str) -> Option<bool> { None }
    fn boolean_parameter(&self, node: &Node, name: &str, default: bool) -> Result<bool, String> {
        if let Some(value) = self.boolean_override(node, name) { return Ok(value); }
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
        if port.kind!="InputVariable"{
            if let Some(value)=self.variable_value(node,port.id,depth)?{return Ok(value.number()?!=0.);}
        }
        if let Some(input)=self.mirrored_variable_input(node,port){
            if let Some(&(id,port))=input.links.first(){return self.boolean(id,port,depth+1);}
        }
        if port.kind=="InputVariable"||self.mirrored_variable_input(node,port).is_some(){
            match atom(&node.params,&port.name){Some("true")=>return Ok(true),Some("false")=>return Ok(false),_=>{}}
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
        if let Some(input)=self.mirrored_variable_input(node,port){
            if let Some(&(id,port))=input.links.first(){return self.integer(id,port,depth+1);}
            if let Some(value)=atom(&node.params,&port.name){return value.parse().map_err(|_|format!("invalid integer {}",port.name));}
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
    /// Declaration/enum inputs follow the authored connection before defaults.
    fn declaration_input(&self, node:&Node, name:&str, default:&str)->Result<String,String>{
        fn read(inputs:&(impl GraphInputs + ?Sized),id:u32,port:u32,depth:usize)->Result<String,String>{
            if depth>64{return Err("cyclic declaration graph".into());}
            let node=inputs.node(id)?;
            let point=node.ports.iter().find(|point|point.id==port).ok_or("declaration port absent")?;
            if point.kind=="InputVariable" {if let Some(&(id,port))=point.links.first(){return read(inputs,id,port,depth+1);}}
            if let Some(input)=inputs.mirrored_variable_input(node,point){if let Some(&(id,port))=input.links.first(){return read(inputs,id,port,depth+1);}}
            atom(&node.params,&point.name).or_else(||atom(&node.params,"m_value")).map(str::to_owned)
                .ok_or_else(||format!("unsupported declaration producer {} {}",node.class,point.name))
        }
        if let Some((id,port))=self.input(node,name){read(self,id,port,0)}else{Ok(atom(&node.params,name).unwrap_or(default).to_owned())}
    }
    /// 523db0 follows connected short-string-id outputs before authored defaults.
    fn string_id(&self,id:u32,port:u32,depth:usize)->Result<u32,String>{
        if depth>64{return Err("cyclic string-id graph".into());}
        let node=self.node(id)?;
        let point=node.ports.iter().find(|point|point.id==port).ok_or("string-id port absent")?;
        if point.kind=="InputVariable" {
            if let Some(&(id,port))=point.links.first(){return self.string_id(id,port,depth+1);}
        }
        if let Some(input)=self.mirrored_variable_input(node,point){if let Some(&(id,port))=input.links.first(){return self.string_id(id,port,depth+1);}}
        let value=atom(&node.params,&point.name).or_else(||
            (node.class.contains("StringId")||node.class.contains("StringID")).then(||atom(&node.params,"m_value")).flatten())
            .ok_or_else(||format!("unsupported string-id producer {} {}",node.class,point.name))?;
        Ok(if value.is_empty(){0}else{crate::assets::lang::key_id(value)})
    }
    fn string_id_input(&self,node:&Node,name:&str)->Result<u32,String>{
        if let Some((id,port))=self.input(node,name){self.string_id(id,port,0)}
        else{let value=atom(&node.params,name).unwrap_or("");Ok(if value.is_empty(){0}else{crate::assets::lang::key_id(value)})}
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
