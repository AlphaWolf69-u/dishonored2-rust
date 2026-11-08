//! Authored gameflow mission definitions, loaded from the manager's tree.
//!
//! Definitions are separate from live progression: a map name or an authored
//! `m_autoActivate` flag alone does not establish that a mission is active.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use crate::assets::{archive::Catalog, entities::Value};
use crate::gameplay::options::decl::{atom, boolean, list_items, read_edit, scope, unquote};

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub image: String,
}

#[derive(Clone, Debug)]
pub struct Task {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub optional: bool,
    pub auto_activate: bool,
    pub show_in_journal: bool,
    pub show_when_canceled: bool,
    pub condition_index: i32,
    pub lethality: i32,
}

#[derive(Clone, Debug)]
pub struct Objective {
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub optional: bool,
    pub auto_activate: bool,
    pub show_in_journal: bool,
    pub tasks: Vec<Task>,
}

#[derive(Clone, Debug)]
pub struct Mission {
    pub name: String,
    pub description: String,
    pub targets: Vec<Target>,
    pub objectives: Vec<Objective>,
}

type Fields = [(String, Value)];
fn text(fields: &Fields, key: &str) -> String { atom(fields, key).unwrap_or("").to_owned() }
fn image(fields: &Fields, key: &str) -> Option<String> {
    atom(fields, key).filter(|value| !value.is_empty() && *value != "NULL").map(str::to_owned)
}
fn children(fields: &Fields) -> impl Iterator<Item = &Fields> {
    scope(fields, "m_children").into_iter().flat_map(list_items).filter_map(|value| match value {
        Value::Scope(fields) => Some(fields.as_slice()), Value::Atom(_) => None,
    })
}
fn settings(node: &Fields) -> (&str, &Fields) {
    let Some(settings) = scope(node, "m_settings") else { return ("", &[]) };
    (atom(settings, "m_class").unwrap_or(""), scope(settings, "m_object").unwrap_or(&[]))
}

/// 1404e9010 visits ordered children of the objective subtree, skipping
/// nested objective/mission decorators. Parallel nodes configured to return
/// on first success (but not first failure) assign distinct child conditions.
fn gather_tasks(node: &Fields, tasks: &mut Vec<Task>, condition: i32) {
    let (class, data) = settings(node);
    let split = matches!(class, "arkTreeNodeSettings_Parallel" | "arkTreeNodeSettings_Parallel_GameflowOr")
        && boolean(data, "m_returnOnFirstSuccess", false)
        && !boolean(data, "m_returnOnFirstFail", false);
    for (index, child) in children(node).enumerate() {
        let condition = if split { condition + 1 + index as i32 } else { condition };
        let (kind, fields) = settings(child);
        if kind == "arkTreeNodeSettings_Action_GameflowTask" {
            tasks.push(Task {
                name: text(fields, "m_taskName"), description: text(fields, "m_taskDescription"),
                image: image(fields, "m_journalImage"), optional: boolean(fields, "m_optional", false),
                auto_activate: boolean(fields, "m_autoActivate", true), show_in_journal: boolean(fields, "m_showInJournal", true),
                show_when_canceled: boolean(fields, "m_showWhenCanceled", true), condition_index: condition,
                lethality: match atom(fields, "m_lethal") {
                    Some("UI_LETHALSTATUS_INVALID") => -1, Some("UI_LETHALSTATUS_LETHAL") => 1,
                    Some("UI_LETHALSTATUS_NON_LETHAL") => 2, Some("UI_LETHALSTATUS_MIXED") => 3, _ => 0,
                },
            });
        } else if kind != "arkTreeNodeSettings_Decorator_GameflowObjective" && kind != "arkTreeNodeSettings_Decorator_GameflowMission" {
            gather_tasks(child, tasks, condition);
        }
    }
}

fn gather(node: &Fields, mission: &mut Mission) {
    let (class, data) = settings(node);
    if class == "arkTreeNodeSettings_Decorator_GameflowObjective" {
        let mut objective = Objective {
            name: text(data, "m_objectiveName"), description: text(data, "m_objectiveDescription"),
            image: image(data, "m_journalImage"), optional: boolean(data, "m_optional", false),
            auto_activate: boolean(data, "m_autoActivate", true), show_in_journal: boolean(data, "m_showInJournal", true), tasks: Vec::new(),
        };
        gather_tasks(node, &mut objective.tasks, -1);
        mission.objectives.push(objective);
    }
    for child in children(node) { gather(child, mission); }
}

#[derive(Default)]
struct Acquisition { reads: u64, bytes: u64 }

fn build_mission(node: &Fields, data: &Fields, name: &str) -> Mission {
    let targets = scope(data, "m_targetsInfo").map(|list| list_items(list).into_iter().filter_map(|value| match value {
        Value::Scope(item) => Some(Target { name: text(item, "m_targetName"), image: text(item, "m_journalImage") }),
        Value::Atom(_) => None,
    }).collect()).unwrap_or_default();
    let mut mission = Mission { name: name.to_owned(), description: text(data, "m_description"), targets, objectives: Vec::new() };
    gather(node, &mut mission);
    mission
}

fn find_in_node(catalog: &Catalog, node: &Fields, wanted: &str, visited: &mut HashSet<String>, stats: &mut Acquisition) -> Result<Option<Mission>, String> {
    let (class, data) = settings(node);
    if class == "arkTreeNodeSettings_Decorator_GameflowMission" && atom(data, "m_missionName") == Some(wanted) {
        return Ok(Some(build_mission(node, data, wanted)));
    }
    if class == "arkTreeNodeSettings_TreeReference" {
        if let Some(reference) = atom(data, "m_subTree") {
            if let Some(mission) = find_in_tree(catalog, reference, wanted, visited, stats)? { return Ok(Some(mission)); }
        }
    }
    for child in children(node) {
        if let Some(mission) = find_in_node(catalog, child, wanted, visited, stats)? { return Ok(Some(mission)); }
    }
    Ok(None)
}

fn find_in_tree(catalog: &Catalog, name: &str, wanted: &str, visited: &mut HashSet<String>, stats: &mut Acquisition) -> Result<Option<Mission>, String> {
    let name = unquote(name);
    if !visited.insert(name.to_ascii_lowercase()) { return Ok(None); }
    if visited.len() > 256 { return Err("gameflow tree reference limit exceeded".into()); }
    let mut read = |path: &str| {
        let result = catalog.read(path).ok().flatten();
        stats.reads += 1;
        stats.bytes += result.as_ref().map_or(0, |bytes| bytes.len() as u64);
        result
    };
    let tree = read_edit(&mut read, "arktree", name)?;
    match scope(&tree, "root") {
        Some(root) => find_in_node(catalog, root, wanted, visited, stats),
        None => Err(format!("gameflow tree {name} has no root")),
    }
}

/// Resolve a mission identity through the installed manager and referenced
/// trees. Only the selected definition survives this lookup; source trees are
/// dropped as the search unwinds.
pub fn mission(catalog: &Catalog, manager_decl: &str, name: &str) -> Result<Option<Mission>, String> {
    let diagnostic = std::env::var_os("DH2_DEBUG_GAMEFLOW_RESIDENCY").is_some_and(|value| value == "1");
    let started = diagnostic.then(std::time::Instant::now);
    let mut stats = Acquisition::default();
    let mut read = |path: &str| {
        let result = catalog.read(path).ok().flatten();
        stats.reads += 1;
        stats.bytes += result.as_ref().map_or(0, |bytes| bytes.len() as u64);
        result
    };
    let manager = read_edit(&mut read, "gameflowmanager", manager_decl)?;
    let tree = atom(&manager, "m_gameFlowTree").ok_or("gameflow manager has no tree")?;
    let result = find_in_tree(catalog, tree, name, &mut HashSet::new(), &mut stats);
    if let Some(started) = started {
        eprintln!("gameflow acquisition: mission={name} reads={} source_bytes={} elapsed_ms={:.3}", stats.reads, stats.bytes, started.elapsed().as_secs_f64() * 1000.0);
    }
    result
}

/// Resolve a mission only if the installed root traversal reaches it through
/// the supported selector, parallel and tree-reference nodes. Unknown nodes
/// are not treated as transparent: their execution may depend on state.
/// This only establishes reachability, not that a queued request has run.
pub fn reachable_mission(catalog: &Catalog, manager_decl: &str, name: &str) -> Result<Option<Mission>, String> {
    fn tree(catalog: &Catalog, path: &str, wanted: &str, visited: &mut HashSet<String>, stats: &mut Acquisition) -> Result<Option<Mission>, String> {
        let path = unquote(path);
        if !visited.insert(path.to_ascii_lowercase()) { return Ok(None); }
        if visited.len() > 256 { return Err("gameflow tree reference limit exceeded".into()); }
        let mut read = |resource: &str| {
            let bytes = catalog.read(resource).ok().flatten();
            stats.reads += 1;
            stats.bytes += bytes.as_ref().map_or(0, |bytes| bytes.len() as u64);
            bytes
        };
        let fields = read_edit(&mut read, "arktree", path)?;
        let root = scope(&fields, "root").ok_or_else(|| format!("gameflow tree {path} has no root"))?;
        node(catalog, root, wanted, visited, stats)
    }
    fn node(catalog: &Catalog, fields: &Fields, wanted: &str, visited: &mut HashSet<String>, stats: &mut Acquisition) -> Result<Option<Mission>, String> {
        let (class, data) = settings(fields);
        match class {
            "arkTreeNodeSettings_Decorator_GameflowMission" => {
                if atom(data, "m_missionName") != Some(wanted) { return Ok(None); }
                Ok(Some(build_mission(fields, data, wanted)))
            }
            "arkTreeNodeSettings_TreeReference" => match atom(data, "m_subTree") {
                Some(path) => tree(catalog, path, wanted, visited, stats), None => Ok(None),
            },
            "arkTreeNodeSettings_Parallel" | "arkTreeNodeSettings_Parallel_GameflowOr" => {
                for child in children(fields) {
                    if let Some(mission) = node(catalog, child, wanted, visited, stats)? { return Ok(Some(mission)); }
                }
                Ok(None)
            }
            "arkTreeNodeSettings_Selector" => {
                let mut iter = children(fields);
                let Some(only) = iter.next() else { return Ok(None) };
                if iter.next().is_some() { return Ok(None); }
                node(catalog, only, wanted, visited, stats)
            }
            _ => Ok(None),
        }
    }
    let started = std::env::var_os("DH2_DEBUG_GAMEFLOW_RESIDENCY").is_some_and(|value| value == "1").then(std::time::Instant::now);
    let mut stats = Acquisition::default();
    let mut read = |resource: &str| {
        let bytes = catalog.read(resource).ok().flatten();
        stats.reads += 1;
        stats.bytes += bytes.as_ref().map_or(0, |bytes| bytes.len() as u64);
        bytes
    };
    let manager = read_edit(&mut read, "gameflowmanager", manager_decl)?;
    let root = atom(&manager, "m_gameFlowTree").ok_or("gameflow manager has no tree")?;
    let result = tree(catalog, root, name, &mut HashSet::new(), &mut stats);
    if let Some(started) = started {
        eprintln!("gameflow reachable acquisition: mission={name} reads={} source_bytes={} elapsed_ms={:.3}", stats.reads, stats.bytes, started.elapsed().as_secs_f64() * 1000.0);
    }
    result
}

/// Live state is session-owned and changes only when gameflow events are
/// delivered. The map's loading hint is deliberately not used as progress.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
#[derive(Clone, Debug, Default)]
pub struct State {
    manager: Option<Arc<str>>,
    mission: Option<Arc<Mission>>,
    nodes: Arc<HashMap<String, i32>>,
    notifications: Arc<HashMap<String, i32>>,
    marked_tasks: Arc<HashSet<u32>>,
    show_history: bool,
}

impl State {
    /// 140724180 always displays history for the final mission.
    pub fn show_history(&self) -> bool {
        self.show_history || self.mission.as_ref().is_some_and(|mission| mission.name == "Mission_DTTE")
    }
    pub fn set_show_history(&mut self, show: bool) { self.show_history = show; }
    pub fn mission(&self) -> Option<&Mission> { self.mission.as_deref() }
    pub fn status(&self, name: &str) -> Option<i32> { self.nodes.get(name).copied() }
    /// Acquire only the selected immutable mission definition. Replacing it
    /// releases the previous definition after outstanding UI snapshots finish.
    pub fn select_mission(&mut self, catalog: &Catalog, manager_decl: &str, name: Option<&str>) -> Result<(), String> {
        if self.manager.as_deref() == Some(manager_decl) && self.mission.as_ref().map(|mission| mission.name.as_str()) == name { return Ok(()); }
        let next = name.map(|name| mission(catalog, manager_decl, name).and_then(|value|
            value.ok_or_else(|| format!("gameflow mission {name} is missing")))).transpose()?;
        self.mission = next.map(Arc::new);
        self.manager = Some(Arc::from(manager_decl));
        Ok(())
    }
    /// 1404ea190 publishes the native node status (2..5 are journal-visible).
    pub fn set_status(&mut self, name: String, status: i32) { Arc::make_mut(&mut self.nodes).insert(name, status); }
    pub fn set_notification_status(&mut self, name: String, status: i32) {
        Arc::make_mut(&mut self.notifications).insert(name, status);
    }
    pub fn set_task_marked(&mut self, id: u32, marked: bool) {
        let tasks = Arc::make_mut(&mut self.marked_tasks);
        if marked { tasks.insert(id); } else { tasks.remove(&id); }
    }
    pub fn task_marked(&self, name: &str) -> bool { self.marked_tasks.contains(&crate::assets::lang::key_id(name)) }
    pub fn journal_status(&self, name: &str) -> Option<i32> {
        match self.status(name)? { 2 => Some(0), 3 => Some(1), 4 => Some(2), 5 => Some(3), 6 => Some(-1), _ => None }
    }
    /// The journal sets both "past and current" and "ignore notification
    /// filter". Canceled tasks still require notification state 3.
    pub fn task_visible(&self, task: &Task) -> bool {
        if !task.show_in_journal { return false; }
        match self.status(&task.name) {
            Some(2..=4) | Some(6) => true,
            Some(5) => task.show_when_canceled && self.notifications.get(&task.name) == Some(&3),
            _ => false,
        }
    }
}
