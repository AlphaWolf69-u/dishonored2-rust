//! The world-selected game-logic manager and its authored transition rules.
//! 140787770 builds the matrix, 1407886c0 checks a pending switch,
//! 1407885b0 queues it, and 140787510 commits it against the stack.
use crate::{archive::Catalog, entities::{Document, Value}};
use crate::entities::event_graph::{atom, child};

const COUNT: usize = 17;
const ANY: usize = 18;
const NAMES: [&str; COUNT] = [
    "NONE", "INGAME", "PAUSE", "NOTIFICATION", "READNOTE", "CINEMATIC", "MAINMENU",
    "JOURNAL", "POWERWHEEL", "CONVERSATION", "BLACKMARKET", "RESULTS", "TUTORIAL",
    "GAMEOVER", "CHARACTER_CHOICE", "MAP_TRANSITION", "DEMO",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StateId(pub u8);
impl StateId {
    pub const NONE: Self = Self(0);
    pub const INGAME: Self = Self(1);
    pub const PAUSE: Self = Self(2);
    pub const READ_NOTE: Self = Self(4);
    pub const JOURNAL: Self = Self(7);
    pub const RESULTS: Self = Self(11);
    pub const GAME_OVER: Self = Self(13);
    pub const MAP_TRANSITION: Self = Self(15);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rule {
    pub allow: bool,
    pub quit_all: bool,
    pub quit_current: bool,
    pub keep_current_flash: bool,
}

#[derive(Clone, Debug)]
pub struct Declaration {
    pub path: String,
    pub states: [Option<String>; COUNT],
    pub initial: StateId,
    rules: [[Rule; COUNT]; COUNT],
}

fn state_id(name: &str) -> Result<usize, String> {
    let name = name.strip_prefix("ARK_GAME_LOGIC_STATE_").ok_or_else(|| format!("invalid game-logic state {name}"))?;
    if name == "ANY" { return Ok(ANY); }
    NAMES.iter().position(|candidate| *candidate == name).ok_or_else(|| format!("unknown game-logic state {name}"))
}
fn flag(fields: &[(String, Value)], name: &str, default: bool) -> Result<bool, String> {
    match atom(fields, name) {
        None => Ok(default), Some("true") => Ok(true), Some("false") => Ok(false),
        Some(value) => Err(format!("invalid game-logic {name}: {value}")),
    }
}

impl Declaration {
    /// Resolve exactly the declaration selected by this map's worldspawn.
    /// Missing or unresolved inheritance is an error, never a global guess.
    pub fn load(catalog: &Catalog, document: &Document) -> Result<Self, String> {
        let mut worlds = document.definitions.iter().filter(|entry|
            entry.kind == "entity" && atom(&entry.fields, "class") == Some("idWorldspawn"));
        let world = worlds.next().ok_or("worldspawn is absent")?;
        if worlds.next().is_some() { return Err("multiple worldspawn entities".into()); }
        let edit = child(&world.fields, "edit");
        let path = atom(edit, "m_gameLogicDecl").filter(|name| !name.is_empty() && *name != "NULL")
            .ok_or("worldspawn game-logic declaration is absent or requires inheritance")?.to_owned();
        Self::load_selected(catalog, &path)
    }

    /// Load the worldspawn owner's already-resolved manager reference.
    pub fn load_selected(catalog: &Catalog, path: &str) -> Result<Self, String> {
        let fields = crate::entities::player::resolved_decl("gamelogicmanager", path, |resource| {
            catalog.read(resource).map_err(|error| error.to_string()).map(|bytes|
                bytes.map(|bytes| crate::entities::declaration_text(&bytes).into_owned()))
        })?;
        Self::from_fields(path.to_owned(), &fields)
    }

    fn from_fields(path: String, fields: &[(String, Value)]) -> Result<Self, String> {
        let mut states: [Option<String>; COUNT] = std::array::from_fn(|_| None);
        for (name, value) in child(fields, "m_states") {
            let Some(name) = name.strip_prefix("enumItem[").and_then(|name| name.strip_suffix(']')) else { continue };
            let id = state_id(name)?;
            if id >= COUNT { return Err(format!("invalid concrete game-logic state {name}")); }
            let Value::Atom(value) = value else { return Err(format!("invalid game-logic state declaration {name}")); };
            let value = value.trim_matches('"');
            if !value.is_empty() && value != "NULL" { states[id] = Some(value.to_owned()); }
        }
        let initial = state_id(atom(fields, "m_initialState").ok_or("game-logic initial state is absent")?)?;
        if initial == 0 || initial >= COUNT || states[initial].is_none() { return Err("game-logic initial state is not declared".into()); }
        let mut rules = [[Rule::default(); COUNT]; COUNT];
        let mut ordered = Vec::new();
        for (name, value) in child(fields, "m_rules") {
            let Some(index) = name.strip_prefix("item[").and_then(|name| name.strip_suffix(']')) else { continue };
            let index = index.parse::<usize>().map_err(|_| format!("invalid game-logic rule index {name}"))?;
            let Value::Scope(fields) = value else { return Err(format!("invalid game-logic rule {name}")); };
            ordered.push((index, fields.as_slice()));
        }
        ordered.sort_by_key(|(index, _)| *index);
        for (_, fields) in ordered {
            let from = atom(fields, "m_from").map(state_id).transpose()?.unwrap_or(0);
            let to = atom(fields, "m_to").map(state_id).transpose()?.unwrap_or(0);
            // 1407873a0 initializes each rule to allow=1 and other flags 0.
            let rule = Rule { allow: flag(fields, "m_allow", true)?,
                quit_all: flag(fields, "m_quitAll", false)?,
                quit_current: flag(fields, "m_quitCurrent", false)?,
                keep_current_flash: flag(fields, "m_keepCurrentFlash", false)? };
            if from == ANY && to > 0 && to < COUNT {
                for source in 0..COUNT { if source != to { rules[source][to] = rule; } }
            } else if to == ANY && from > 0 && from < COUNT {
                for target in 0..COUNT { if target != from { rules[from][target] = rule; } }
            } else if from < COUNT && to < COUNT {
                if states[from].is_some() && states[to].is_some() { rules[from][to] = rule; }
            } else if !((from == ANY && to == 0) || (from == 0 && to == ANY)) {
                return Err(format!("unsupported game-logic rule {from} -> {to}"));
            }
        }
        Ok(Self { path, states, initial: StateId(initial as u8), rules })
    }
    pub fn rule(&self, from: StateId, to: StateId) -> Option<Rule> {
        self.rules.get(from.0 as usize).and_then(|row| row.get(to.0 as usize)).copied()
    }
}

/// An outcome for the one host that owns game-logic state implementations.
/// The host must execute the returned lifecycle operations in order.
#[derive(Debug)]
pub struct Transition {
    pub exited: Vec<StateId>,
    /// Quit-current first pops and may resume the state underneath before
    /// the new state is pushed; the host applies that lifecycle in order.
    pub resumed_after_pop: Option<(StateId, bool)>,
    pub suspended: Option<StateId>,
    pub entered: StateId,
    pub keep_current_flash: bool,
}

/// 787f50 exits the active stack first; 788520 then destroys each state
/// owner in ascending state-ID order. A host may wait for its movie worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShutdownOp { Exit(StateId), Destroy(StateId) }

#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub struct GameLogic {
    pub declaration: Declaration,
    registered: [bool; COUNT],
    active: [bool; COUNT],
    stack: Vec<StateId>,
    pending: Option<StateId>,
    pending_finishes: Vec<StateId>,
    shutdown_cursor: Option<usize>,
}
impl GameLogic {
    pub fn new(declaration: Declaration) -> Self {
        Self { declaration, registered: [false; COUNT], active: [false; COUNT], stack: Vec::new(), pending: None, pending_finishes: Vec::new(), shutdown_cursor: None }
    }
    /// 140788520 exits the stack and destroys every registered state owner.
    /// The current preparation path constructs the manager before any of
    /// those owners exist. A populated manager needs their real callbacks
    /// before the exclusive world shutdown may remove this resource.
    pub fn validate_shutdown(&self) -> Result<(), String> {
        if self.registered.iter().any(|registered| *registered)
            || !self.stack.is_empty()
            || self.pending.is_some()
            || !self.pending_finishes.is_empty()
        {
            return Err("game-logic state shutdown callbacks are not connected".into());
        }
        Ok(())
    }
    pub fn validate_shutdown_host(&self, mut supports: impl FnMut(StateId) -> bool) -> Result<(), String> {
        for (index, registered) in self.registered.iter().copied().enumerate() {
            if registered && !supports(StateId(index as u8)) {
                return Err(format!("game-logic state {index} has no shutdown host"));
            }
        }
        Ok(())
    }
    /// Resume the original ordered state shutdown. `false` keeps the current
    /// callback and all remaining owners alive while its real host is pending.
    pub fn shutdown(&mut self, mut host: impl FnMut(ShutdownOp) -> Result<bool, String>) -> Result<bool, String> {
        if self.shutdown_cursor.is_none() { self.shutdown_cursor = Some(0); }
        while let Some(&state) = self.stack.last() {
            if self.active[state.0 as usize] {
                if !host(ShutdownOp::Exit(state))? { return Ok(false); }
                self.active[state.0 as usize] = false;
            }
            self.stack.pop();
        }
        let mut cursor = self.shutdown_cursor.expect("shutdown started");
        while cursor < COUNT {
            if self.registered[cursor] {
                if !host(ShutdownOp::Destroy(StateId(cursor as u8)))? {
                    self.shutdown_cursor = Some(cursor);
                    return Ok(false);
                }
                self.registered[cursor] = false;
            }
            cursor += 1;
            self.shutdown_cursor = Some(cursor);
        }
        self.pending = None;
        self.pending_finishes.clear();
        Ok(true)
    }
    /// Only the real state owner can register an implementation; a declared
    /// resource path alone does not prove that a runtime state exists.
    pub fn register(&mut self, id: StateId) -> Result<(), String> {
        let index = id.0 as usize;
        if index == 0 || index >= COUNT || self.declaration.states[index].is_none() {
            return Err(format!("game-logic state {} is not declared", id.0));
        }
        self.registered[index] = true;
        Ok(())
    }
    pub fn current(&self) -> Option<StateId> { self.stack.last().copied() }
    pub fn is_active(&self, id: StateId) -> bool { self.active.get(id.0 as usize).copied().unwrap_or(false) }
    pub fn is_shutting_down(&self) -> bool { self.shutdown_cursor.is_some() }
    pub fn pending(&self) -> Option<StateId> { self.pending }
    /// 1407879f0 enters map transition before ordinary entity loading. State
    /// implementations must already be registered by their actual host.
    pub fn begin_map_transition(&mut self) -> Result<Transition, String> {
        if !self.request(StateId::MAP_TRANSITION) {
            return Err("map-transition game-logic state is unavailable".into());
        }
        self.commit()?.ok_or_else(|| "map-transition request was not committed".into())
    }
    /// 140787960 is called by GameLocal's map-entry handoff. The caller must
    /// run the returned INGAME transition now and process an optional queued
    /// non-INGAME initial state in the normal manager update.
    pub fn enter_initial_state(&mut self) -> Result<Option<Transition>, String> {
        let target = self.declaration.initial;
        let needs_ingame = if target.0 <= StateId::INGAME.0 {
            true
        } else {
            let rule = self.declaration.rule(StateId::INGAME, target)
                .ok_or("invalid initial game-logic transition")?;
            !rule.allow || (!rule.quit_all && !rule.quit_current)
        };
        let entered = if needs_ingame {
            if !self.request(StateId::INGAME) {
                return Err("in-game game-logic state is unavailable".into());
            }
            self.commit()?
        } else { None };
        if target.0 > StateId::INGAME.0 && !self.request(target) {
            return Err(format!("initial game-logic state {} is unavailable", target.0));
        }
        Ok(entered)
    }
    /// 1407886c0 checks the target and any pending request, not the stack top.
    pub(crate) fn registered(&self, target: StateId) -> bool {
        self.registered.get(target.0 as usize).copied().unwrap_or(false)
    }
    pub fn available(&self, target: StateId) -> bool {
        let index = target.0 as usize;
        self.shutdown_cursor.is_none() && index > 0 && index < COUNT && self.registered[index]
            && self.pending.is_none_or(|pending| self.declaration.rule(pending, target).is_some_and(|rule| rule.allow))
    }
    /// 1407885b0 queues a switch; commit is a separate manager update.
    pub fn request(&mut self, target: StateId) -> bool {
        if !self.available(target) { return false; }
        self.pending = Some(target);
        true
    }
    pub fn commit(&mut self) -> Result<Option<Transition>, String> {
        let Some(target) = self.pending.take() else { return Ok(None) };
        let current = self.current().unwrap_or(StateId::NONE);
        if current == target { return Err(format!("game-logic state {} is already current", target.0)); }
        let rule = self.declaration.rule(current, target).ok_or("invalid game-logic state")?;
        if !rule.allow { return Err(format!("game-logic transition {} -> {} denied", current.0, target.0)); }
        let mut exited = Vec::new();
        let mut resumed_after_pop = None;
        if rule.quit_all {
            while let Some(state) = self.stack.pop() {
                if std::mem::take(&mut self.active[state.0 as usize]) { exited.push(state); }
            }
        } else if rule.quit_current {
            if let Some(state) = self.stack.pop() {
                if std::mem::take(&mut self.active[state.0 as usize]) { exited.push(state); }
                if let Some(previous) = self.current() {
                    let kept = self.declaration.rule(previous, state).is_some_and(|rule| rule.keep_current_flash);
                    self.active[previous.0 as usize] = true;
                    resumed_after_pop = Some((previous, kept));
                }
            }
        }
        let suspended = (!rule.keep_current_flash).then(|| self.current()).flatten();
        if let Some(state) = suspended { self.active[state.0 as usize] = false; }
        self.stack.push(target);
        self.active[target.0 as usize] = true;
        Ok(Some(Transition { exited, resumed_after_pop, suspended, entered: target, keep_current_flash: rule.keep_current_flash }))
    }
    /// 140787510 consumes a pending pop before a queued switch. Return both
    /// outcomes even if the switch is denied, so the host still applies the
    /// already-completed pop lifecycle.
    pub fn update(&mut self) -> (Option<(StateId, Option<(StateId, bool)>)>, Result<Option<Transition>, String>) {
        let finished = self.commit_finish();
        let switched = self.commit();
        (finished, switched)
    }

    /// 1407881a0 queues the current state's completion once; the manager
    /// consumes it later in its update rather than mutating the stack inline.
    pub fn request_finish_current(&mut self) -> bool {
        let Some(current) = self.current() else { return false };
        if !self.pending_finishes.contains(&current) { self.pending_finishes.push(current); }
        true
    }
    /// 140787bf0 pops only when the current state has requested completion.
    pub fn commit_finish(&mut self) -> Option<(StateId, Option<(StateId, bool)>)> {
        let current = self.current()?;
        let index = self.pending_finishes.iter().position(|state| *state == current)?;
        self.pending_finishes.remove(index);
        self.stack.pop();
        self.active[current.0 as usize] = false;
        let resumed = self.current().map(|previous| {
            let kept = self.declaration.rule(previous, current).is_some_and(|rule| rule.keep_current_flash);
            self.active[previous.0 as usize] = true;
            (previous, kept)
        });
        Some((current, resumed))
    }
}
