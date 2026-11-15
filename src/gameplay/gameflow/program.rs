//! Compact executable projection of arkTree gameflow nodes. Generic declaration
//! syntax is discarded after acquisition; journal definitions share one Arc.
use super::*;
use crate::gameplay::options::decl::read_edit;
use std::sync::{
    Mutex,
    atomic::{AtomicI32, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Mission,
    Objective,
    Task,
}
#[derive(Clone, Copy, Debug)]
pub struct Update {
    pub domain: Domain,
    pub name: u32,
    pub status: i32,
}
#[derive(Clone, Copy)]
enum Kind {
    Named(Domain, bool),
    Monitor(Domain, u32, i32),
    Sequence,
    Selector,
    Parallel(bool, bool),
    Always(bool),
    Flip,
    Loop(i32),
    Reference,
}
struct Node {
    kind: Kind,
    id: u32,
    name: String,
    children: Vec<usize>,
}
pub(crate) type ProgramRegistry = crate::assets::resource_registry::Registry<Program>;
pub struct Program {
    nodes: Vec<Node>,
    identities: HashMap<u32, (Domain, usize)>,
    missions: HashMap<u32, Arc<Mission>>,
    pub(crate) paths: Vec<String>,
    manager: Arc<str>,
}
pub struct Mailbox {
    program: Arc<Program>,
    pending: Mutex<Vec<Update>>,
    portrait_requests: Mutex<Vec<(u32, Portrait)>>,
    statuses: Vec<AtomicI32>,
    notifications: Mutex<Vec<Update>>,
    markers: Mutex<Arc<HashMap<u32, Vec<MarkerTarget>>>>,
}
impl Mailbox {
    pub fn submit_portrait(&self, id: u32, portrait: Portrait) -> Result<bool, String> {
        if !self
            .program
            .identities
            .get(&id)
            .is_some_and(|(domain, _)| *domain == Domain::Task)
        {
            return Ok(false);
        }
        self.portrait_requests
            .lock()
            .map_err(|_| "gameflow portrait owner poisoned")?
            .push((id, portrait));
        Ok(true)
    }
    /// 6711a0/671270/671330 mutate target pairs synchronously, independent
    /// of whether the task is currently shown. Only changed lists are copied.
    pub fn control_marker(
        &self,
        task: u32,
        operation: &str,
        target: MarkerTarget,
        replacement: MarkerTarget,
    ) -> Result<bool, String> {
        let Some(definition) = self
            .program
            .missions
            .values()
            .flat_map(|mission| &mission.objectives)
            .flat_map(|objective| &objective.tasks)
            .find(|item| crate::assets::lang::key_id(&item.name) == task)
        else {
            return Ok(false);
        };
        let mut owner = self
            .markers
            .lock()
            .map_err(|_| "objective marker owner poisoned")?;
        let list = owner
            .get(&task)
            .map(Vec::as_slice)
            .unwrap_or(&definition.markers);
        let at = list.iter().position(|item| *item == target);
        let valid = match operation {
            "Add Target" => at.is_none(),
            "Remove Target" | "Replace Target" => at.is_some(),
            _ => return Err(format!("unknown marker action {operation}")),
        };
        if !valid {
            return Ok(false);
        }
        let list = Arc::make_mut(&mut *owner)
            .entry(task)
            .or_insert_with(|| definition.markers.clone());
        match operation {
            "Add Target" => list.push(target),
            "Remove Target" => {
                list.remove(at.unwrap());
            }
            "Replace Target" => list[at.unwrap()] = replacement,
            _ => unreachable!(),
        };
        Ok(true)
    }
    pub fn publish_markers(&self, state: &mut State) {
        let owner = self
            .markers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !Arc::ptr_eq(&owner, &state.marker_overrides) {
            state.marker_overrides = owner.clone();
        }
    }
    pub fn take_notifications(&self, buffer: &mut Vec<Update>) -> Result<(), String> {
        buffer.clear();
        std::mem::swap(
            buffer,
            &mut *self
                .notifications
                .lock()
                .map_err(|_| "gameflow notification owner poisoned")?,
        );
        Ok(())
    }
    pub fn status(&self, domain: Domain, name: u32) -> Option<i32> {
        self.program
            .identities
            .get(&name)
            .filter(|(kind, _)| *kind == domain)
            .map(|&(_, node)| self.statuses[node].load(Ordering::Relaxed))
    }

    pub fn submit(&self, update: Update) -> Result<bool, String> {
        if !(0..=6).contains(&update.status) {
            return Err("gameflow status outside native enum".into());
        }
        if !self
            .program
            .identities
            .get(&update.name)
            .is_some_and(|&(domain, _)| domain == update.domain)
        {
            return Ok(false);
        }
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| "gameflow request owner poisoned")?;
        pending.push(update);
        // 530550/530660 also cancel/fail every task of an active objective.
        // Its decorator is waiting on those child continuations and cannot
        // consume the objective request until they return.
        if update.domain == Domain::Objective
            && matches!(update.status, 4 | 5)
            && self.status(update.domain, update.name) == Some(2)
        {
            let (_, node) = self.program.identities[&update.name];
            for objective in self
                .program
                .missions
                .values()
                .flat_map(|mission| &mission.objectives)
                .filter(|objective| objective.name == self.program.nodes[node].name)
            {
                pending.extend(objective.tasks.iter().map(|task| Update {
                    domain: Domain::Task,
                    name: crate::assets::lang::key_id(&task.name),
                    status: update.status,
                }));
            }
        }
        Ok(true)
    }
}

fn status(value: &str) -> Result<i32, String> {
    Ok(match value {
        "GAMEFLOW_NODESTATE_UNINITIALIZED" => 0,
        "GAMEFLOW_NODESTATE_INACTIVE" => 1,
        "GAMEFLOW_NODESTATE_ACTIVE" => 2,
        "GAMEFLOW_NODESTATE_COMPLETE" => 3,
        "GAMEFLOW_NODESTATE_FAILED" => 4,
        "GAMEFLOW_NODESTATE_CANCELED" => 5,
        _ => value
            .parse()
            .map_err(|_| format!("unknown gameflow status {value}"))?,
    })
}
impl Program {
    pub fn marker_entities(&self) -> impl Iterator<Item = &str> {
        self.missions
            .values()
            .flat_map(|mission| &mission.objectives)
            .flat_map(|objective| &objective.tasks)
            .flat_map(|task| &task.markers)
            .map(|target| target.entity.as_str())
    }
    pub fn mission(&self, name: &str) -> Option<Arc<Mission>> {
        self.missions
            .get(&crate::assets::lang::key_id(name))
            .cloned()
    }
    pub fn reachable_mission(&self, name: &str) -> Option<Arc<Mission>> {
        fn reaches(program: &Program, index: usize, wanted: u32) -> bool {
            let node = &program.nodes[index];
            match node.kind {
                Kind::Named(Domain::Mission, _) => node.id == wanted,
                Kind::Parallel(..) => node
                    .children
                    .iter()
                    .any(|&child| reaches(program, child, wanted)),
                Kind::Reference | Kind::Selector if node.children.len() == 1 => {
                    reaches(program, node.children[0], wanted)
                }
                _ => false,
            }
        }
        reaches(self, 0, crate::assets::lang::key_id(name))
            .then(|| self.mission(name))
            .flatten()
    }
    pub fn inventory(&self) -> (usize, usize, usize, usize, usize) {
        let objectives = self
            .identities
            .values()
            .filter(|(domain, _)| *domain == Domain::Objective)
            .count();
        let tasks = self
            .identities
            .values()
            .filter(|(domain, _)| *domain == Domain::Task)
            .count();
        (
            self.nodes.len(),
            self.missions.len(),
            objectives,
            tasks,
            self.paths.len(),
        )
    }

    pub fn load(catalog: &Catalog, manager: &str) -> Result<Arc<Self>, String> {
        let path = format!(
            "generated/decls/gameflowmanager/{}.decl",
            manager.to_ascii_lowercase()
        );
        catalog
            .gameflow_programs
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| "gameflow declaration owner poisoned")?
            .acquire_with_sharing(&path, 0, !catalog.has_overlays(), |_| {
                let mut read = |path: &str| catalog.read(path).ok().flatten();
                let fields = read_edit(&mut read, "gameflowmanager", manager)?;
                let root =
                    atom(&fields, "m_gameFlowTree").ok_or("gameflow manager has no root tree")?;
                let mut program = Self {
                    nodes: Vec::new(),
                    identities: HashMap::new(),
                    missions: HashMap::new(),
                    paths: vec![path.clone()],
                    manager: Arc::from(manager),
                };
                program.tree(catalog, root, None, &mut HashSet::new())?;
                Ok(Arc::new(program))
            })
    }
    fn tree(
        &mut self,
        catalog: &Catalog,
        path: &str,
        parent: Option<usize>,
        visiting: &mut HashSet<String>,
    ) -> Result<usize, String> {
        let path = path.to_ascii_lowercase();
        if !visiting.insert(path.clone()) {
            return Err(format!("cyclic gameflow tree {path}"));
        }
        let resource = format!("generated/decls/arktree/{path}.decl");
        let mut read = |path: &str| catalog.read(path).ok().flatten();
        let fields = read_edit(&mut read, "arktree", &path)?;
        self.paths.push(resource);
        let root = scope(&fields, "root").ok_or("gameflow tree has no root")?;
        let index = self.node(catalog, root, parent, visiting)?;
        visiting.remove(&path);
        Ok(index)
    }
    fn node(
        &mut self,
        catalog: &Catalog,
        fields: &Fields,
        parent: Option<usize>,
        visiting: &mut HashSet<String>,
    ) -> Result<usize, String> {
        let (class, data) = settings(fields);
        let mut name = String::new();
        let kind = match class {
            "arkTreeNodeSettings_Decorator_GameflowMission" => {
                name = text(data, "m_missionName");
                self.missions.insert(
                    crate::assets::lang::key_id(&name),
                    Arc::new(build_mission(fields, data, &name)),
                );
                Kind::Named(Domain::Mission, false)
            }
            "arkTreeNodeSettings_Decorator_GameflowObjective" => {
                name = text(data, "m_objectiveName");
                Kind::Named(Domain::Objective, boolean(data, "m_autoActivate", true))
            }
            "arkTreeNodeSettings_Action_GameflowTask" => {
                name = text(data, "m_taskName");
                Kind::Named(Domain::Task, boolean(data, "m_autoActivate", true))
            }
            "arkTreeNodeSettings_Monitor_GameflowTask"
            | "arkTreeNodeSettings_Monitor_GameflowObjective" => {
                if boolean(data, "m_monitor", false) || boolean(data, "m_not", false) {
                    return Err(format!("unsupported gameflow monitor mode {class}"));
                }
                let domain = if class.ends_with("Task") {
                    Domain::Task
                } else {
                    Domain::Objective
                };
                let field = if domain == Domain::Task {
                    "m_taskName"
                } else {
                    "m_objectiveName"
                };
                Kind::Monitor(
                    domain,
                    crate::assets::lang::key_id(atom(data, field).unwrap_or("")),
                    status(atom(data, "m_returnedState").unwrap_or("GAMEFLOW_NODESTATE_COMPLETE"))?,
                )
            }
            "arkTreeNodeSettings_Sequence" => Kind::Sequence,
            "arkTreeNodeSettings_Selector" => Kind::Selector,
            "arkTreeNodeSettings_Parallel" | "arkTreeNodeSettings_Parallel_GameflowOr" => {
                Kind::Parallel(
                    boolean(data, "m_returnOnFirstFail", false),
                    boolean(data, "m_returnOnFirstSuccess", false),
                )
            }
            "arkTreeNodeSettings_Decorator_AlwaysFail" => Kind::Always(false),
            "arkTreeNodeSettings_Decorator_AlwaysSucceed" => Kind::Always(true),
            "arkTreeNodeSettings_Decorator_FlipCondition" => Kind::Flip,
            "arkTreeNodeSettings_Loop" => Kind::Loop(
                atom(data, "m_stopCondition")
                    .unwrap_or("-1")
                    .parse()
                    .map_err(|_| "invalid loop stop condition")?,
            ),
            "arkTreeNodeSettings_TreeReference" => Kind::Reference,
            _ => return Err(format!("unsupported executable gameflow node {class}")),
        };
        let index = self.nodes.len();
        if let Kind::Named(domain, _) = kind {
            if name.is_empty()
                || self
                    .identities
                    .insert(crate::assets::lang::key_id(&name), (domain, index))
                    .is_some()
            {
                return Err(format!("duplicate/empty gameflow identity {name}"));
            }
        }
        self.nodes.push(Node {
            kind,
            id: crate::assets::lang::key_id(&name),
            name,
            children: Vec::new(),
        });
        if matches!(kind, Kind::Reference) {
            let path = atom(data, "m_subTree").ok_or("gameflow tree reference is empty")?;
            let child = self.tree(catalog, path, Some(index), visiting)?;
            self.nodes[index].children.push(child);
        } else {
            for child in children(fields) {
                let child = self.node(catalog, child, Some(index), visiting)?;
                self.nodes[index].children.push(child);
            }
        }
        if matches!(kind, Kind::Parallel(..)) && self.nodes[index].children.len() != 2 {
            return Err(format!(
                "gameflow parallel {index} must have its two native continuations"
            ));
        }
        Ok(index)
    }
}

#[derive(Clone, Copy)]
struct NodeState {
    last: u8,
    second: u8,
    cursor: isize,
    active: bool,
    waiting: bool,
    loop_push: bool,
    remaining: i32,
    stack: usize,
    branch: Option<usize>,
    status: i32,
}
impl Default for NodeState {
    fn default() -> Self {
        Self {
            last: 2,
            second: 2,
            cursor: -1,
            active: false,
            waiting: false,
            loop_push: false,
            remaining: 0,
            stack: 0,
            branch: None,
            status: 0,
        }
    }
}
#[derive(Default)]
struct Stack {
    nodes: Vec<usize>,
    owner: Option<usize>,
}
enum Decision {
    Done(u8),
    Yield,
    Push(usize),
    Fork(usize, usize),
}
pub struct Runtime {
    program: Arc<Program>,
    mailbox: Arc<Mailbox>,
    states: Vec<NodeState>,
    pending: Vec<Update>,
    changes: Vec<(usize, i32)>,
    stacks: Vec<Stack>,
    root_result: u8,
    portrait_requests: Vec<(u32, Portrait)>,
    portraits: Arc<HashMap<u32, Portrait>>,
}
impl Runtime {
    pub fn new(program: Arc<Program>) -> Self {
        let states = vec![NodeState::default(); program.nodes.len()];
        let mailbox = Arc::new(Mailbox {
            program: program.clone(),
            pending: Mutex::new(Vec::new()),
            portrait_requests: Mutex::new(Vec::new()),
            notifications: Mutex::new(Vec::new()),
            markers: Mutex::new(Arc::default()),
            statuses: (0..program.nodes.len())
                .map(|_| AtomicI32::new(0))
                .collect(),
        });
        Self {
            program,
            mailbox,
            states,
            pending: Vec::new(),
            changes: Vec::new(),
            stacks: vec![Stack::default()],
            root_result: 2,
            portrait_requests: Vec::new(),
            portraits: Arc::default(),
        }
    }
    pub fn marker_entities(&self) -> impl Iterator<Item = &str> {
        self.program.marker_entities()
    }
    pub fn mailbox(&self) -> Arc<Mailbox> {
        self.mailbox.clone()
    }
    pub fn publish_portraits(&self, progress: &mut State) {
        if !Arc::ptr_eq(&self.portraits, &progress.portraits) {
            progress.portraits = self.portraits.clone();
        }
    }
    pub fn publish(&mut self, progress: &mut State) -> Vec<super::ObjectiveUpdate> {
        let mut notifications = Vec::new();
        for (index, status) in self.changes.drain(..) {
            let node = &self.program.nodes[index];
            progress.set_status(node.name.clone(), status);
            if matches!(node.kind, Kind::Named(Domain::Task, _)) {
                if let Some(notification) = progress.task_update(&node.name, status) {
                    if let Some(portrait) = &notification.portrait {
                        progress.mark_task_notified(node.id);
                        progress.notify_target(portrait);
                    }
                    notifications.push(notification);
                }
            }
            if matches!(node.kind, Kind::Named(Domain::Mission, _)) && status == 2 {
                progress.mission = self
                    .program
                    .missions
                    .get(&crate::assets::lang::key_id(&node.name))
                    .cloned();
                progress.manager = Some(self.program.manager.clone());
            }
        }
        notifications
    }

    pub fn changed(&self) -> bool {
        !self.changes.is_empty()
    }
    fn set_status(&mut self, index: usize, status: i32) {
        if self.states[index].status == status {
            return;
        }
        self.states[index].status = status;
        self.mailbox.statuses[index].store(status, Ordering::Relaxed);
        self.changes.push((index, status));
        if let Kind::Named(domain, _) = self.program.nodes[index].kind {
            let name = self.program.nodes[index].id;
            self.mailbox
                .notifications
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(Update {
                    domain,
                    name,
                    status,
                });
        }
    }
    fn command(&mut self, index: usize, domain: Domain) {
        let id = self.program.nodes[index].id;
        let old = self.states[index].status;
        let allowed = |next| match domain {
            Domain::Mission => (old, next) == (0, 2),
            Domain::Objective => matches!(
                (old, next),
                (1, 2) | (1, 5) | (2, 4) | (2, 5) | (3, 2) | (3, 4)
            ),
            Domain::Task => matches!(
                (old, next),
                (1, 2)
                    | (1, 3)
                    | (1, 4)
                    | (1, 5)
                    | (2, 0)
                    | (2, 3)
                    | (2, 4)
                    | (2, 5)
                    | (3, 2)
                    | (4, 2)
                    | (5, 0)
            ),
        };
        if let Some(at) = self.pending.iter().position(|update| {
            update.domain == domain && update.name == id && allowed(update.status)
        }) {
            let update = self.pending.remove(at);
            self.set_status(index, update.status);
        }
    }
    /// Task updates 40eed0/40f1d0/40fa70; child continuations 5c1100/5c1110;
    /// traversal 5d8440/5d83c0. Running without a child yields until next update.
    fn decision(&mut self, index: usize) -> Decision {
        let kind = self.program.nodes[index].kind;
        match kind {
            Kind::Named(domain, auto) => {
                let before = self.pending.len();
                self.command(index, domain);
                // 40fa70 consumes portrait parameters only if no state request
                // was accepted by this task in the current update.
                if domain == Domain::Task && self.pending.len() == before {
                    let id = self.program.nodes[index].id;
                    if let Some(at) = self
                        .portrait_requests
                        .iter()
                        .position(|(task, _)| *task == id)
                    {
                        let (_, portrait) = self.portrait_requests.remove(at);
                        Arc::make_mut(&mut self.portraits).insert(id, portrait);
                    }
                }
                match self.states[index].status {
                    3 => Decision::Done(1),
                    4 | 5 => Decision::Done(0),
                    0 if domain != Domain::Mission => {
                        self.set_status(index, if auto { 2 } else { 1 });
                        Decision::Yield
                    }
                    2 if domain != Domain::Task => {
                        let answer = self.delegate(index);
                        if let Decision::Done(result) = answer {
                            self.set_status(index, if result == 1 { 3 } else { 4 });
                        }
                        answer
                    }
                    _ => Decision::Yield,
                }
            }
            Kind::Monitor(domain, id, wanted) => {
                let matched = self
                    .program
                    .identities
                    .get(&id)
                    .is_some_and(|&(kind, node)| {
                        kind == domain && self.states[node].status == wanted
                    })
                    || self.pending.iter().any(|update| {
                        update.domain == domain && update.name == id && update.status == wanted
                    });
                if matched {
                    Decision::Done(1)
                } else {
                    Decision::Yield
                }
            }
            Kind::Sequence | Kind::Selector => {
                let terminal = u8::from(matches!(kind, Kind::Selector));
                if self.states[index].last == terminal {
                    return Decision::Done(terminal);
                }
                self.states[index].cursor += 1;
                match self.program.nodes[index]
                    .children
                    .get(self.states[index].cursor as usize)
                {
                    Some(&child) => Decision::Push(child),
                    None => Decision::Done(1 - terminal),
                }
            }
            Kind::Parallel(first_fail, first_success) => {
                let state = self.states[index];
                if state.last == 2 {
                    let children = &self.program.nodes[index].children;
                    // The installed parallel task has two child continuations:
                    // the last on this stack, the first on its auxiliary stack.
                    if children.len() != 2 {
                        return Decision::Done(0);
                    }
                    Decision::Fork(children[1], children[0])
                } else if state.second != 2 {
                    Decision::Done(state.second)
                } else if (first_fail && state.last == 0) || (first_success && state.last == 1) {
                    Decision::Done(state.last)
                } else {
                    Decision::Yield
                }
            }
            Kind::Loop(condition) => {
                let state = &mut self.states[index];
                if state.last == 2 {
                    state.loop_push = true;
                } else if (condition == 0 && state.last == 0) || (condition < 0 && state.last == 1)
                {
                    return Decision::Done(state.last);
                } else if condition > 0 {
                    state.remaining -= 1;
                    if state.remaining == 0 {
                        return Decision::Done(state.last);
                    }
                }
                if state.loop_push {
                    state.loop_push = false;
                    self.program.nodes[index]
                        .children
                        .first()
                        .copied()
                        .map(Decision::Push)
                        .unwrap_or(Decision::Done(0))
                } else {
                    state.loop_push = true;
                    Decision::Yield
                }
            }
            Kind::Always(value) => match self.delegate(index) {
                Decision::Done(_) => Decision::Done(u8::from(value)),
                answer => answer,
            },
            Kind::Flip => match self.delegate(index) {
                Decision::Done(result) => Decision::Done(1 - result),
                answer => answer,
            },
            Kind::Reference => self.delegate(index),
        }
    }
    fn delegate(&self, index: usize) -> Decision {
        if self.states[index].last != 2 {
            Decision::Done(self.states[index].last)
        } else if self.program.nodes[index].children.len() == 1 {
            Decision::Push(self.program.nodes[index].children[0])
        } else {
            Decision::Done(0)
        }
    }
    fn push(&mut self, stack: usize, node: usize) {
        self.states[node] = NodeState {
            status: self.states[node].status,
            active: true,
            stack,
            remaining: match self.program.nodes[node].kind {
                Kind::Loop(count) => count,
                _ => 0,
            },
            ..Default::default()
        };
        self.stacks[stack].nodes.push(node);
    }
    fn child_finished(&mut self, parent: usize, result: u8) {
        let state = &mut self.states[parent];
        if let Kind::Parallel(first_fail, first_success) = self.program.nodes[parent].kind {
            if state.last == 2 {
                state.last = result;
                if (first_fail && result == 0) || (first_success && result == 1) {
                    state.waiting = false;
                }
            } else if state.waiting {
                state.second = result;
                state.waiting = false;
            }
        } else {
            state.last = result;
            state.waiting = false;
        }
    }
    fn abort_above(&mut self, stack: usize, parent: Option<usize>) {
        while let Some(&node) = self.stacks[stack].nodes.last() {
            if Some(node) == parent {
                break;
            }
            self.stacks[stack].nodes.pop();
            if let Some(branch) = self.states[node].branch.take() {
                self.abort_above(branch, None);
                self.stacks[branch].owner = None;
            }
            if matches!(self.program.nodes[node].kind, Kind::Named(..)) {
                self.set_status(node, 5);
            }
            self.states[node].active = false;
        }
    }
    fn run_stack(&mut self, stack: usize) -> Result<(), String> {
        // Same infinite-loop guard as 5d8440; it is an error, never a time budget.
        for _ in 0..1000 {
            let Some(&node) = self.stacks[stack].nodes.last() else {
                return Ok(());
            };
            if self.states[node].waiting {
                return Ok(());
            }
            match self.decision(node) {
                Decision::Yield => return Ok(()),
                Decision::Push(child) => {
                    self.states[node].waiting = true;
                    self.push(stack, child);
                }
                Decision::Fork(last, first) => {
                    let branch = (stack + 1..self.stacks.len())
                        .find(|&at| self.stacks[at].owner.is_none())
                        .unwrap_or_else(|| {
                            self.stacks.push(Stack::default());
                            self.stacks.len() - 1
                        });
                    self.stacks[branch].owner = Some(node);
                    self.states[node].branch = Some(branch);
                    self.states[node].waiting = true;
                    self.push(stack, last);
                    self.push(branch, first);
                }
                Decision::Done(result) => {
                    self.stacks[stack].nodes.pop();
                    self.states[node].active = false;
                    if let Some(branch) = self.states[node].branch.take() {
                        self.abort_above(branch, None);
                        self.stacks[branch].owner = None;
                    }
                    if let Some(&parent) = self.stacks[stack].nodes.last() {
                        self.child_finished(parent, result);
                    } else if let Some(parent) = self.stacks[stack].owner {
                        self.child_finished(parent, result);
                        if !self.states[parent].waiting {
                            self.abort_above(self.states[parent].stack, Some(parent));
                        }
                        return Ok(());
                    } else {
                        self.root_result = result;
                        return Ok(());
                    }
                }
            }
        }
        Err("gameflow traversal infinite loop (1000 continuations)".into())
    }
    pub fn update(&mut self) -> Result<(), String> {
        let mut messages = self
            .mailbox
            .pending
            .lock()
            .map_err(|_| "gameflow request owner poisoned")?;
        self.pending.extend(messages.drain(..));
        drop(messages);
        self.portrait_requests.extend(
            self.mailbox
                .portrait_requests
                .lock()
                .map_err(|_| "gameflow portrait owner poisoned")?
                .drain(..),
        );
        // Requests never resurrect a completed traversal. Re-entry comes from
        // the authored loop/reference, exactly as task allocation in 5d8a30.
        if self.stacks[0].nodes.is_empty() && self.root_result == 2 {
            self.push(0, 0);
        }
        self.run_stack(0)?;
        let mut stack = 1;
        while stack < self.stacks.len() {
            if self.stacks[stack].owner.is_some() {
                self.run_stack(stack)?;
            }
            stack += 1;
        }
        Ok(())
    }
}

pub(crate) fn release_marker(catalog: &Catalog, marker: u8) {
    if let Some(owner) = catalog.gameflow_programs.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.release(catalog.identity(), "gameflowManager", marker);
        let sources = owner
            .live_values()
            .flat_map(|program| program.paths.clone())
            .collect::<HashSet<_>>();
        for kind in ["gameflowManager", "arkTree"] {
            crate::loading::retain_catalog_kind(catalog.identity(), kind, |path| {
                owner.owns_read_history(path) || sources.contains(path)
            });
        }
    }
}

pub(crate) fn register_preload(catalog: &Catalog, path: &str, marker: u8) -> Result<(), String> {
    catalog
        .gameflow_programs
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "gameflow declaration owner poisoned")?
        .register(&path.to_ascii_lowercase(), marker, false);
    Ok(())
}
pub(crate) fn promote_bootstrap(catalog: &Catalog) -> Result<(), String> {
    if let Some(owner) = catalog.gameflow_programs.get() {
        owner
            .lock()
            .map_err(|_| "gameflow declaration owner poisoned")?
            .promote_bootstrap();
    }
    Ok(())
}
