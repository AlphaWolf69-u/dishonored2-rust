//! Random/sequence container timing and sequence traversal.
use crate::assets::wwise::ContainerPlayback;
use crate::assets::wwise::SelectionMode;
use std::collections::{HashSet, VecDeque};

/// Playback-local cycle limit (`FUN_14111b7b0`). Step containers omit it.
#[derive(Clone, Debug)]
pub struct Loops {
    remaining: Option<u16>,
}

impl Loops {
    pub fn new(settings: &ContainerPlayback, mut random15: impl FnMut() -> u16) -> Self {
        let remaining = match settings.loop_count {
            0 => None,
            1 => Some(1),
            count => {
                // FUN_141118990 uses signed 16-bit modifiers and a rounded
                // MSVC rand() fraction, then the caller clamps to one cycle.
                let [min, max] = settings.loop_modifiers;
                let difference = max.wrapping_sub(min);
                let offset = if difference == 0 { 0 } else {
                    (f64::from(random15()) / 32767.0 * f64::from(difference) + 0.5) as i32 as i16
                };
                let sampled = (count as i16).wrapping_add(min.wrapping_add(offset));
                Some(sampled.max(1) as u16)
            },
        };
        Self { remaining }
    }

    /// Called after a complete traversal, before selecting its next item.
    pub fn next_cycle(&mut self) -> bool {
        match self.remaining.as_mut() {
            None => true,
            Some(remaining) => {
                *remaining = remaining.saturating_sub(1);
                *remaining != 0
            },
        }
    }
}

/// `FUN_14111b290`: authored milliseconds, additive randomized milliseconds,
/// and the transition-time RTPC (whose value is expressed in seconds).
pub fn transition_ms(settings: &ContainerPlayback, rtpc_seconds: f32, mut random15: impl FnMut() -> u16) -> f32 {
    let [min, max] = settings.transition_modifiers_ms;
    let difference = max - min;
    let offset = if difference == 0.0 { 0.0 } else {
        (f64::from(random15()) / 32767.0 * f64::from(difference)) as f32
    };
    let time = offset + min + settings.transition_ms + rtpc_seconds * 1000.0;
    if time < 0.0 { 0.0 } else { time }
}

/// Sequence state starts before item zero, moving forward (14113dc40).
#[derive(Clone, Debug)]
pub struct Sequence {
    cursor: i32,
    forward: bool,
}

impl Default for Sequence {
    fn default() -> Self { Self { cursor: -1, forward: true } }
}

impl Sequence {
    /// `FUN_14111ad10`: reverse playlists bounce at the endpoints without
    /// repeating them. A cycle finishes after returning to the first item.
    pub fn next(&mut self, length: usize, restart_backward: bool, loops: Option<&mut Loops>) -> Option<usize> {
        if length == 0 { return None; }
        let mut finished_cycle = false;
        if length == 1 {
            finished_cycle = self.cursor == 0;
            self.cursor = 0;
        } else if self.forward {
            if self.cursor + 1 == length as i32 {
                if restart_backward {
                    self.forward = false;
                    self.cursor -= 1;
                } else {
                    self.cursor = 0;
                    finished_cycle = true;
                }
            } else {
                self.cursor += 1;
            }
        } else if self.cursor == 0 {
            self.cursor = 1;
            self.forward = true;
            finished_cycle = true;
        } else {
            self.cursor -= 1;
        }
        if finished_cycle && loops.is_some_and(|loops| !loops.next_cycle()) {
            return None;
        }
        Some(self.cursor as usize)
    }
}

/// Selection state is shared for step containers and local to each continuous
/// playback. The cycle tracks every visited entry (14111aa00 / 141119b70).
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub sequence: Sequence,
    history: VecDeque<usize>,
    visited: HashSet<usize>,
}

impl Selection {
    pub fn next(&mut self, mode: SelectionMode, weights: &[u32], avoid_repeat: u16,
        restart_backward: bool, loops: Option<&mut Loops>) -> Option<usize>
    {
        if weights.is_empty() { return None; }
        if mode == SelectionMode::Sequence {
            return self.sequence.next(weights.len(), restart_backward, loops);
        }
        if self.visited.len() == weights.len() {
            if loops.is_some_and(|loops| !loops.next_cycle()) { return None; }
            self.visited.clear();
        }
        let avoid = usize::from(avoid_repeat).min(weights.len().saturating_sub(1));
        // Shuffle always retains at least the last entry across cycles.
        let avoid = if mode == SelectionMode::Shuffle { avoid.max(1).min(weights.len().saturating_sub(1)) } else { avoid };
        while self.history.len() > avoid { self.history.pop_front(); }
        let eligible = |index: usize| !self.history.contains(&index)
            && (mode != SelectionMode::Shuffle || !self.visited.contains(&index));
        let total = weights.iter().enumerate().filter(|(index, _)| eligible(*index)).map(|(_, weight)| u64::from(*weight)).sum::<u64>();
        if total == 0 { return None; }
        let mut pick = rand::random_range(0..total);
        let index = weights.iter().enumerate().position(|(index, weight)| {
            if !eligible(index) { return false; }
            if pick < u64::from(*weight) { true } else { pick -= u64::from(*weight); false }
        })?;
        self.visited.insert(index);
        self.history.push_back(index);
        while self.history.len() > avoid { self.history.pop_front(); }
        Some(index)
    }
}
