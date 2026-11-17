//! Level-owned entity-message registry and queued delivery.
//!
//! 140810000 delivers global listeners first, then listeners of the matching
//! entity, applying removals after each event and deferred additions after the
//! queue. 140877430 posts into that same queue during dispatch. Deferred
//! listener changes follow the sorted grouping in 140810430 / 140810550.
//! Normal messages use +0xc8e8 (1408103f0/1408101f0); messages posted by
//! 140810410 use +0xc918 and are drained by 140810200 only after the
//! game-local removal batches (1408ab380), through the same listeners.
use std::collections::{HashMap, VecDeque};

use super::{component_state::entity::ComponentHandle, entity_lifetime::EntityId};

/// A callback is identified by its actual owning component/graph generation
/// and its concrete handler, rather than by an entity name or placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listener {
    ///GameLocal rumble manager7b64e0's normal11a/11b callbacks.
    ControllerFeedback { generation: u64 },
    ///458d00/58a560/608be0: actual AIWorld startup-manager receivers.
    AIWorldStartup { world: EntityId, handle: ComponentHandle, generation: u32, handler: u16 },
    AIStatistics { handle: ComponentHandle, generation:u32, handler:u16 },
    ///41e220's actual AIDangerZone receiver on one full-generation AIWorld.
    AIWorldDanger { world: EntityId, handle: ComponentHandle, generation: u32 },
    SwarmManager { owner: usize, handler: u16 },
    Component {
        owner: usize,
        handle: ComponentHandle,
        generation: u32,
        handler: u16,
    },
    Graph {
        instance: u64,
        node: u32,
        handler: u16,
    },
    /// The mantle state's 8608e0 listener, bound to its real player and dummy.
    MantleState { player: EntityId, dummy: EntityId },
    ///8558e0 normal11d receiver owned by the live UseHeart player state.
    HeartState { player: EntityId },
    ///611fd0's actual Heart component End-of-speech receiver.
    HeartComponent { entity:EntityId },
    PlayerChaos { player: EntityId },
    /// One 854fd0 Action-state callback; the same state ID in another mode
    /// has a distinct source-owned instance and event binding.
    PlayerStateAction { owner: usize, handle: ComponentHandle, generation: u32, mode: usize, state_id: i32 },
    /// One of 842ab0's callbacks, owned by the actual Peeking state instance.
    PlayerStatePeeking { owner: usize, handle: ComponentHandle, generation: u32, mode: u8, state_id: i32, handler: u16 },
    /// The level's single music/ambiance manager, with its native callback ID.
    MusicAndAmbiance { owner: usize, generation: u64, handler: u16 },
    /// 78c6c0's global OnNotesClosed listener, scoped to the live Journal state.
    Journal { generation: u64 },
    /// 674440's entity-targeted Health HUD callback, owned by the live
    /// InGame element rather than by a player component.
    HudHealth { generation: u64 },
    /// 67cc50's three global callbacks on the live game-local statistics owner.
    PlayerStatistics { generation: u64, handler: u16 },
    /// 646670's callbacks on the one live game-local achievement manager.
    Achievements { generation: u64, handler: u16 },
    /// The actual game-local GameFlow receiver; identity changes when that
    /// receiver is replaced, so queued callbacks cannot cross world lifetimes.
    GameFlow { generation: u64, handler: u16 },
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: u16,
    pub target: Option<EntityId>,
    pub payload: Payload,
}

/// Messages whose producer and resource owner already exist on main. Other
/// domain payloads are added by their owning feature when its live callback is
/// connected; they must not introduce a second registry or declaration cache.
#[derive(Clone, Debug)]
pub enum Payload {
    /// 875ba0 -> 70a030: signed joint and the actual SoundEvent declaration lease.
    AnimationSound { joint:i16, event:std::sync::Arc<crate::assets::wwise::SoundEventDeclaration>, placement:usize },
    ///458490's76 packet owns its managed NPC list; no process-pointer list
    ///header is serialized into Bytes across the rewrite's receiver boundary.
    AICueRequest{kind:u32,position:[f32;3],perceivers:Vec<EntityId>,midnight_handle:u32},
    ///458490/458600/458730's77 packet, with both full managed generations.
    AICueGranted{cue:EntityId,instigator:EntityId},
    ///57b470/57b580: the CrewCore48 managed binding and50 atomic name.
    CrewBinding { entity:Option<EntityId>, name:std::sync::Arc<str> },
    ///588a90/588950: a real compiled-tree lease replaces the native pointer.
    CrewTree(Option<std::sync::Arc<crate::assets::ai_world::crew_tree::Definition>>),
    ///588950's27 packet preserves the default-tree comparison and result.
    CrewTreeComplete { default_tree:bool, result:i32, entity:EntityId },
    ///5dbfb0's50 request packet: handler tag, actual trigger and managed target.
    BarkRequest {tag:std::sync::Arc<str>,trigger:u32,target:Option<EntityId>},
    ///5ff2d0/600490: real Observable component, not an NPC-name proxy.
    AIObservable { entity: EntityId, component: super::pickable_component::CheckedComponent, category: u8 },
    ///431c90's3B packet: managed body reference and source mode word.
    AIStatisticsBodyFound { body:Option<EntityId>, kind:i32 },
    ///490510/491b00: source mode, velocity, sphere and actual grenade owner.
    AIDangerSphere(super::ai_world::SphereDanger),
    None,
    ///4d86c1 posts the normal numeric14c packet consumed by626ff0:
    ///six velocity floats, keep-finite byte and a real culprit entity ID.
    ProjectileDeflection { linear:[f32;3], angular:[f32;3], keep_finite:bool,
        culprit:Option<EntityId> },
    ///6dd740's C6 and6dfa50/6df8c0's29 retain the canonical sound
    ///object identity instead of copying a process pointer across owners.
    SoundStimulus { object:super::sound::Emitter, position:[f32;3],
        context:i32, distance:f32, attention_meter:i32,
        vertical_attenuation:bool, call_for_help:bool },
    ///779e00: ARK_MSG_TRIGGER_TOUCH (d2), owned by the actual trigger.
    TriggerTouch { trigger: ComponentHandle, other: EntityId, entered: bool },
    ///779e00: ARK_MSG_TRIGGER_CONTACT (d3), delivered to the other entity.
    TriggerContact { trigger: ComponentHandle, entered: bool },
    ///77ff90 ->80bc60: actual Wind component and full-generation body ID.
    WindContact { wind: ComponentHandle, body: u32, entered: bool,
        ///Host projection of the same immutable Wind declaration and payload.
        source_index: usize, priority: i32 },
    Use { instigator: Option<EntityId> },
    Bytes(Vec<u8>),
    Entity(EntityId),
    /// Inline request-owner notification (1406645e0), event 16's two words.
    MusicRequest { handle: i32, operation: i32 },
    /// 876660 ->874ad0: ARK_MSG_DAMAGED (A2), including the exact
    /// source parameters followed by its health calculation result.
    Damage { params: super::damage::Parameters, result: super::damage::DamageResult },
    /// 8767a0 ->874b90: ARK_MSG_DAMAGING (A3), delivered to attacker.
    Damaging { target: EntityId, params: super::damage::Parameters, result: super::damage::DamageResult },
    /// 8769f0/717950: A5 incapacitation or 1A6 damage after death.
    DamageParameters(super::damage::Parameters),
    ///8769f0 snapshots parameters+88 bit0 before subsequent context changes.
    Incapacitation { params:super::damage::Parameters, non_counting:bool },
    /// 6170b0, message1AD: the actual item declaration and quantity change.
    InventoryQuantity { item: std::sync::Arc<crate::assets::inventory_item::PickableItem>, previous: u32, current: u32 },
    /// 615840/6147f0, messages9B/9C: transferred count and resulting total.
    InventoryTransfer { item: std::sync::Arc<crate::assets::inventory_item::PickableItem>, count: u32, total: u32 },
}

#[derive(Clone, Copy)]
enum Phase { Normal, AfterEntityRemovals }
impl Phase {
    fn index(self) -> usize { self as usize }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Binding {
    event: u16,
    target: Option<EntityId>,
    listener: Listener,
}

#[derive(Clone, Copy, Debug)]
enum Change {
    Add(Binding),
    Remove(Binding),
}

impl Change {
    fn binding(self) -> Binding {
        match self {
            Self::Add(value) | Self::Remove(value) => value,
        }
    }
}

/// Commands produced by one callback are transferred back to this same owner
/// before the next queued event. Callbacks may also append component lifecycle
/// work through the exclusive host that invokes them.
#[derive(Default)]
pub struct Commands {
    events: [Vec<Event>; 2],
    changes: Vec<Change>,
}

impl Commands {
    pub fn post(&mut self, event: Event) {
        self.events[Phase::Normal.index()].push(event);
    }
    /// 140810410: defer to the post-removal drain, even during a normal pump.
    pub fn post_after_entity_removals(&mut self, event: Event) {
        self.events[Phase::AfterEntityRemovals.index()].push(event);
    }
    pub fn register(&mut self, event: u16, target: Option<EntityId>, listener: Listener) {
        self.changes.push(Change::Add(Binding {
            event,
            target,
            listener,
        }));
    }
    pub fn unregister(&mut self, event: u16, target: Option<EntityId>, listener: Listener) {
        self.changes.push(Change::Remove(Binding {
            event,
            target,
            listener,
        }));
    }
}

#[cfg_attr(feature = "game", derive(bevy::prelude::Resource))]
pub struct EntityEvents {
    listeners: HashMap<(u16, Option<EntityId>), Vec<Listener>>,
    queues: [VecDeque<Event>; 2],
    pending: Vec<Change>,
    trace: Option<Trace>,
}

#[derive(Default)]
struct Trace {
    pumps: u64,
    events: u64,
    deliveries: u64,
    failures: u64,
    peak_queue: usize,
    peak_pending: usize,
    elapsed: std::time::Duration,
}

impl Default for EntityEvents {
    fn default() -> Self {
        Self {
            listeners: HashMap::new(),
            queues: std::array::from_fn(|_| VecDeque::new()),
            pending: Vec::new(),
            trace: (std::env::var("DH2_DEBUG_ENTITY_EVENTS").as_deref() == Ok("1"))
                .then(Trace::default),
        }
    }
}

impl Drop for EntityEvents {
    fn drop(&mut self) {
        if let Some(trace) = &self.trace {
            if trace.pumps != 0 {
                eprintln!("ENTITY_EVENTS pumps={} events={} deliveries={} errors={} peak_queue={} peak_changes={} pump_ms={:.3}",
                    trace.pumps, trace.events, trace.deliveries, trace.failures, trace.peak_queue,
                    trace.peak_pending, trace.elapsed.as_secs_f64() * 1000.0);
            }
        }
    }
}

impl EntityEvents {
    /// Inspect the existing normal FIFO/deferred listener edits without
    /// dispatching the post-removal queue during whole-world destruction.
    pub(crate) fn normal_pending(&self)->bool{!self.queues[Phase::Normal.index()].is_empty()||!self.pending.is_empty()}
    /// Final game-local owner teardown, after entity callbacks have detached.
    /// Do not dispatch the secondary queue here: 810200 belongs to ordinary
    /// removal batches, not whole-world destruction.
    pub fn clear_for_world_shutdown(&mut self) {
        self.listeners = HashMap::new();
        self.queues = std::array::from_fn(|_| VecDeque::new());
        self.pending = Vec::new();
    }

    /// Exclusive-world teardown checks the actual registered callbacks before
    /// changing any component. Pending additions belong to this same owner.
    pub fn validate_listeners(&self, mut validate: impl FnMut(Listener) -> Result<(), String>) -> Result<(), String> {
        for listener in self.listeners.values().flatten().copied() {
            validate(listener)?;
        }
        for change in &self.pending {
            if let Change::Add(binding) = change { validate(binding.listener)?; }
        }
        Ok(())
    }

    /// Registration outside dispatch on the designated registry thread follows
    /// 140810820's immediate path; duplicate callbacks remain separate entries.
    pub fn register(&mut self, event: u16, target: Option<EntityId>, listener: Listener) {
        self.add(Binding {
            event,
            target,
            listener,
        });
    }

    pub fn unregister(&mut self, event: u16, target: Option<EntityId>, listener: Listener) {
        self.remove(Binding {
            event,
            target,
            listener,
        });
    }

    /// 140810820 / 140811170 queue operations when dispatch is active or the
    /// caller is not the thread selected by 140166670. The host must choose
    /// that path from the actual registry-thread designation.
    pub fn defer_register(&mut self, event: u16, target: Option<EntityId>, listener: Listener) {
        self.defer(Change::Add(Binding {
            event,
            target,
            listener,
        }));
    }

    pub fn defer_unregister(&mut self, event: u16, target: Option<EntityId>, listener: Listener) {
        self.defer(Change::Remove(Binding {
            event,
            target,
            listener,
        }));
    }

    fn defer(&mut self, change: Change) {
        if self.pending.len() < 0x400 {
            self.pending.push(change);
        }
    }

    pub fn post(&mut self, event: Event) {
        self.queues[Phase::Normal.index()].push_back(event);
    }

    /// 140810410, not a second listener registry or a normal-frame message.
    pub fn post_after_entity_removals(&mut self, event: Event) {
        self.queues[Phase::AfterEntityRemovals.index()].push_back(event);
    }

    fn add(&mut self, binding: Binding) {
        self.listeners
            .entry((binding.event, binding.target))
            .or_default()
            .push(binding.listener);
    }

    fn remove(&mut self, binding: Binding) {
        let key = (binding.event, binding.target);
        if let Some(listeners) = self.listeners.get_mut(&key) {
            if let Some(index) = listeners
                .iter()
                .position(|listener| *listener == binding.listener)
            {
                listeners.remove(index);
            }
            if listeners.is_empty() {
                self.listeners.remove(&key);
            }
        }
    }

    /// Deliver all currently queued messages and messages posted by listeners.
    /// The host owns component/graph/audio state and can queue registry work
    /// inside `dispatch`; callback errors are isolated to that listener.
    pub fn pump<F>(&mut self, dispatch: F) -> Vec<String>
    where
        F: FnMut(Listener, &Event, &mut Commands) -> Result<(), String>,
    {
        self.pump_phase(Phase::Normal, dispatch)
    }

    /// 140810200 at 1408ababc: call after all current entity-removal batches,
    /// not after each component drain or each removed entity.
    pub fn pump_after_entity_removals<F>(&mut self, dispatch: F) -> Vec<String>
    where
        F: FnMut(Listener, &Event, &mut Commands) -> Result<(), String>,
    {
        self.pump_phase(Phase::AfterEntityRemovals, dispatch)
    }

    fn pump_phase<F>(&mut self, phase: Phase, mut dispatch: F) -> Vec<String>
    where
        F: FnMut(Listener, &Event, &mut Commands) -> Result<(), String>,
    {
        let started = self.trace.as_ref().map(|_| std::time::Instant::now());
        let mut events = 0u64;
        let mut deliveries = 0u64;
        let mut peak_queue = self.queues[phase.index()].len();
        let mut peak_pending = self.pending.len();
        let mut errors = Vec::new();
        let mut commands = Commands::default();
        self.apply_pending(false);
        while let Some(event) = self.queues[phase.index()].pop_front() {
            if started.is_some() {
                events += 1;
            }
            if let Some(global) = self.listeners.get(&(event.id, None)) {
                for &listener in global {
                    if started.is_some() {
                        deliveries += 1;
                    }
                    if let Err(error) = dispatch(listener, &event, &mut commands) {
                        errors.push(error);
                    }
                }
            }
            if let Some(target) = event.target {
                if let Some(local) = self.listeners.get(&(event.id, Some(target))) {
                    for &listener in local {
                        if started.is_some() {
                            deliveries += 1;
                        }
                        if let Err(error) = dispatch(listener, &event, &mut commands) {
                            errors.push(error);
                        }
                    }
                }
            }
            for (target, events) in self.queues.iter_mut().zip(&mut commands.events) {
                target.extend(events.drain(..));
            }
            // 140810820/140811170 discard deferred operations once their
            // fixed 0x400-entry operation buffer is full.
            let available = 0x400usize.saturating_sub(self.pending.len());
            self.pending
                .extend(commands.changes.drain(..).take(available));
            if started.is_some() {
                peak_queue = peak_queue.max(self.queues[phase.index()].len());
                peak_pending = peak_pending.max(self.pending.len());
            }
            self.apply_pending(true);
        }
        self.apply_pending(false);
        if let (Some(started), Some(trace)) = (started, self.trace.as_mut()) {
            trace.pumps += 1;
            trace.events += events;
            trace.deliveries += deliveries;
            trace.failures += errors.len() as u64;
            trace.peak_queue = trace.peak_queue.max(peak_queue);
            trace.peak_pending = trace.peak_pending.max(peak_pending);
            trace.elapsed += started.elapsed();
        }
        errors
    }

    fn apply_pending(&mut self, removals_only: bool) {
        if self.pending.is_empty() {
            return;
        }
        let mut changes = std::mem::take(&mut self.pending);
        if changes.len() == 1 {
            let change = changes[0];
            match change {
                Change::Add(binding) if !removals_only => self.add(binding),
                Change::Remove(binding) => self.remove(binding),
                Change::Add(_) => {}
            }
            if !removals_only {
                changes.clear();
            }
            self.pending = changes;
            return;
        }
        // Native sorting groups by entity and event and keeps the original
        // operation ordinal inside each group. Different groups mutate
        // disjoint listener vectors, so their host order is immaterial.
        let mut groups = HashMap::new();
        let mut next_group = 0usize;
        changes.sort_by_key(|change| {
            let binding = change.binding();
            *groups
                .entry((binding.event, binding.target))
                .or_insert_with(|| {
                    let group = next_group;
                    next_group += 1;
                    group
                })
        });
        let mut start = 0;
        while start < changes.len() {
            let binding = changes[start].binding();
            let mut end = start + 1;
            while end < changes.len() && changes[end].binding() == binding {
                end += 1;
            }
            let group = &changes[start..end];
            if removals_only {
                if matches!(group.last(), Some(Change::Remove(_))) {
                    self.remove(binding);
                }
                // 140810550's compaction branch at 140810659 is not taken
                // for its descending nonnegative indices. The group remains
                // pending for subsequent events and the final full flush.
                self.pending.extend_from_slice(group);
            } else {
                let additions = group
                    .iter()
                    .filter(|change| matches!(change, Change::Add(_)))
                    .count();
                if additions != group.len() - additions {
                    match group.last().copied().expect("nonempty listener group") {
                        Change::Add(binding) => self.add(binding),
                        Change::Remove(binding) => self.remove(binding),
                    }
                }
            }
            start = end;
        }
        if self.pending.is_empty() {
            changes.clear();
            self.pending = changes;
        }
    }
}
