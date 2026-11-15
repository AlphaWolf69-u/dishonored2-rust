//! Per-instance hierarchy traversal; definitions and encoded media stay shared.
use crate::assets::wwise::{MusicGraph, MusicKind, MusicRule};
use super::{decision, playlist::Cursor, timing::Meter};
fn group_value(value: u32) -> u32 {
    // 14115def0: an uninitialized group subscribes as Wwise's None ID.
    if value == 0 { 0x2ca33bdb } else { value }
}

pub struct Branch { pub actor: u32, state: State }
enum State {
    Leaf { consumed: bool },
    Playlist { cursor: Cursor, child: Option<Box<Branch>> },
    Switch { selected: u32, values: Vec<u32>, child: Option<Box<Branch>> },
}
impl Branch {
    pub fn jump(&mut self, graph: &MusicGraph, id: u32, random15: &mut impl FnMut() -> u16) -> bool {
        match &mut self.state {
            State::Playlist { cursor, child } => {
                let Some(actor) = graph.actors.get(&self.actor) else { return false; };
                let MusicKind::Playlist { playlist, .. } = &actor.kind else { return false; };
                if cursor.jump(self.actor, playlist, id, &graph.history, &mut *random15) {
                    *child = None;
                    true
                } else { child.as_mut().is_some_and(|child| child.jump(graph, id, random15)) }
            },
            State::Switch { child, .. } => child.as_mut().is_some_and(|child| child.jump(graph, id, random15)),
            State::Leaf { .. } => false,
        }
    }
    pub fn new(actor: u32, graph: &MusicGraph, value: &mut impl FnMut(u32, u8) -> u32, random15: &mut impl FnMut() -> u16) -> Option<Self> {
        let state = match &graph.actors.get(&actor)?.kind {
            MusicKind::Track { .. } | MusicKind::Segment { .. } => State::Leaf { consumed: false },
            MusicKind::Playlist { playlist, .. } => State::Playlist { cursor: Cursor::new(playlist, random15), child: None },
            MusicKind::Switch { groups, tree, mode, .. } => {
                let values = groups.iter().map(|&(group, kind)| group_value(value(group, kind))).collect::<Vec<_>>();
                let selected = decision::select(tree, *mode, &values, &mut *random15).unwrap_or(0);
                let child = (selected != 0).then(|| Self::new(selected, graph, value, random15).map(Box::new)).flatten();
                State::Switch { selected, values, child }
            },
        };
        Some(Self { actor, state })
    }
    /// Each call consumes one segment selection, never a source's PCM. An
    /// exhausted playlist exits through its actual outer hierarchy.
    pub fn next(&mut self, graph: &MusicGraph, value: &mut impl FnMut(u32, u8) -> u32, random15: &mut impl FnMut() -> u16) -> Option<u32> {
        match &mut self.state {
            State::Leaf { consumed } => { if *consumed { None } else { *consumed = true; Some(self.actor) } },
            State::Switch { child, .. } => child.as_mut()?.next(graph, value, random15),
            State::Playlist { cursor, child } => loop {
                if let Some(segment) = child.as_mut().and_then(|child| child.next(graph, value, random15)) { return Some(segment); }
                let MusicKind::Playlist { playlist, .. } = &graph.actors.get(&self.actor)?.kind else { return None; };
                let (selected, _) = cursor.next(self.actor, playlist, &graph.history, &mut *random15)?;
                *child = Self::new(selected, graph, value, random15).map(Box::new);
                if child.is_none() { return None; }
            },
        }
    }
    /// 141157c90 / 14115e910: only subscribed group changes re-evaluate the
    /// decision path. ContinuePlayback preserves an unchanged selected child.
    pub fn refresh(&mut self, graph: &MusicGraph, value: &mut impl FnMut(u32, u8) -> u32,
        random15: &mut impl FnMut() -> u16) -> Option<u32> {
        match &mut self.state {
            State::Switch { selected, values, child } => {
                let MusicKind::Switch { groups, tree, mode, continue_playback, .. } = &graph.actors.get(&self.actor)?.kind else { return None; };
                let mut changed = false;
                for ((group, kind), previous) in groups.iter().zip(values.iter_mut()) {
                    let current = group_value(value(*group, *kind));
                    changed |= current != *previous;
                    *previous = current;
                }
                if changed {
                    let next = decision::select(tree, *mode, values, &mut *random15).unwrap_or(0);
                    if next != *selected || !continue_playback {
                        *selected = next;
                        *child = (next != 0).then(|| Self::new(next, graph, value, random15).map(Box::new)).flatten();
                        return Some(self.actor);
                    }
                }
                child.as_mut().and_then(|child| child.refresh(graph, value, random15))
            },
            State::Playlist { child, .. } => child.as_mut().and_then(|child| child.refresh(graph, value, random15)),
            State::Leaf { .. } => None,
        }
    }
    pub fn waiting(&self) -> bool {
        match &self.state {
            State::Switch { child: None, .. } => true,
            State::Switch { child: Some(child), .. } | State::Playlist { child: Some(child), .. } => child.waiting(),
            _ => false,
        }
    }
}

pub fn meter(graph: &MusicGraph, actor: u32) -> Meter {
    let Some(node) = graph.actors.get(&actor) else { return Meter::default(); };
    for properties in node.properties.nodes.iter() {
        if let Some(meter) = graph.meters.get(&properties.id) { return *meter; }
    }
    Meter::default()
}
/// Reverse-priority transition lookup (141150e00), matching actual ancestor
/// paths as 141150f30 does for nested music contexts. Rule zero is fallback.
pub fn rule<'a>(graph: &'a MusicGraph, owner: u32, source: u32, destination: u32) -> Option<&'a MusicRule> {
    let actor = graph.actors.get(&owner)?;
    let rules = match &actor.kind { MusicKind::Switch { rules, .. } | MusicKind::Playlist { rules, .. } => rules, _ => return None };
    let matches = |ids: &[u32], target| ids.first() == Some(&u32::MAX) || ids.contains(&target)
        || graph.actors.get(&target).is_some_and(|actor| actor.properties.nodes.iter().any(|node| ids.contains(&node.id)));
    rules.iter().rev().find(|rule| matches(&rule.sources, source) && matches(&rule.destinations, destination)).or_else(|| rules.first())
}
