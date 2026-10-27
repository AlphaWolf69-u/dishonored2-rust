//! Switch history and retained Play contexts (141133190 / 141132f00).
//! Commands preserve callback order; the host applies them synchronously.
use crate::assets::wwise::{SwitchCase, SwitchPlayback};
use super::Emitter;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default)]
pub struct History { pub value: u32, pub count: u32 }

/// A retained Play context deliberately has no source-continuation stack
/// (141132370). State changes start new ordinary plays through 141133710.
#[derive(Clone)]
pub struct Controller<C> {
    pub node: u32,
    pub emitter: Emitter,
    pub playing_id: u32,
    pub paused: bool,
    pub context: C,
}

pub struct Initial {
    pub case: Option<u32>,
    /// Parallel handling depends on the authored case length, including
    /// children filtered by first-only or unavailable in the actor registry.
    pub authored_count: usize,
    pub children: Vec<u32>,
}

pub enum Command<C> {
    CancelPending { emitter: Emitter, child: u32 },
    Stop { emitter: Emitter, child: u32, milliseconds: i32, curve: u8 },
    Release { emitter: Emitter, child: u32, owner: u32 },
    ResetHistory { node: u32, emitter: Emitter },
    Play { controller: Controller<C>, child: u32, milliseconds: i32, curve: u8 },
    StoreHistory { node: u32, emitter: Emitter, history: History },
}

pub struct Change<C> { pub commands: Vec<Command<C>> }

pub fn case(settings: &SwitchPlayback, value: u32) -> Option<&SwitchCase> {
    settings.cases.iter().find(|case| case.value == value)
        .or_else(|| settings.cases.iter().find(|case| case.value == settings.default))
}

pub struct Switches<C> {
    history: HashMap<(u32, Emitter), History>,
    controllers: Vec<Controller<C>>,
}
impl<C> Default for Switches<C> {
    fn default() -> Self { Self { history: HashMap::new(), controllers: Vec::new() } }
}
impl<C> Switches<C> {
    pub fn history(&self, node: u32, emitter: Emitter) -> History {
        self.history.get(&(node, emitter)).copied().unwrap_or_default()
    }

    /// Register before selecting the initial case, even if no source starts.
    /// The caller uses this only for continuous ordinary Play contexts.
    pub fn register(&mut self, controller: Controller<C>) { self.controllers.push(controller); }

    /// 141133190 updates history before playing any child. An absent case
    /// leaves history unchanged; an authored empty case still updates it.
    pub fn initial(&mut self, node: u32, settings: &SwitchPlayback, emitter: Emitter, value: u32) -> Initial {
        let Some(case) = case(settings, value) else {
            return Initial { case: None, authored_count: 0, children: Vec::new() };
        };
        let previous = self.history(node, emitter);
        let count = if previous.value == case.value { previous.count.wrapping_add(1) } else { 1 };
        self.history.insert((node, emitter), History { value: case.value, count });
        let children = case.children.iter().copied().filter(|child| count == 1 ||
            !settings.parameters.iter().find(|controls| controls.node == *child).is_some_and(|controls| controls.first_only)).collect();
        Initial { case: Some(case.value), authored_count: case.children.len(), children }
    }

    /// 141132da0 snapshots every retained game-object entry, including
    /// duplicates. Do not deduplicate: an unmapped raw value differs from the
    /// stored fallback value, so later entries can cause another change.
    pub fn recipients(&mut self, node: u32) -> Vec<Emitter> {
        let recipients = self.controllers.iter().filter(|controller| controller.node == node)
            .map(|controller| controller.emitter).collect::<Vec<_>>();
        if recipients.is_empty() { self.history.retain(|&(owner, _), _| owner != node); }
        recipients
    }

    /// 141134190 removes contexts by game object and/or playing ID, then
    /// clears the object's shared history if at least one context was removed,
    /// even when other plays remain.
    pub fn remove(&mut self, node: u32, emitter: Option<Emitter>, playing_id: Option<u32>) {
        let count = self.controllers.len();
        self.controllers.retain(|controller| !matches(controller, node, emitter, playing_id));
        if self.controllers.len() != count {
            self.history.retain(|&(owner, object), _| owner != node || emitter.is_some_and(|emitter| emitter != object));
        }
    }

    pub fn pause(&mut self, node: u32, emitter: Option<Emitter>, playing_id: Option<u32>, paused: bool) {
        for controller in &mut self.controllers {
            if matches(controller, node, emitter, playing_id) { controller.paused = paused; }
        }
    }

    /// Apply history commands at their position in a Change, after old-child
    /// stops and around new-child plays, rather than before all callbacks.
    pub fn apply_history(&mut self, command: &Command<C>) {
        match command {
            Command::ResetHistory { node, emitter } => { self.history.remove(&(*node, *emitter)); },
            Command::StoreHistory { node, emitter, history } => { self.history.insert((*node, *emitter), *history); },
            _ => {},
        }
    }
}

fn matches<C>(controller: &Controller<C>, node: u32, emitter: Option<Emitter>, playing_id: Option<u32>) -> bool {
    controller.node == node && emitter.is_none_or(|emitter| emitter == controller.emitter)
        && playing_id.is_none_or(|playing_id| playing_id == controller.playing_id)
}

impl<C: Clone> Switches<C> {
    /// 141132f00 / 1411343b0 / 1411342e0. Comparison uses the raw incoming
    /// value; both previous children and repeat history use mapped case IDs.
    /// New plays have their own empty continuations and linear fade-ins.
    pub fn change(&self, node: u32, settings: &SwitchPlayback, emitter: Emitter, value: u32) -> Option<Change<C>> {
        let previous = self.history(node, emitter);
        if previous.value == value { return None; }
        let old = settings.cases.iter().find(|case| case.value == previous.value);
        let new = case(settings, value);
        let mut commands = Vec::new();
        if let Some(old) = old {
            for &child in &old.children {
                let controls = settings.parameters.iter().find(|controls| controls.node == child);
                if settings.continuous && controls.is_some_and(|controls| controls.continue_playback)
                    && new.is_some_and(|case| case.children.contains(&child)) { continue; }
                commands.push(Command::CancelPending { emitter, child });
                if controls.is_some_and(|controls| controls.on_switch == 1) {
                    commands.push(Command::Stop { emitter, child, milliseconds: controls.unwrap().fade_out_ms, curve: 4 });
                } else { commands.push(Command::Release { emitter, child, owner: node }); }
            }
        }
        commands.push(Command::ResetHistory { node, emitter });
        let mut history = History::default();
        let selected = new.map_or(settings.default, |case| case.value);
        for controller in self.controllers.iter().filter(|controller| controller.node == node && controller.emitter == emitter) {
            history.count = if history.value == selected { history.count.wrapping_add(1) } else { 1 };
            history.value = selected;
            if let Some(new) = new {
                for &child in &new.children {
                    let controls = settings.parameters.iter().find(|controls| controls.node == child);
                    if controls.is_some_and(|controls| controls.continue_playback)
                        && old.is_some_and(|case| case.children.contains(&child)) { continue; }
                    // First-only is intentionally absent here: 141133710
                    // receives every non-continuing child on a state change.
                    commands.push(Command::Play { controller: controller.clone(), child,
                        milliseconds: controls.map_or(0, |controls| controls.fade_in_ms), curve: 4 });
                }
            }
        }
        commands.push(Command::StoreHistory { node, emitter, history });
        Some(Change { commands })
    }
}
