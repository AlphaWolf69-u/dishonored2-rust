//! Authored gameflow mission definitions, loaded from the manager's tree.
//!
//! Definitions are separate from live progression: a map name or an authored
//! `m_autoActivate` flag alone does not establish that a mission is active.
pub mod program;
use crate::assets::{archive::Catalog, entities::Value};
use crate::gameplay::options::decl::{atom, boolean, list_items, scope};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Target {
    pub name: String,
    pub image: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkerTarget {
    pub name: u32,
    pub entity: String,
}

/// 40f670/66eff0: authored target notification, independent of task text.
#[derive(Clone, Debug, Default)]
pub struct Portrait {
    pub show: bool,
    pub state: String,
    pub kind: String,
    pub female: bool,
    pub image: Option<String>,
    pub name: u32,
}
impl Portrait {
    pub fn from_fields(fields: &Fields) -> Self {
        Self {
            show: boolean(fields, "m_showPortrait", false),
            state: atom(fields, "m_portraitStatus")
                .unwrap_or("GAMEFLOW_UI_PORTRAIT_STATE_ASSASSINATED")
                .to_owned(),
            kind: atom(fields, "m_portraitType")
                .unwrap_or("GAMEFLOW_UI_PORTRAIT_TYPE_TARGET")
                .to_owned(),
            female: boolean(fields, "m_portraitFemale", false),
            image: image(fields, "m_portraitImage"),
            name: atom(fields, "m_portraitName")
                .filter(|name| !name.is_empty())
                .map_or(0, crate::assets::lang::key_id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Task {
    pub portrait: Portrait,
    pub markers: Vec<MarkerTarget>,
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub optional: bool,
    pub auto_activate: bool,
    pub show_notification: bool,
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
fn text(fields: &Fields, key: &str) -> String {
    atom(fields, key).unwrap_or("").to_owned()
}
fn image(fields: &Fields, key: &str) -> Option<String> {
    atom(fields, key)
        .filter(|value| !value.is_empty() && *value != "NULL")
        .map(str::to_owned)
}
fn children(fields: &Fields) -> impl Iterator<Item = &Fields> {
    scope(fields, "m_children")
        .into_iter()
        .flat_map(list_items)
        .filter_map(|value| match value {
            Value::Scope(fields) => Some(fields.as_slice()),
            Value::Atom(_) => None,
        })
}
fn settings(node: &Fields) -> (&str, &Fields) {
    let Some(settings) = scope(node, "m_settings") else {
        return ("", &[]);
    };
    (
        atom(settings, "m_class").unwrap_or(""),
        scope(settings, "m_object").unwrap_or(&[]),
    )
}

/// 1404e9010 visits ordered children of the objective subtree, skipping
/// nested objective/mission decorators. Parallel nodes configured to return
/// on first success (but not first failure) assign distinct child conditions.
fn gather_tasks(node: &Fields, tasks: &mut Vec<Task>, condition: i32) {
    let (class, data) = settings(node);
    let split = matches!(
        class,
        "arkTreeNodeSettings_Parallel" | "arkTreeNodeSettings_Parallel_GameflowOr"
    ) && boolean(data, "m_returnOnFirstSuccess", false)
        && !boolean(data, "m_returnOnFirstFail", false);
    for (index, child) in children(node).enumerate() {
        let condition = if split {
            condition + 1 + index as i32
        } else {
            condition
        };
        let (kind, fields) = settings(child);
        if kind == "arkTreeNodeSettings_Action_GameflowTask" {
            tasks.push(Task {
                portrait: Portrait::from_fields(fields),
                markers: scope(fields, "m_targetsToMark")
                    .into_iter()
                    .flat_map(list_items)
                    .filter_map(|value| match value {
                        Value::Scope(fields) => Some(MarkerTarget {
                            name: crate::assets::lang::key_id(atom(fields, "m_name").unwrap_or("")),
                            entity: text(fields, "m_entity"),
                        }),
                        _ => None,
                    })
                    .collect(),
                name: text(fields, "m_taskName"),
                description: text(fields, "m_taskDescription"),
                image: image(fields, "m_journalImage"),
                optional: boolean(fields, "m_optional", false),
                auto_activate: boolean(fields, "m_autoActivate", true),
                show_notification: boolean(fields, "m_showNotification", true),
                show_in_journal: boolean(fields, "m_showInJournal", true),
                show_when_canceled: boolean(fields, "m_showWhenCanceled", true),
                condition_index: condition,
                lethality: match atom(fields, "m_lethal") {
                    Some("UI_LETHALSTATUS_INVALID") => -1,
                    Some("UI_LETHALSTATUS_LETHAL") => 1,
                    Some("UI_LETHALSTATUS_NON_LETHAL") => 2,
                    Some("UI_LETHALSTATUS_MIXED") => 3,
                    _ => 0,
                },
            });
        } else if kind != "arkTreeNodeSettings_Decorator_GameflowObjective"
            && kind != "arkTreeNodeSettings_Decorator_GameflowMission"
        {
            gather_tasks(child, tasks, condition);
        }
    }
}

fn gather(node: &Fields, mission: &mut Mission) {
    let (class, data) = settings(node);
    if class == "arkTreeNodeSettings_Decorator_GameflowObjective" {
        let mut objective = Objective {
            name: text(data, "m_objectiveName"),
            description: text(data, "m_objectiveDescription"),
            image: image(data, "m_journalImage"),
            optional: boolean(data, "m_optional", false),
            auto_activate: boolean(data, "m_autoActivate", true),
            show_in_journal: boolean(data, "m_showInJournal", true),
            tasks: Vec::new(),
        };
        gather_tasks(node, &mut objective.tasks, -1);
        mission.objectives.push(objective);
    }
    for child in children(node) {
        gather(child, mission);
    }
}

fn build_mission(node: &Fields, data: &Fields, name: &str) -> Mission {
    let targets = scope(data, "m_targetsInfo")
        .map(|list| {
            list_items(list)
                .into_iter()
                .filter_map(|value| match value {
                    Value::Scope(item) => Some(Target {
                        name: text(item, "m_targetName"),
                        image: text(item, "m_journalImage"),
                    }),
                    Value::Atom(_) => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut mission = Mission {
        name: name.to_owned(),
        description: text(data, "m_description"),
        targets,
        objectives: Vec::new(),
    };
    gather(node, &mut mission);
    mission
}

/// Explicit lookup shares the canonical program's decoded mission definition.
/// The compatibility value-returning API clones only for callers asking for it;
/// live state and Journal snapshots retain the shared immutable Arc instead.
pub fn mission(
    catalog: &Catalog,
    manager_decl: &str,
    name: &str,
) -> Result<Option<Mission>, String> {
    Ok(program::Program::load(catalog, manager_decl)?
        .mission(name)
        .map(|mission| (*mission).clone()))
}
/// Static reachability, never an activation request or a selected-map guess.
pub fn reachable_mission(
    catalog: &Catalog,
    manager_decl: &str,
    name: &str,
) -> Result<Option<Mission>, String> {
    Ok(program::Program::load(catalog, manager_decl)?
        .reachable_mission(name)
        .map(|mission| (*mission).clone()))
}

/// Live state is level-owned and changes only when gameflow events are
/// delivered. The map's loading hint is deliberately not used as progress.
#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
#[derive(Clone, Debug, Default)]
pub struct State {
    pub(super) portraits: Arc<HashMap<u32, Portrait>>,
    target_states: Arc<HashMap<u32, i32>>,
    manager: Option<Arc<str>>,
    mission: Option<Arc<Mission>>,
    nodes: Arc<HashMap<String, i32>>,
    notifications: Arc<HashMap<String, i32>>,
    unmarked_tasks: Arc<HashSet<u32>>,
    marker_overrides: Arc<HashMap<u32, Vec<MarkerTarget>>>,
    show_history: bool,
}

impl State {
    /// 140724180 always displays history for the final mission.
    pub fn show_history(&self) -> bool {
        self.show_history
            || self
                .mission
                .as_ref()
                .is_some_and(|mission| mission.name == "Mission_DTTE")
    }
    pub fn set_show_history(&mut self, show: bool) {
        self.show_history = show;
    }
    pub fn mission(&self) -> Option<&Mission> {
        self.mission.as_deref()
    }
    pub fn status(&self, name: &str) -> Option<i32> {
        self.nodes.get(name).copied()
    }
    /// Acquire only the selected immutable mission definition. Replacing it
    /// releases the previous definition after outstanding UI snapshots finish.
    pub fn select_mission(
        &mut self,
        catalog: &Catalog,
        manager_decl: &str,
        name: Option<&str>,
    ) -> Result<(), String> {
        if self.manager.as_deref() == Some(manager_decl)
            && self.mission.as_ref().map(|mission| mission.name.as_str()) == name
        {
            return Ok(());
        }
        let next = name
            .map(|name| {
                program::Program::load(catalog, manager_decl)?
                    .mission(name)
                    .ok_or_else(|| format!("gameflow mission {name} is missing"))
            })
            .transpose()?;
        self.mission = next;
        self.manager = Some(Arc::from(manager_decl));
        Ok(())
    }
    /// 1404ea190 publishes the native node status (2..5 are journal-visible).
    pub fn set_status(&mut self, name: String, status: i32) {
        Arc::make_mut(&mut self.nodes).insert(name, status);
    }
    pub fn set_notification_status(&mut self, name: String, status: i32) {
        Arc::make_mut(&mut self.notifications).insert(name, status);
    }
    pub fn set_task_marked(&mut self, id: u32, marked: bool) {
        let tasks = Arc::make_mut(&mut self.unmarked_tasks);
        if marked {
            tasks.remove(&id);
        } else {
            tasks.insert(id);
        }
    }
    pub fn marker_targets<'a>(&'a self, task: &'a Task) -> &'a [MarkerTarget] {
        self.marker_overrides
            .get(&crate::assets::lang::key_id(&task.name))
            .map(Vec::as_slice)
            .unwrap_or(&task.markers)
    }
    pub fn task_marked(&self, name: &str) -> bool {
        !self
            .unmarked_tasks
            .contains(&crate::assets::lang::key_id(name))
    }
    pub fn journal_status(&self, name: &str) -> Option<i32> {
        match self.status(name)? {
            2 => Some(0),
            3 => Some(1),
            4 => Some(2),
            5 => Some(3),
            6 => Some(-1),
            _ => None,
        }
    }
    /// The journal sets both "past and current" and "ignore notification
    /// filter". Canceled tasks still require notification state 3.
    pub fn task_visible(&self, task: &Task) -> bool {
        if !task.show_in_journal {
            return false;
        }
        match self.status(&task.name) {
            Some(2..=4) | Some(6) => true,
            Some(5) => task.show_when_canceled && self.notifications.get(&task.name) == Some(&3),
            _ => false,
        }
    }
}

/// Immutable HUD publications; labels are resolved by the UI worker's shared
/// localization table. These are queued only for actual task status changes.
#[derive(Clone, Debug)]
pub struct TaskUpdate {
    pub name: String,
    pub optional: bool,
    pub status: i32,
    pub lethality: i32,
    pub id: u32,
    pub condition: i32,
}
#[derive(Clone, Debug)]
pub struct ObjectiveUpdate {
    pub portrait: Option<Portrait>,
    pub name: String,
    pub optional: bool,
    pub status: i32,
    pub tasks: Vec<TaskUpdate>,
}
impl State {
    /// Mission target records start at -1 (40e970) and change when the HUD
    /// queues a target result (66f340), independently of its image residency.
    pub fn target_status(&self, name: &str) -> i32 {
        self.target_states
            .get(&crate::assets::lang::key_id(name))
            .copied()
            .unwrap_or(-1)
    }
    pub(super) fn notify_target(&mut self, portrait: &Portrait) {
        let state = match portrait.state.as_str() {
            "GAMEFLOW_UI_PORTRAIT_STATE_ASSASSINATED" => 0,
            "GAMEFLOW_UI_PORTRAIT_STATE_NEUTRALIZED" => 1,
            "GAMEFLOW_UI_PORTRAIT_STATE_RESCUED" => 2,
            _ => return,
        };
        Arc::make_mut(&mut self.target_states).insert(portrait.name, state);
    }
    pub fn mark_task_notified(&mut self, id: u32) {
        let name = self
            .mission()
            .into_iter()
            .flat_map(|mission| &mission.objectives)
            .flat_map(|objective| &objective.tasks)
            .find(|task| crate::assets::lang::key_id(&task.name) == id)
            .map(|task| task.name.clone());
        if let Some(name) = name {
            self.set_notification_status(name, 3);
        }
    }
    pub fn task_update(&self, name: &str, status: i32) -> Option<ObjectiveUpdate> {
        let mission = self.mission()?;
        let objective = mission
            .objectives
            .iter()
            .find(|objective| objective.tasks.iter().any(|task| task.name == name))?;
        let task = objective.tasks.iter().find(|task| task.name == name)?;
        if !task.show_notification
            || !(2..=5).contains(&status)
            || (status == 5 && self.notifications.get(name) != Some(&3))
        {
            return None;
        }
        let portrait = self
            .portraits
            .get(&crate::assets::lang::key_id(name))
            .unwrap_or(&task.portrait);
        Some(ObjectiveUpdate {
            portrait: (status == 3 && portrait.show).then(|| portrait.clone()),
            name: objective.name.clone(),
            optional: objective.optional,
            status: self.journal_status(&objective.name).unwrap_or(0),
            tasks: vec![TaskUpdate {
                name: task.name.clone(),
                optional: task.optional,
                status: status - 2,
                lethality: task.lethality,
                id: crate::assets::lang::key_id(&task.name),
                condition: task.condition_index,
            }],
        })
    }
}

impl State {
    pub fn diagnostic(&self) -> String {
        let mut nodes = self
            .nodes
            .iter()
            .map(|(name, status)| (name.as_str(), *status))
            .collect::<Vec<_>>();
        nodes.sort_unstable();
        format!(
            "GAMEFLOW_PROGRESSION mission={:?} nodes={nodes:?}",
            self.mission().map(|mission| mission.name.as_str())
        )
    }
}
