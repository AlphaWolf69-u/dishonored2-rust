//! Global state groups (1410d7fc0 / 1410d87b0). Switch containers read the
//! current state immediately; property transitions use the authored duration.
use crate::assets::wwise::{NodeStates, StateGroup};
use hashbrown::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    Arc, Mutex, Weak,
};

#[derive(Default)]
struct Subscribers {
    nodes: Mutex<HashMap<u32, HashMap<usize, Weak<Node>>>>,
    tick: AtomicU64,
}

#[derive(Clone, Copy, Default)]
struct Lane {
    from: f32,
    to: f32,
    start: u64,
    duration: u64,
    transitioning: bool,
}
impl Lane {
    fn value(self, tick: u64, property: usize) -> f32 {
        if !self.transitioning {
            return self.to;
        }
        let db = matches!(property, 0 | 1 | 5);
        let age = tick.saturating_sub(self.start);
        let value = if age >= self.duration {
            self.to
        } else {
            let (from, to) = if db {
                (
                    super::mix::power10(self.from * 0.05),
                    super::mix::power10(self.to * 0.05),
                )
            } else {
                (self.from, self.to)
            };
            return Self::convert(
                super::mix::interpolate(age as f32 / self.duration as f32, from, to, 4),
                db,
            );
        };
        if db {
            Self::convert(super::mix::power10(value * 0.05), true)
        } else {
            value
        }
    }
    fn convert(value: f32, db: bool) -> f32 {
        if db {
            (super::mix::ln(value) * 0.4342945) * 20.
        } else {
            value
        }
    }
}

struct Group {
    id: u32,
    lanes: [Lane; 6],
}

/// The canonical node's source lease owns its subscription; the global state
/// manager retains only a weak callback reference, like141111300/141111630.
pub struct Node {
    id: u32,
    source: Weak<NodeStates>,
    owner: Weak<Subscribers>,
    groups: Mutex<Vec<Group>>,
    values: [AtomicU32; 6],
    transitioning: AtomicBool,
    last_tick: AtomicU64,
}
impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateNode").finish_non_exhaustive()
    }
}
impl Drop for Node {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.upgrade() {
            let key = self as *const Self as usize;
            let mut registered = owner.nodes.lock().unwrap_or_else(|e| e.into_inner());
            for group in self.groups.get_mut().unwrap_or_else(|e| e.into_inner()) {
                if let Some(nodes) = registered.get_mut(&group.id) {
                    nodes.remove(&key);
                    if nodes.is_empty() {
                        registered.remove(&group.id);
                    }
                }
            }
        }
    }
}
impl Node {
    pub fn sample(&self) -> [f32; 6] {
        if self.transitioning.load(Ordering::Relaxed) {
            let tick = self
                .owner
                .upgrade()
                .map_or(0, |owner| owner.tick.load(Ordering::Relaxed));
            if self.last_tick.load(Ordering::Relaxed) != tick {
                let groups = self.groups.lock().unwrap_or_else(|e| e.into_inner());
                if self.last_tick.load(Ordering::Relaxed) != tick {
                    self.publish(&groups, tick);
                }
            }
        }
        self.published()
    }
    fn published(&self) -> [f32; 6] {
        std::array::from_fn(|property| {
            f32::from_bits(self.values[property].load(Ordering::Relaxed))
        })
    }
    fn publish(&self, groups: &[Group], tick: u64) {
        let mut values = [0.; 6];
        for group in groups.iter() {
            for (property, value) in values.iter_mut().enumerate() {
                *value += group.lanes[property].value(tick, property);
            }
        }
        for (target, value) in self.values.iter().zip(values) {
            target.store(value.to_bits(), Ordering::Relaxed);
        }
        self.transitioning.store(
            groups.iter().any(|group| {
                group.lanes.iter().any(|lane| {
                    lane.transitioning && tick.saturating_sub(lane.start) < lane.duration
                })
            }),
            Ordering::Relaxed,
        );
        self.last_tick.store(tick, Ordering::Relaxed);
    }
    fn transition(&self, group: u32, state: u32, tick: u64, duration: u64) {
        let Some(source) = self.source.upgrade() else {
            return;
        };
        let Some(binding) = source.groups.iter().find(|binding| binding.group == group) else {
            return;
        };
        let mut target = [0.; 6];
        if let Some(value) = binding.values.iter().find(|value| value.value == state) {
            for &(property, bits) in &value.definition.values {
                if let Some(slot) = target.get_mut(usize::from(property)) {
                    *slot = f32::from_bits(bits);
                }
            }
        }
        let mut groups = self.groups.lock().unwrap_or_else(|e| e.into_inner());
        let Some(current) = groups.iter_mut().find(|current| current.id == group) else {
            return;
        };
        for property in 0..6 {
            let from = current.lanes[property].value(tick, property);
            if from != target[property] {
                current.lanes[property] = Lane {
                    from,
                    to: target[property],
                    start: tick,
                    duration,
                    transitioning: duration != 0,
                };
            }
        }
        self.publish(&groups, tick);
    }
}

#[derive(Default)]
pub struct States {
    groups: HashMap<u32, StateGroup>,
    values: HashMap<u32, u32>,
    subscribers: Arc<Subscribers>,
}

impl States {
    pub fn new(groups: impl IntoIterator<Item = StateGroup>) -> Self {
        Self {
            groups: groups.into_iter().map(|group| (group.id, group)).collect(),
            values: HashMap::new(),
            subscribers: Default::default(),
        }
    }

    /// New and absent groups return zero (1410d7fc0).
    pub fn current(&self, group: u32) -> u32 {
        self.values.get(&group).copied().unwrap_or(0)
    }

    /// Unknown groups are ignored. The current ID changes without waiting
    /// for the node-property fade to finish (1410d87b0's final store).
    pub fn transition_ms(&self, group: u32, value: u32) -> Option<u32> {
        let definition = self.groups.get(&group)?;
        let previous = self.current(group);
        if previous == value {
            return None;
        }
        let milliseconds = definition
            .transitions
            .iter()
            .find(|transition| transition.from == previous && transition.to == value)
            .map_or(definition.milliseconds, |transition| {
                transition.milliseconds
            });
        Some(milliseconds)
    }

    pub fn set(&mut self, group: u32, value: u32) -> Option<u32> {
        let milliseconds = self.transition_ms(group, value)?;
        let tick = self.subscribers.tick.load(Ordering::Relaxed);
        let nodes = self
            .subscribers
            .nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&group)
            .into_iter()
            .flat_map(|nodes| nodes.values())
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        let duration = u64::from(milliseconds).div_ceil(21);
        for node in nodes {
            node.transition(group, value, tick, duration);
        }
        self.values.insert(group, value);
        Some(milliseconds)
    }

    ///141130ab0 uses the shared1024-sample output quantum, not a game clock.
    pub fn publish_tick(&self, tick: u64) {
        self.subscribers.tick.store(tick, Ordering::Relaxed);
    }

    pub fn subscribe(&self, id: u32, source: &Arc<NodeStates>) -> Option<Arc<Node>> {
        if source.groups.is_empty() {
            return None;
        }
        let mut registered = source.runtime.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(node) = registered.as_ref().filter(|node| {
            node.owner
                .upgrade()
                .is_some_and(|owner| Arc::ptr_eq(&owner, &self.subscribers))
        }) {
            return Some(node.clone());
        }
        let node = {
            let groups = source
                .groups
                .iter()
                .map(|binding| {
                    let mut lanes = [Lane::default(); 6];
                    if let Some(value) = binding
                        .values
                        .iter()
                        .find(|value| value.value == self.current(binding.group))
                    {
                        for &(property, bits) in &value.definition.values {
                            if let Some(lane) = lanes.get_mut(usize::from(property)) {
                                lane.to = f32::from_bits(bits);
                            }
                        }
                    }
                    Group {
                        id: binding.group,
                        lanes,
                    }
                })
                .collect();
            let node = Arc::new(Node {
                id,
                source: Arc::downgrade(source),
                owner: Arc::downgrade(&self.subscribers),
                groups: Mutex::new(groups),
                values: std::array::from_fn(|_| AtomicU32::new(0)),
                transitioning: AtomicBool::new(false),
                last_tick: AtomicU64::new(0),
            });
            node.publish(
                &node.groups.lock().unwrap_or_else(|e| e.into_inner()),
                self.subscribers.tick.load(Ordering::Relaxed),
            );
            let mut registered = self
                .subscribers
                .nodes
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            for binding in &source.groups {
                registered
                    .entry(binding.group)
                    .or_default()
                    .insert(Arc::as_ptr(&node) as usize, Arc::downgrade(&node));
            }
            drop(registered);
            node
        };
        *registered = Some(node.clone());
        Some(node)
    }

    ///On-demand source/output diagnosis; no running history or idle scan.
    pub fn visit_properties(&self, mut visit: impl FnMut(u32, [f32; 6])) {
        let mut seen = HashSet::new();
        let nodes = self
            .subscribers
            .nodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .flat_map(|nodes| nodes.values())
            .filter(|node| seen.insert(node.as_ptr() as usize))
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for node in nodes {
            visit(node.id, node.published());
        }
    }
}
