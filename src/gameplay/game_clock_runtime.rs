//! Loaded-game clock owner, using the original counter conversion, measured-FPS
//! producer and five-group update. This publishes real game-clock state, not a
//! Bevy elapsed-time substitute. The original identity-ordered gameplay view
//! clocks are published after gameplay and extracted for native rendering.
//! Camera construction, model lifecycle and save/time-stream transition
//! ownership remain separate integrations.
use crate::{
    game_clock::{
        self, ClockControls, ClockStepInputs, CounterMicroseconds, FrameTimingEstimator, GameClock,
    },
    game_settings,
};
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};

use crate::gameplay::entity_lifetime::EntityId;
use crate::gameplay::powers::world_effects::{TimeLease, TimeRequest, TimeRequests};
use crate::platform::performance_counter as platform;
use std::collections::{HashMap, VecDeque};

struct EntityClock {
    timespace: u32,
    offset: f64,
    remaining: Option<f32>,
    managed_override: bool,
    /// idEntity+2b8, constructor874c50 initializes the explicit time-space
    /// attachment separately from the actor's ordinary binding parent.
    time_parent: Option<EntityId>,
    time_children: Vec<EntityId>,
}
impl EntityClock {
    ///80f040 advances only the root countdown here. The application publishes
    ///80f620's real binding traversal before changing selectors/model clocks.
    fn advance(&mut self, _clock: &GameClock, bent: bool, step: f32) -> Option<u32> {
        if !self.managed_override {
            return None;
        }
        let unbent = if let Some(remaining) = self.remaining.as_mut() {
            let active = bent && step < *remaining;
            if *remaining < f32::from_bits(0x7149_f2ca) {
                *remaining -= step;
            }
            if !active {
                self.remaining = None;
            }
            active
        } else {
            false
        };
        if self.remaining.is_none() {
            self.managed_override = false;
        }
        (self.timespace <= 1).then_some(u32::from(unbent))
    }
}
struct EntityTimeNode {
    entity: EntityId,
    stage: u8,
    changed: bool,
}
struct EntityTimePublication {
    root: Option<EntityId>,
    target: u32,
    pending: Vec<EntityId>,
    current: Option<EntityTimeNode>,
    visited: Vec<EntityId>,
}

struct PerformanceCounter {
    origin: i64,
    frequency: i64,
    microseconds: CounterMicroseconds,
}

impl PerformanceCounter {
    fn new() -> Result<Self, String> {
        let frequency = platform::frequency()?;
        let origin = platform::counter()?;
        let mut microseconds = CounterMicroseconds::default();
        microseconds
            .read(0, frequency)
            .ok_or("QPC frequency has no native microsecond divisor")?;
        Ok(Self {
            origin,
            frequency,
            microseconds,
        })
    }
    fn elapsed_counter(&self) -> Result<i64, String> {
        Ok(platform::counter()?.wrapping_sub(self.origin))
    }
    fn microseconds(&mut self) -> Result<u64, String> {
        let counter = self.elapsed_counter()?;
        self.microseconds
            .read(counter, self.frequency)
            .ok_or_else(|| "counter scale unavailable".into())
    }
}

struct LoadedClock {
    clock: GameClock,
    source: PerformanceCounter,
    timing: FrameTimingEstimator,
    controls: ClockControls,
    /// Native common constructor 03e4c0 initializes +1ed8 to zero. Loaded
    /// normal gameplay has no save/time-stream override. 048940/048a50's
    /// transition I/O/callback lifecycle is not silently emulated here.
    common_counter: i32,
    advanced: bool,
    /// 80d8c0/80f040: the player root's timed unbent request. This is a
    /// duration in game-local unquantized time, not an extra time factor.
    player_entity: Option<EntityId>,
    player_override: Option<f32>,
    player_timespace: u32,
    player_clock_offset: f64,
    player_time_children: Vec<EntityId>,
    entity_clocks: HashMap<EntityId, EntityClock>,
    entity_overrides: Vec<EntityId>,
    /// Actual87ce70/87d050 notifications awaiting the application owners.
    /// This is a publication FIFO, never a second clock or entity registry.
    entity_transitions: VecDeque<(EntityId, u32)>,
    entity_publication: Option<EntityTimePublication>,
    publication_spare: Option<EntityTimePublication>,
    power_requests: TimeRequests,
}

impl LoadedClock {
    fn entity_values(&self, id: EntityId) -> Option<(u32, f64, Option<f32>)> {
        if self.player_entity == Some(id) {
            return Some((
                self.player_timespace,
                self.player_clock_offset,
                self.player_override,
            ));
        }
        self.entity_clocks
            .get(&id)
            .map(|record| (record.timespace, record.offset, record.remaining))
    }
    fn advance_player_timer(&mut self, bent: bool, step: f32) -> u32 {
        let unbent = if let Some(remaining) = self.player_override.as_mut() {
            let active = bent && step < *remaining;
            if *remaining < f32::from_bits(0x7149_f2ca) {
                *remaining -= step;
            }
            if !active {
                self.player_override = None;
            }
            active
        } else {
            false
        };
        u32::from(unbent)
    }
    /// A paused frame (`FUN_1408ad580`): 0450d0 still updates the frame
    /// estimator, but 8af320 (frame counter and 059990) is not called.
    fn paused(&mut self) -> Result<(), String> {
        self.timing.update(|| self.source.microseconds())?;
        self.advanced = false;
        Ok(())
    }

    fn advance(&mut self) -> Result<(), String> {
        // 0450d0 updates the frame estimator before running the game. 8af320
        // increments the clock's frame counter before invoking 059990.
        self.timing.update(|| self.source.microseconds())?;
        self.clock.increment_frame_counter();
        self.advanced = self.clock.step(ClockStepInputs {
            counter_since_origin: self.source.elapsed_counter()?,
            counter_frequency: self.source.frequency,
            variable_step: game_clock::allows_variable_step(self.common_counter),
            force_fixed_step: self.controls.force_fixed_step,
            fixed_step: game_clock::DEFAULT_STEP,
            jitter_window: self.controls.physics_step_tolerance,
            rate_cap: self.timing.frame_rate(),
            grow_step_limit: self.controls.limit_step_variation,
        });
        if self.advanced {
            let bent = self.clock.rates(3).is_some_and(|v| v[0] < 1.0)
                || self.clock.rates(4).is_some_and(|v| v[0] < 1.0);
            // 8af320 passes clock group4 to 80f040, including the
            // current unbent scaling; it is not the wall-clock delta.
            let step = self.clock.delta(4).unwrap_or(0.0);
            if self.player_entity.is_none() {
                //The already constructed but not yet managed Player keeps its
                //one existing bridge; no fabricated identity enters the list.
                let timespace = self.advance_player_timer(bent, step);
                if timespace != self.player_timespace {
                    let old = if self.player_timespace == 0 { 3 } else { 4 };
                    let new = if timespace == 0 { 3 } else { 4 };
                    let elapsed = (self
                        .clock
                        .previous_time(old)
                        .ok_or("player old clock group absent")?
                        - self.player_clock_offset) as f32;
                    self.player_clock_offset = self
                        .clock
                        .previous_time(new)
                        .ok_or("player new clock group absent")?
                        - f64::from(elapsed);
                    self.player_timespace = timespace;
                }
            }
            //80f040 visits the actual unbent roots in reverse and removes
            //expired entries by swapping the final slot, preserving its order.
            for index in (0..self.entity_overrides.len()).rev() {
                let id = self.entity_overrides[index];
                if self.player_entity == Some(id) {
                    let space = self.advance_player_timer(bent, step);
                    self.entity_transitions.push_back((id, space));
                    if self.player_override.is_none() {
                        self.entity_overrides.swap_remove(index);
                    }
                    continue;
                }
                let Some(entity) = self.entity_clocks.get_mut(&id) else {
                    self.entity_overrides.swap_remove(index);
                    continue;
                };
                if let Some(space) = entity.advance(&self.clock, bent, step) {
                    self.entity_transitions.push_back((id, space));
                }
                if !entity.managed_override {
                    self.entity_overrides.swap_remove(index);
                }
            }
        }
        Ok(())
    }
}

/// Actual main-world owner available to the native scene publisher. A platform
/// or configuration error stays unavailable: no zero-clock/Instant fallback.
#[derive(Resource)]
pub struct ClockRuntime(Result<LoadedClock, String>);

impl ClockRuntime {
    pub fn clock(&self) -> Result<&GameClock, &str> {
        self.0
            .as_ref()
            .map(|state| &state.clock)
            .map_err(String::as_str)
    }
    pub fn advanced(&self) -> bool {
        self.0.as_ref().is_ok_and(|state| state.advanced)
    }
    /// 140059890 publication, using the existing byte-exact easing owner.
    pub fn set_rate_target(&mut self, group: usize, value: f32, ease: f32) -> Result<(), String> {
        self.0
            .as_mut()
            .map_err(|error| error.clone())?
            .clock
            .set_rate_target(group, value, ease)
    }
    pub fn acquire_time_request(&mut self, request: TimeRequest) -> Result<TimeLease, String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        let (lease, target) = state.power_requests.acquire(request);
        state.clock.set_rate_target(3, target.world, target.ease)?;
        state.clock.set_rate_target(4, target.unbent, target.ease)?;
        Ok(lease)
    }
    pub fn release_time_request(&mut self, lease: TimeLease, ease: f32) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        if let Some(target) = state.power_requests.release(lease, ease) {
            state.clock.set_rate_target(3, target.world, target.ease)?;
            state.clock.set_rate_target(4, target.unbent, target.ease)?;
        }
        Ok(())
    }
    /// Stop power requests while actual entity roots remain registered until
    /// their component, render and audio retirement acknowledges release.
    pub fn clear_power_time(&mut self) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        state.power_requests = TimeRequests::default();
        for id in state.entity_overrides.drain(..) {
            if state.player_entity == Some(id) {
                state.player_override = None;
                state.entity_transitions.push_back((id, 0));
                continue;
            }
            if let Some(entity) = state.entity_clocks.get_mut(&id) {
                if let Some(space) = entity.advance(&state.clock, false, 0.0) {
                    state.entity_transitions.push_back((id, space));
                }
            }
        }
        state.player_override = None;
        if let Some(player) = state.player_entity {
            if state.player_timespace != 0
                && !state
                    .entity_transitions
                    .iter()
                    .any(|&(id, space)| id == player && space == 0)
            {
                state.entity_transitions.push_back((player, 0));
            }
        } else {
            state.player_timespace = 0;
            state.player_clock_offset = 0.;
        }
        state.clock.set_rate_target(3, 1.0, 0.0)?;
        state.clock.set_rate_target(4, 1.0, 0.0)
    }
    /// 14080d8c0: an indefinite request cannot be replaced by a shorter
    /// request until the last world-time lease has restored normal time.
    pub fn set_player_time_override(&mut self, duration: f32) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|error| error.clone())?;
        if state.player_override != Some(f32::from_bits(0x7149_f2ca)) {
            state.player_override = Some(duration);
        }
        if let Some(player) = state.player_entity {
            if !state.entity_overrides.contains(&player) {
                state.entity_overrides.push(player);
            }
        }
        Ok(())
    }
    pub fn player_timespace(&self) -> Result<u32, &str> {
        self.0
            .as_ref()
            .map(|state| state.player_timespace)
            .map_err(String::as_str)
    }
    pub fn player_delta(&self) -> Result<f32, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        state
            .clock
            .entity_delta(state.player_timespace)
            .ok_or("player clock timespace is invalid")
    }
    pub fn player_time(&self) -> Result<f64, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let group = if state.player_timespace == 0 { 3 } else { 4 };
        Ok(state
            .clock
            .time(group)
            .ok_or("player clock group is absent")?
            - state.player_clock_offset)
    }
    /// Bind the real managed Player to its existing pre-admission clock
    /// fields. No generic EntityClock record duplicates this same object.
    pub fn bind_player_entity(&mut self, entity: EntityId) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        if state.player_entity.is_some_and(|current| current != entity) {
            return Err("another player owns the actual clock bridge".into());
        }
        if state.entity_clocks.contains_key(&entity) {
            return Err("actual player already has a duplicate generic clock record".into());
        }
        state.player_entity = Some(entity);
        if state.player_override.is_some() && !state.entity_overrides.contains(&entity) {
            state.entity_overrides.push(entity);
        }
        Ok(())
    }
    /// Actual entity roots register once, then their component/model owners
    /// query this same timespace selection. No entity is fabricated here.
    pub fn register_entity_time(&mut self, entity: EntityId, timespace: u32) -> Result<(), String> {
        if timespace > 3 {
            return Err("entity timespace is outside 0..4".into());
        }
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        if state.player_entity == Some(entity) {
            return Ok(());
        }
        state.entity_clocks.entry(entity).or_insert(EntityClock {
            timespace,
            offset: 0.0,
            remaining: None,
            managed_override: false,
            time_parent: None,
            time_children: Vec::new(),
        });
        Ok(())
    }
    ///875cf0 is Enable's direct selector reset. It preserves the actual
    ///entity+2a0 offset/countdown instead of synthesizing a time transition.
    pub(crate) fn reset_entity_timespace(&mut self,entity:EntityId)->Result<(),String> {
        let state=self.0.as_mut().map_err(|e|e.clone())?;
        if state.player_entity==Some(entity){state.player_timespace=0;return Ok(());}
        state.entity_clocks.get_mut(&entity).ok_or("enabled entity clock is not registered")?.timespace=0;
        Ok(())
    }
    pub fn unregister_entity_time(&mut self, entity: EntityId) {
        if let Ok(state) = &mut self.0 {
            //Final generation destruction releases both attachment links.
            //Surviving children must first pass the application's80e050
            //detach path, while actual binding/Scene owners still exist.
            let parent=state.entity_clocks.get(&entity).and_then(|record|record.time_parent);
            if let Some(parent)=parent {
                if state.player_entity==Some(parent) {
                    if let Some(index)=state.player_time_children.iter().position(|child|*child==entity) {state.player_time_children.swap_remove(index);}
                } else if let Some(record)=state.entity_clocks.get_mut(&parent) {
                    if let Some(index)=record.time_children.iter().position(|child|*child==entity) {record.time_children.swap_remove(index);}
                }
            }
            let children=if state.player_entity==Some(entity) {std::mem::take(&mut state.player_time_children)}
                else {state.entity_clocks.get_mut(&entity).map(|record|std::mem::take(&mut record.time_children)).unwrap_or_default()};
            for child in children {
                if let Some(record)=state.entity_clocks.get_mut(&child) {
                    if record.time_parent==Some(entity) {record.time_parent=None;}
                }
            }
            if state.player_entity == Some(entity) {
                state.player_entity = None;
                state.player_override = None;
                //The actual Player has retired. Its next constructed bridge
                //receives the same source constructor selector/offset zero.
                state.player_timespace = 0;
                state.player_clock_offset = 0.;
                state.player_time_children.clear();
            }
            state.entity_clocks.remove(&entity);
            state.entity_overrides.retain(|id| *id != entity);
            state.entity_transitions.retain(|(id, _)| *id != entity);
            if state
                .entity_publication
                .as_ref()
                .is_some_and(|work| work.root == Some(entity))
            {
                if let Some(mut work) = state.entity_publication.take() {
                    work.root = None;
                    work.pending.clear();
                    work.visited.clear();
                    work.current = None;
                    state.publication_spare = Some(work);
                }
            } else if let Some(work) = state.entity_publication.as_mut() {
                work.pending.retain(|id| *id != entity);
                if work
                    .current
                    .as_ref()
                    .is_some_and(|node| node.entity == entity)
                {
                    work.current = None;
                }
            }
        }
    }
    pub fn set_entity_time_override(
        &mut self,
        entity: EntityId,
        duration: f32,
    ) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        if state.player_entity == Some(entity) {
            if state.player_override != Some(f32::from_bits(0x7149_f2ca)) {
                state.player_override = Some(duration);
            }
            if !state.entity_overrides.contains(&entity) {
                state.entity_overrides.push(entity);
            }
            return Ok(());
        }
        let record = state
            .entity_clocks
            .get_mut(&entity)
            .ok_or("entity clock root is not registered")?;
        if record.remaining != Some(f32::from_bits(0x7149_f2ca)) {
            record.remaining = Some(duration);
        }
        record.managed_override = true;
        if !state.entity_overrides.contains(&entity) {
            state.entity_overrides.push(entity);
        }
        Ok(())
    }
    pub(crate) fn entity_time_parent(&self, entity: EntityId) -> Result<Option<EntityId>, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        if state.player_entity == Some(entity) {
            return Ok(None);
        }
        Ok(state
            .entity_clocks
            .get(&entity)
            .ok_or("entity clock root is not registered")?
            .time_parent)
    }
    pub(crate) fn is_player_entity(&self, entity: EntityId) -> bool {
        self.0.as_ref().is_ok_and(|state| state.player_entity == Some(entity))
    }
    pub(crate) fn entity_time_children(&self, entity: EntityId) -> Result<&[EntityId], &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        if state.player_entity == Some(entity) { return Ok(&state.player_time_children); }
        Ok(&state.entity_clocks.get(&entity).ok_or("time attachment owner is not registered")?.time_children)
    }
    ///80cd60 changes only explicit+2b8/+2c0 links, never selectors/offsets.
    pub(crate) fn attach_entity_time(&mut self, parent: EntityId, child: EntityId) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|error|error.clone())?;
        if state.player_entity == Some(child) { return Err("explicit Player time-parent admission is not hosted".into()); }
        if state.entity_clocks.get(&child).ok_or("time attachment child is not registered")?.time_parent.is_some() {
            return Err("time attachment child must detach its previous parent".into());
        }
        let children = if state.player_entity == Some(parent) { &mut state.player_time_children }
            else { &mut state.entity_clocks.get_mut(&parent).ok_or("time attachment parent is not registered")?.time_children };
        if !children.contains(&child) {
            children.try_reserve(1).map_err(|error| format!("time attachment children: {error}"))?;
            children.push(child);
        }
        state.entity_clocks.get_mut(&child).ok_or("time attachment child disappeared")?.time_parent = Some(parent);
        Ok(())
    }
    ///80e050's raw unlink. The application computes its actual old/new roots
    ///and transfers the selector1 countdown before acknowledging the operation.
    pub(crate) fn unlink_entity_time(&mut self, child: EntityId) -> Result<Option<EntityId>, String> {
        let state = self.0.as_mut().map_err(|error|error.clone())?;
        if state.player_entity == Some(child) { return Ok(None); }
        let Some(record) = state.entity_clocks.get_mut(&child) else { return Ok(None); };
        let Some(parent) = record.time_parent.take() else { return Ok(None); };
        //80e050 uses7b8270's first-match swap removal, preserving its exact
        //subsequent80f620 attachment traversal order.
        if state.player_entity == Some(parent) {
            if let Some(index)=state.player_time_children.iter().position(|id|*id==child) {state.player_time_children.swap_remove(index);}
        } else if let Some(record) = state.entity_clocks.get_mut(&parent) {
            if let Some(index)=record.time_children.iter().position(|id|*id==child) {record.time_children.swap_remove(index);}
        }
        Ok(Some(parent))
    }
    pub(crate) fn entity_transition_front(
        &mut self,
    ) -> Result<Option<(EntityId, u32, u8, bool)>, String> {
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        loop {
            if state.entity_publication.is_none() {
                let Some(&(root, target)) = state.entity_transitions.front() else {
                    return Ok(None);
                };
                let mut work =
                    state
                        .publication_spare
                        .take()
                        .unwrap_or_else(|| EntityTimePublication {
                            root: None,
                            target: 0,
                            pending: Vec::new(),
                            current: None,
                            visited: Vec::new(),
                        });
                work.root = Some(root);
                work.target = target;
                work.pending.clear();
                work.pending.push(root);
                work.current = None;
                work.visited.clear();
                state.entity_publication = Some(work);
            }
            let work = state
                .entity_publication
                .as_mut()
                .ok_or("entity time publication absent")?;
            if work.current.is_none() {
                if let Some(entity) = work.pending.pop() {
                    if work.visited.contains(&entity) {
                        return Err("cyclic entity time publication".into());
                    }
                    work.visited.push(entity);
                    work.current = Some(EntityTimeNode {
                        entity,
                        stage: 0,
                        changed: false,
                    });
                } else {
                    state.entity_transitions.pop_front();
                    let mut work = state
                        .entity_publication
                        .take()
                        .ok_or("entity time publication absent")?;
                    work.root = None;
                    work.pending.clear();
                    work.visited.clear();
                    work.current = None;
                    state.publication_spare = Some(work);
                    continue;
                }
            }
            let node = work
                .current
                .as_ref()
                .ok_or("entity time publication node absent")?;
            return Ok(Some((node.entity, work.target, node.stage, node.changed)));
        }
    }
    pub(crate) fn prepare_entity_transition(
        &mut self,
        entity: EntityId,
        space: u32,
        excluded: bool,
    ) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        let (current, offset, _) = state
            .entity_values(entity)
            .ok_or("entity clock root is not registered")?;
        let work = state
            .entity_publication
            .as_mut()
            .ok_or("entity time publication absent")?;
        let node = work
            .current
            .as_mut()
            .ok_or("entity time publication node absent")?;
        if node.entity != entity || work.target != space || node.stage != 0 {
            return Err("entity time preparation differs".into());
        }
        if !excluded && current != space {
            let old = *[3usize, 4, 2, 0]
                .get(current as usize)
                .ok_or("entity timespace invalid")?;
            let new = *[3usize, 4, 2, 0]
                .get(space as usize)
                .ok_or("entity target timespace invalid")?;
            let elapsed = (state
                .clock
                .previous_time(old)
                .ok_or("entity clock old group absent")?
                - offset) as f32;
            let offset = state
                .clock
                .previous_time(new)
                .ok_or("entity clock new group absent")?
                - f64::from(elapsed);
            if state.player_entity == Some(entity) {
                state.player_timespace = space;
                state.player_clock_offset = offset;
            } else {
                let record = state
                    .entity_clocks
                    .get_mut(&entity)
                    .ok_or("entity clock root is not registered")?;
                record.timespace = space;
                record.offset = offset;
            }
            node.changed = true;
        }
        node.stage = 1;
        Ok(())
    }
    pub(crate) fn advance_entity_transition(
        &mut self,
        entity: EntityId,
        space: u32,
        stage: u8,
    ) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        let work = state
            .entity_publication
            .as_mut()
            .ok_or("entity time publication absent")?;
        let node = work
            .current
            .as_mut()
            .ok_or("entity time publication node absent")?;
        if node.entity != entity || work.target != space || node.stage != stage {
            return Err("entity time acknowledgement differs".into());
        }
        node.stage += 1;
        Ok(())
    }
    pub(crate) fn finish_entity_transition(
        &mut self,
        entity: EntityId,
        space: u32,
        children: &[EntityId],
        attached_roots: &[EntityId],
    ) -> Result<(), String> {
        let state = self.0.as_mut().map_err(|e| e.clone())?;
        let work = state
            .entity_publication
            .as_mut()
            .ok_or("entity time publication absent")?;
        let node = work
            .current
            .as_ref()
            .ok_or("entity time publication node absent")?;
        if node.entity != entity || work.target != space || node.stage != 4 {
            return Err("entity time completion differs".into());
        }
        work.pending
            .try_reserve(children.len() + attached_roots.len())
            .map_err(|error| format!("entity time publication: {error}"))?;
        //80f620 visits actor children before explicit attachments, whose
        //actual877500 binding roots are supplied by the LiveEntities owner.
        work.pending.extend(attached_roots.iter().rev().copied());
        work.pending.extend(children.iter().rev().copied());
        work.current = None;
        Ok(())
    }
    ///87ce70/87d050 project the real entity+2a0 offset to model+1c2c and
    ///select its game-time group through1422952d8, without another clock.
    pub fn entity_model_clock(&self, entity: EntityId) -> Result<(u8, f32), &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let (space, offset, _) = state
            .entity_values(entity)
            .ok_or("entity clock root is not registered")?;
        let group = *[3, 4, 2, 0]
            .get(space as usize)
            .ok_or("entity timespace invalid")?;
        Ok((group, offset as f32))
    }
    pub(crate) fn time_is_bent(&self) -> Result<bool, &str> {
        Ok(self.0.as_ref().map_err(String::as_str)?.clock.as_bytes()[0x1a0] != 0)
    }
    /// Source entity+2ac selector and+2a8 countdown, including the actual
    /// Player bridge. None is the constructor/expired zero.
    pub fn entity_time_state(&self, entity: EntityId) -> Result<(u32, Option<f32>), &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let (space, _, remaining) = state
            .entity_values(entity)
            .ok_or("entity clock root is not registered")?;
        Ok((space, remaining))
    }
    pub fn entity_rates(&self, entity: EntityId) -> Result<[f32; 2], &str> {
        let (space, _) = self.entity_time_state(entity)?;
        self.0
            .as_ref()
            .map_err(String::as_str)?
            .clock
            .entity_rates(space)
            .ok_or("entity timespace invalid")
    }
    pub fn entity_delta(&self, entity: EntityId) -> Result<f32, &str> {
        let (space, _) = self.entity_time_state(entity)?;
        self.0
            .as_ref()
            .map_err(String::as_str)?
            .clock
            .entity_delta(space)
            .ok_or("entity timespace invalid")
    }
    /// 878030 reads the entity's selector and offset. An unmodified placed
    /// entity has its constructed zero offset and needs no separate record.
    pub fn entity_time_from_definition(&self, entity: EntityId, selector: u32) -> Result<f64, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let (space, offset, _) = state.entity_values(entity).unwrap_or((selector, 0., None));
        let group = *[3, 4, 2, 0].get(space as usize).ok_or("entity timespace invalid")?;
        Ok((state.clock.time(group).ok_or("entity clock group absent")? - offset) as f32 as f64)
    }
    pub fn entity_time(&self, entity: EntityId) -> Result<f64, &str> {
        let state = self.0.as_ref().map_err(String::as_str)?;
        let (space, offset, _) = state
            .entity_values(entity)
            .ok_or("entity clock root is not registered")?;
        let group = *[3, 4, 2, 0]
            .get(space as usize)
            .ok_or("entity timespace invalid")?;
        Ok(state.clock.time(group).ok_or("entity clock group absent")? - offset)
    }
}

impl FromWorld for ClockRuntime {
    fn from_world(_: &mut World) -> Self {
        let state = (|| {
            let (path, controls) = game_settings::read_game_clock_controls()?;
            let source = PerformanceCounter::new()?;
            let mut clock = GameClock::from_bytes([0; game_clock::STATE_SIZE]);
            clock.initialize(); // Opaque owned padding is not given field meanings.
            info!(
                "Game clock: QPC {} Hz, controls {:?} from {} (registered defaults where absent)",
                source.frequency,
                controls,
                path.display()
            );
            Ok(LoadedClock {
                clock,
                source,
                timing: FrameTimingEstimator::default(),
                controls,
                common_counter: 0,
                advanced: false,
                player_entity: None,
                player_override: None,
                player_timespace: 0,
                player_clock_offset: 0.0,
                player_time_children: Vec::new(),
                entity_clocks: HashMap::new(),
                entity_overrides: Vec::new(),
                entity_transitions: VecDeque::new(),
                entity_publication: None,
                publication_spare: None,
                power_requests: TimeRequests::default(),
            })
        })();
        if let Err(error) = &state {
            error!("Game clock unavailable: {error}");
        }
        Self(state)
    }
}

/// A coherent original gameplay-view clock snapshot. All models in one
/// publication use the same five native seconds/deltas, not per-draw QPC reads.
/// Missing owner state remains an error, never a zero or Bevy-clock fallback.
#[derive(Resource, Clone, ExtractResource)]
pub struct GameplayViewClock(Result<game_clock::GameplayViewClocks, String>);

impl GameplayViewClock {
    pub fn clocks(&self) -> Result<&game_clock::GameplayViewClocks, &str> {
        self.0.as_ref().map_err(String::as_str)
    }
    /// Actual world/view bindings, distinct from a render model's offset
    /// and selected clock. Keep the source definition byte4, not Time2->2.
    pub fn view_shader_times(
        &self,
        source_flag_04: u8,
    ) -> Result<game_clock::ViewShaderTimes, String> {
        Ok(self
            .clocks()
            .map_err(str::to_owned)?
            .view_shader_times(source_flag_04))
    }

    pub fn model_local_time(&self, offset: f32, model_flags_1c31: u8) -> Result<[f32; 2], String> {
        self.clocks()
            .map_err(str::to_owned)?
            .model_local_time(offset, model_flags_1c31)
            .ok_or_else(|| {
                format!(
                    "model clock group {} is outside the five gameplay clocks",
                    crate::render_model::model_shader_time_group(model_flags_1c31)
                )
            })
    }
}

impl FromWorld for GameplayViewClock {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource::<ClockRuntime>()
                .clock()
                .map(game_clock::GameplayViewClocks::from_clock)
                .map_err(str::to_owned),
        )
    }
}

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClockSystems {
    Advance,
    PublishView,
}

fn publish_gameplay_view_clock(owner: Res<ClockRuntime>, mut view: ResMut<GameplayViewClock>) {
    view.0 = owner
        .clock()
        .map(game_clock::GameplayViewClocks::from_clock)
        .map_err(str::to_owned);
}

/// `idGameLocal::IsPaused` (vtable slot 29, `FUN_1408a89b0`): the top game
/// logic state's decl `m_pause` (or `gameLocal+0xc`). While set, the game
/// frame renders the world without simulating it.
#[derive(Resource, Default, Clone, Copy, Debug, PartialEq)]
pub struct GamePaused(pub bool);

fn advance_clock(mut owner: ResMut<ClockRuntime>, paused: Res<GamePaused>) {
    if let Ok(state) = &mut owner.0 {
        let result = if paused.0 {
            state.paused()
        } else {
            state.advance()
        };
        if let Err(error) = result {
            error!("Game clock unavailable: {error}");
            owner.0 = Err(error);
        }
    }
}

pub struct ClockRuntimePlugin;
impl Plugin for ClockRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ClockRuntime>()
            .init_resource::<GamePaused>()
            .init_resource::<GameplayViewClock>()
            .add_plugins(ExtractResourcePlugin::<GameplayViewClock>::default())
            .add_systems(First, advance_clock.in_set(ClockSystems::Advance))
            .add_systems(
                PostUpdate,
                publish_gameplay_view_clock.in_set(ClockSystems::PublishView),
            );
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[test]
    fn loaded_clock_owner_uses_actual_qpc_without_window_or_bevy_elapsed() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ClockRuntimePlugin);
        app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::ZERO,
        ));
        app.update();
        let owner = app.world().resource::<ClockRuntime>();
        let before = owner.clock().unwrap().time(3).unwrap();
        assert!(owner.advanced());
        std::thread::sleep(std::time::Duration::from_millis(20));
        app.update();
        let owner = app.world().resource::<ClockRuntime>();
        let clock = owner.clock().unwrap();
        assert!(clock.time(3).unwrap() > before);
        assert!(clock.delta(3).unwrap() > 0.0);
        assert!(owner.advanced());
        let view = app.world().resource::<GameplayViewClock>();
        for group in 0..game_clock::GROUP_COUNT {
            let actual = view.model_local_time(0.5, (group as u8) << 4).unwrap();
            let expected = crate::render_model::model_shader_local_time(
                0.5,
                clock.time(group).unwrap(),
                clock.delta(group).unwrap(),
            );
            assert_eq!(actual.map(f32::to_bits), expected.map(f32::to_bits));
        }
        assert!(view.model_local_time(0.0, 0x50).is_err());
        assert_eq!(
            app.world().resource::<Time>().elapsed(),
            std::time::Duration::ZERO
        );
    }

    #[test]
    fn gameplay_view_clock_preserves_unavailable_owner() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ClockRuntimePlugin);
        app.world_mut()
            .insert_resource(ClockRuntime(Err("test unavailable native clock".into())));
        app.update();
        let view = app.world().resource::<GameplayViewClock>();
        assert_eq!(view.clocks().unwrap_err(), "test unavailable native clock");
        assert_eq!(
            view.view_shader_times(game_clock::RESET_VIEW_LOCAL_TIME_FLAG)
                .unwrap_err(),
            "test unavailable native clock"
        );
        assert_eq!(
            view.model_local_time(0.0, 0x30).unwrap_err(),
            "test unavailable native clock"
        );
    }

    #[test]
    #[ignore = "requires a graphics adapter; verifies native main-world publication/render-world extraction"]
    fn gameplay_view_clock_is_extracted_after_publication() {
        use bevy::render::RenderApp;
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(bevy::window::WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                })
                .disable::<bevy::winit::WinitPlugin>()
                .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
        )
        .add_plugins(ClockRuntimePlugin);
        app.finish();
        app.cleanup();
        app.update();
        let main = app
            .world()
            .resource::<GameplayViewClock>()
            .clocks()
            .unwrap();
        let render = app
            .sub_app(RenderApp)
            .world()
            .resource::<GameplayViewClock>()
            .clocks()
            .unwrap();
        let mut main_bytes = [0; 0x3c];
        let mut render_bytes = [0; 0x3c];
        main.write_game_result(&mut main_bytes).unwrap();
        render.write_game_result(&mut render_bytes).unwrap();
        assert_eq!(main_bytes, render_bytes);
        let game = app.world().resource::<ClockRuntime>().clock().unwrap();
        for group in 0..game_clock::GROUP_COUNT {
            assert_eq!(
                &main_bytes[group * 8..group * 8 + 8],
                &game.time(group).unwrap().to_le_bytes()
            );
            assert_eq!(
                &main_bytes[0x28 + group * 4..0x2c + group * 4],
                &game.delta(group).unwrap().to_le_bytes()
            );
        }
    }
}
