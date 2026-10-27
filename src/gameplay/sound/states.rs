//! Global state groups (1410d7fc0 / 1410d87b0). Switch containers read the
//! current state immediately; property transitions use the authored duration.
use crate::assets::wwise::StateGroup;
use std::collections::HashMap;

#[derive(Default)]
pub struct States {
    groups: HashMap<u32, StateGroup>,
    values: HashMap<u32, u32>,
}

impl States {
    pub fn new(groups: impl IntoIterator<Item = StateGroup>) -> Self {
        Self { groups: groups.into_iter().map(|group| (group.id, group)).collect(), values: HashMap::new() }
    }

    /// New and absent groups return zero (1410d7fc0).
    pub fn current(&self, group: u32) -> u32 { self.values.get(&group).copied().unwrap_or(0) }

    /// Unknown groups are ignored. The current ID changes without waiting
    /// for the node-property fade to finish (1410d87b0's final store).
    pub fn transition_ms(&self, group: u32, value: u32) -> Option<u32> {
        let definition = self.groups.get(&group)?;
        let previous = self.current(group);
        if previous == value { return None; }
        let milliseconds = definition.transitions.iter().find(|transition| transition.from == previous && transition.to == value)
            .map_or(definition.milliseconds, |transition| transition.milliseconds);
        Some(milliseconds)
    }

    pub fn set(&mut self, group: u32, value: u32) -> Option<u32> {
        let milliseconds = self.transition_ms(group, value)?;
        self.values.insert(group, value);
        Some(milliseconds)
    }
}
