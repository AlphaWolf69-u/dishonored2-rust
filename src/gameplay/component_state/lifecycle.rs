//! Component initialization/start batches7ea340, preserving callback mutation.
use super::{entity::ComponentHandle, ComponentOwner, ComponentRegistry};
use crate::storage::PointerList;
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
enum ReleasePhase { #[default] Idle, Stop, StopPoll, StopFinish, ReleasePreflight, Deinitialize, Release, ReleaseFinish, Restore }

struct ImmediateRelease { component: ComponentHandle, generation: u32, shutdown_done: bool, running: bool }
struct PendingStopCallback { component: ComponentHandle, generation: u32, running: bool }
struct PendingStopCompletion { component: ComponentHandle, generation: u32, complete: bool }

/// A callback that is waiting for an external owner retains its queue cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseStatus { Pending, Complete }

#[derive(Default)]
pub(crate) struct ReleaseProgress {
    phase: ReleasePhase,
    count: u32,
    next: u32,
    deferred: Vec<u64>,
    restore_next: usize,
    prefer_release: bool,
    running: bool,
    immediate: Vec<ImmediateRelease>,
    stop_callbacks: Vec<PendingStopCallback>,
    waiting_stop: Option<(ComponentHandle, u32)>,
    // Host: independent backend acknowledgements retain only identities.
    // Capacity survives batches; completed callbacks are never replayed.
    pending_stops: Vec<PendingStopCompletion>,
    stop_poll_next: usize,
}

#[derive(Default)]
pub(crate) struct StartupProgress {
    initialization_active: bool,
    initialization_next: u32,
    initialization_step: u8,
    start_loop: bool,
    start_active: bool,
    start_next: u32,
    deferred: Vec<u64>,
    restore_next: usize,
    restoring: bool,
}

/// Each method dispatches through the same live typed component pools.
/// The priority is virtual80's factory initialization rank, not declaration LOD.
pub trait ComponentLifecycleWorld {
    fn priority(&self, component: ComponentHandle) -> Result<i32, String>;
    fn owner(&self, component: ComponentHandle) -> Result<&ComponentOwner, String>;
    fn owner_mut(&mut self, component: ComponentHandle) -> Result<&mut ComponentOwner, String>;
    fn initialize(
        &mut self,
        component: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String>;
    fn start(
        &mut self,
        component: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String>;
    /// Virtual40 follows the stop-queue transition, even for callbacks that
    /// change flags or append registrations while this batch is being processed.
    fn stop_queued(
        &mut self,
        component: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String>;
    fn initialization_failed(
        &mut self,
        component: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String>;
}

/// Release7e72c0 invokes the type's shutdown and release callbacks, separately
/// from the failed-initialization callback used by batch dispatch.
pub trait ComponentReleaseWorld: ComponentLifecycleWorld {
    /// 8101f0 dispatches queued entity events and may append lifecycle work.
    fn pump_events(&mut self, _registry: &mut ComponentRegistry) -> Result<(), String> {
        Err("component teardown requires the shared entity event owner".into())
    }
    /// Virtual30 completes a queued stop before deinitialization.
    fn stop(&mut self, component: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String>;
    /// `first` is true only on the initial callback entry. Later polls
    /// retain the same handle and generation and must not repeat its request.
    fn poll_stop(&mut self, component: ComponentHandle, registry: &mut ComponentRegistry,
        _first: bool) -> Result<ReleaseStatus, String> {
        self.stop(component, registry)?;
        Ok(ReleaseStatus::Complete)
    }
    /// Virtual20 deinitializes an initialized component and unlinks it.
    fn deinitialize(&mut self, component: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String>;
    fn shutdown(
        &mut self,
        component: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String>;
    fn released(
        &mut self,
        component: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String>;
}

pub(super) fn sort_batch(list: &mut PointerList, world: &impl ComponentLifecycleWorld) -> Result<(), String> {
    // All understood virtual80 methods are pure factory-rank getters. Capture
    // these once without copying or moving any component out of its real pool.
    let ranks = list
        .values()
        .iter()
        .map(|&value| {
            world
                .priority(ComponentHandle::from_packed(value)?)
                .map(|rank| (value, rank))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;
    crate::sorting::quick_sort(list.values_mut()?, |a, b| ranks[b].wrapping_sub(ranks[a]));
    Ok(())
}

impl ComponentRegistry {
    /// The world shutdown preflight runs only before a new drain, not while
    /// an earlier failed batch or immediate callback is being resumed.
    pub(crate) fn release_idle(&self) -> bool {
        matches!(self.release_progress.phase, ReleasePhase::Idle)
            && self.scoped_release.is_none() && !self.scoped_release_running
            && !self.release_progress.running && self.release_progress.immediate.is_empty()
            && self.release_progress.stop_callbacks.is_empty()
            && self.release_progress.deferred.is_empty()
            && self.release_progress.pending_stops.is_empty() && self.release_progress.waiting_stop.is_none()
    }

    pub(crate) fn startup_idle(&self) -> bool {
        !self.startup_progress.initialization_active && !self.startup_progress.start_loop
            && !self.startup_progress.start_active && !self.startup_progress.restoring
            && self.startup_progress.deferred.is_empty()
    }

    /// Host cancellation must finish the entered callback's world cursor
    /// before another component can request body publication/destruction.
    pub(crate) fn interrupted_startup_component(&self) -> Result<Option<ComponentHandle>, String> {
        let value = if self.startup_progress.initialization_active {
            self.registered.values().get(self.startup_progress.initialization_next as usize)
        } else if self.startup_progress.start_active {
            self.pending_start.values().get(self.startup_progress.start_next as usize)
        } else { None };
        value.copied().map(ComponentHandle::from_packed).transpose()
    }

    /// 7ea870: stops precede releases; each sorted batch is visited in reverse.
    /// Deinitialization and destruction are separate passes, since callbacks
    /// may still refer to other components in the same release batch. The
    /// shared event owner must be connected before this drain handles live
    /// entity callbacks: 7ea870 pumps 8101f0 after each finished batch.
    pub fn stop_and_release(&mut self, world: &mut impl ComponentReleaseWorld) -> Result<(), String> {
        match self.poll_stop_and_release(world)? {
            ReleaseStatus::Complete => Ok(()),
            ReleaseStatus::Pending => Err("component release awaits asynchronous Stop completion".into()),
        }
    }

    pub fn poll_stop_and_release(&mut self, world: &mut impl ComponentReleaseWorld) -> Result<ReleaseStatus, String> {
        if self.scoped_release.is_some() || self.scoped_release_running {
            return Err("entity-scoped component release is active".into());
        }
        if !self.release_progress.stop_callbacks.is_empty() {
            return Err("component stop callback must finish before release drain".into());
        }
        if self.release_progress.running { return Err("component release drain is already active".into()); }
        self.release_progress.running = true;
        let result = self.stop_and_release_inner(world);
        self.release_progress.running = false;
        result
    }

    fn stop_and_release_inner(&mut self, world: &mut impl ComponentReleaseWorld) -> Result<ReleaseStatus, String> {
        loop {
            match self.release_progress.phase {
                ReleasePhase::Idle => {
                    if self.release_events_pending {
                        world.pump_events(self)?;
                        self.release_events_pending = false;
                    }
                    if self.release_progress.prefer_release && self.pending_release.count() == 0 {
                        if !self.release_progress.deferred.is_empty() {
                            self.release_progress.restore_next = 0;
                            self.release_progress.phase = ReleasePhase::Restore;
                            continue;
                        }
                        self.release_progress.prefer_release = false;
                    }
                    if !self.release_progress.prefer_release && self.pending_stop.count() != 0 {
                        let count = self.pending_stop.count();
                        self.processed_stop_count = count as i32;
                        sort_batch(&mut self.pending_stop, world)?;
                        // Reserve before entering a callback: recording Pending
                        // must never fail after its backend request was accepted.
                        self.release_progress.pending_stops.try_reserve(count as usize)
                            .map_err(|error| format!("component Stop acknowledgement batch: {error}"))?;
                        self.release_progress.count = count;
                        self.release_progress.next = count;
                        self.release_progress.phase = ReleasePhase::Stop;
                    } else if self.pending_release.count() != 0 {
                        let count = self.pending_release.count();
                        sort_batch(&mut self.pending_release, world)?;
                        self.release_progress.count = count;
                        self.release_progress.next = count;
                        self.release_progress.phase = ReleasePhase::ReleasePreflight;
                    } else if self.pending_stop.count() != 0 {
                        self.release_progress.prefer_release = false;
                    } else { return Ok(ReleaseStatus::Complete); }
                }
                ReleasePhase::Stop => {
                    if self.release_progress.next == 0 {
                        if self.release_progress.pending_stops.is_empty() {
                            self.release_progress.phase = ReleasePhase::StopFinish;
                            continue;
                        }
                        // Native7ea870 issues every Stop in the captured
                        // reverse-sorted prefix before its destruction pass.
                        // Backend acknowledgements suspend that barrier, not
                        // admission of unrelated Stop requests in this batch.
                        self.release_progress.stop_poll_next = 0;
                        self.release_progress.phase = ReleasePhase::StopPoll;
                        return Ok(ReleaseStatus::Pending);
                    }
                    let index = self.release_progress.next as usize - 1;
                    let component = ComponentHandle::from_packed(*self.pending_stop.values().get(index).ok_or("stop batch changed during release")?)?;
                    let generation = world.owner(component)?.generation;
                    let first = match self.release_progress.waiting_stop {
                        None => true,
                        Some(identity) if identity == (component, generation) => false,
                        Some(_) => return Err("pending Stop component identity changed".into()),
                    };
                    self.release_progress.waiting_stop = Some((component, generation));
                    match world.poll_stop(component, self, first)? {
                        ReleaseStatus::Pending => self.release_progress.pending_stops.push(PendingStopCompletion {
                            component, generation, complete: false,
                        }),
                        ReleaseStatus::Complete => {},
                    }
                    self.release_progress.waiting_stop = None;
                    self.release_progress.next -= 1;
                }
                ReleasePhase::StopPoll => {
                    while self.release_progress.stop_poll_next < self.release_progress.pending_stops.len() {
                        let index = self.release_progress.stop_poll_next;
                        let pending = &self.release_progress.pending_stops[index];
                        if !pending.complete {
                            let (component, generation) = (pending.component, pending.generation);
                            if world.owner(component)?.generation != generation {
                                return Err("pending Stop acknowledgement generation changed".into());
                            }
                            if world.poll_stop(component, self, false)? == ReleaseStatus::Complete {
                                self.release_progress.pending_stops[index].complete = true;
                            }
                        }
                        self.release_progress.stop_poll_next += 1;
                    }
                    self.release_progress.stop_poll_next = 0;
                    if self.release_progress.pending_stops.iter().any(|pending| !pending.complete) {
                        return Ok(ReleaseStatus::Pending);
                    }
                    self.release_progress.pending_stops.clear();
                    self.release_progress.phase = ReleasePhase::StopFinish;
                }
                ReleasePhase::StopFinish => {
                    if self.pending_stop.finish_prefix(self.release_progress.count, 64)? {
                        self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
                    }
                    self.processed_stop_count = 0;
                    self.release_events_pending = true;
                    self.release_progress.phase = ReleasePhase::Idle;
                }
                ReleasePhase::ReleasePreflight => {
                    for &value in self.pending_release.values().iter().take(self.release_progress.count as usize) {
                        let component = ComponentHandle::from_packed(value)?;
                        if world.owner(component)?.flags & 12 != 12 { self.ensure_no_update_references(component)?; }
                    }
                    self.release_progress.phase = ReleasePhase::Deinitialize;
                }
                ReleasePhase::Deinitialize => {
                    if self.release_progress.next == 0 {
                        self.release_progress.next = self.release_progress.count;
                        self.release_progress.phase = ReleasePhase::Release;
                        continue;
                    }
                    let index = self.release_progress.next as usize - 1;
                    let value = *self.pending_release.values().get(index).ok_or("release batch changed during deinitialization")?;
                    let component = ComponentHandle::from_packed(value)?;
                    if world.owner(component)?.flags & 12 == 12 {
                        self.release_progress.deferred.try_reserve(1).map_err(|error| format!("deferred component release: {error}"))?;
                        self.release_progress.deferred.push(value);
                    } else { world.deinitialize(component, self)?; }
                    self.release_progress.next -= 1;
                }
                ReleasePhase::Release => {
                    if self.release_progress.next == 0 { self.release_progress.phase = ReleasePhase::ReleaseFinish; continue; }
                    let index = self.release_progress.next as usize - 1;
                    let value = *self.pending_release.values().get(index).ok_or("release batch changed during destruction")?;
                    let component = ComponentHandle::from_packed(value)?;
                    if world.owner(component)?.flags & 3 < 2 {
                        self.ensure_no_update_references(component)?;
                        world.released(component, self)?;
                    }
                    self.release_progress.next -= 1;
                }
                ReleasePhase::ReleaseFinish => {
                    if self.pending_release.finish_prefix(self.release_progress.count, 64)? {
                        self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
                    }
                    self.release_events_pending = true;
                    self.release_progress.prefer_release = true;
                    self.release_progress.phase = ReleasePhase::Idle;
                }
                ReleasePhase::Restore => {
                    if self.release_progress.restore_next < self.release_progress.deferred.len() {
                        let index = self.release_progress.restore_next;
                        let value = self.release_progress.deferred[index];
                        self.pending_release.insert_preserving(index as u32, value, false)?;
                        self.release_progress.restore_next += 1;
                        continue;
                    }
                    self.release_progress.deferred.clear();
                    self.release_progress.restore_next = 0;
                    self.release_progress.prefer_release = false;
                    self.release_progress.phase = ReleasePhase::Idle;
                    if self.pending_stop.count() == 0 {
                        return Err("component release is waiting for an unhosted stop/event transition".into());
                    }
                }
            }
        }
    }
    ///7ec370, including the source's signed missing-entry/cursor comparison.
    pub fn cancel_initialization(&mut self, component: usize) -> Result<bool, String> {
        let index = self
            .registered
            .values()
            .iter()
            .position(|&v| v == component as u64)
            .map_or(-1, |i| i as i32);
        if self.processed_initialization_count <= index {
            self.registered.remove_at(index)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    ///7e72c0 stops first, then either cancels an unprocessed registration and
    /// releases immediately, or appends the component to the release queue.
    pub fn request_release(
        &mut self,
        component: ComponentHandle,
        world: &mut impl ComponentReleaseWorld,
    ) -> Result<(), String> {
        if self.release_progress.stop_callbacks.iter().any(|entry| entry.component == component && entry.running) {
            return Err("component stop callback must finish before release".into());
        }
        if self.release_progress.immediate.iter().any(|entry| entry.component == component) {
            return self.resume_immediate_release(component, world);
        }
        self.request_stop(component, world)?;
        let phase = world.owner(component)?.flags & 3;
        if phase != 3 {
            if phase == 1 { self.ensure_no_update_references(component)?; }
            if phase == 1 {
                // Reserve retry ownership before canceling initialization.
                self.release_progress.immediate.try_reserve(1)
                    .map_err(|error| format!("immediate component release: {error}"))?;
            }
            if phase == 1 && self.cancel_initialization(component.packed() as usize)? {
                let generation = world.owner(component)?.generation;
                self.release_progress.immediate.push(ImmediateRelease { component, generation, shutdown_done: false, running: false });
                self.resume_immediate_release(component, world)?;
            } else {
                let previous_flags = world.owner(component)?.flags;
                world.owner_mut(component)?.flags = previous_flags | 3;
                let previous_count = self.pending_release.count();
                match self.pending_release.append_preserving(component.packed(), false) {
                    Ok(index) if index == previous_count && self.pending_release.count() == previous_count + 1 => {}
                    Ok(_) => {
                        world.owner_mut(component)?.flags = previous_flags;
                        return Err("component release queue rejected the append".into());
                    }
                    Err(error) => {
                        world.owner_mut(component)?.flags = previous_flags;
                        return Err(error);
                    }
                }
                self.maximum_registered = self
                    .maximum_registered
                    .max(self.pending_release.count() as i32);
            }
        }
        Ok(())
    }

    fn resume_immediate_release(&mut self, component: ComponentHandle,
        world: &mut impl ComponentReleaseWorld) -> Result<(), String> {
        let position = self.release_progress.immediate.iter().position(|entry| entry.component == component)
            .ok_or("immediate component release progress absent")?;
        let generation = self.release_progress.immediate[position].generation;
        if self.release_progress.immediate[position].running { return Err("immediate component release is already active".into()); }
        self.release_progress.immediate[position].running = true;
        let result = self.resume_immediate_release_inner(component, generation, world);
        if let Some(entry) = self.release_progress.immediate.iter_mut().find(|entry|
            entry.component == component && entry.generation == generation) { entry.running = false; }
        result
    }

    fn resume_immediate_release_inner(&mut self, component: ComponentHandle, generation: u32,
        world: &mut impl ComponentReleaseWorld) -> Result<(), String> {
        let position = self.release_progress.immediate.iter().position(|entry|
            entry.component == component && entry.generation == generation)
            .ok_or("immediate component release progress absent")?;
        if world.owner(component)?.generation != generation { return Err("stale immediate component release".into()); }
        if !self.release_progress.immediate[position].shutdown_done {
            world.shutdown(component, self)?;
            let position = self.release_progress.immediate.iter().position(|entry|
                entry.component == component && entry.generation == generation)
                .ok_or("immediate component release progress disappeared")?;
            self.release_progress.immediate[position].shutdown_done = true;
        }
        world.released(component, self)?;
        let position = self.release_progress.immediate.iter().position(|entry|
            entry.component == component && entry.generation == generation)
            .ok_or("immediate component release progress disappeared")?;
        self.release_progress.immediate.remove(position);
        Ok(())
    }
    ///7ec4c0, including the source's missing-entry (-1) cursor comparison.
    pub fn cancel_pending_start(&mut self, component: usize) -> Result<bool, String> {
        let index = self
            .pending_start
            .values()
            .iter()
            .position(|&v| v == component as u64)
            .map_or(-1, |i| i as i32);
        if self.processed_start_count <= index {
            self.pending_start.remove_at(index)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    ///7e7c00 uses the actual owner flags again after cancellation callbacks.
    /// Exclusive level ownership replaces the source's spinlock only.
    pub fn request_stop(
        &mut self,
        component: ComponentHandle,
        world: &mut impl ComponentLifecycleWorld,
    ) -> Result<(), String> {
        if let Some(index) = self.release_progress.stop_callbacks.iter().position(|entry| entry.component == component) {
            if self.release_progress.stop_callbacks[index].running {
                // The source has already set the stopped phase before the
                // callback, so a nested request for this component is a no-op.
                return Ok(());
            }
            if world.owner(component)?.generation != self.release_progress.stop_callbacks[index].generation {
                return Err("stale component stop callback".into());
            }
            self.release_progress.stop_callbacks[index].running = true;
            let result = world.stop_queued(component, self);
            let index = self.release_progress.stop_callbacks.iter().position(|entry| entry.component == component)
                .ok_or("component stop callback ownership was lost")?;
            if result.is_ok() { self.release_progress.stop_callbacks.remove(index); }
            else { self.release_progress.stop_callbacks[index].running = false; }
            return result;
        }
        let identity = component.packed() as usize;
        let phase = (world.owner(component)?.flags >> 2) & 3;
        if phase != 3 {
            if phase == 1 && self.cancel_pending_start(identity)? {
                world.owner_mut(component)?.flags &= !12;
            }
            let flags = world.owner(component)?.flags;
            if flags & 12 == 8 {
                self.release_progress.stop_callbacks.try_reserve(1)
                    .map_err(|error| format!("component stop callback: {error}"))?;
                world.owner_mut(component)?.flags = flags | 12;
                if flags & 0x1000_0000 == 0 {
                    let previous_count = self.pending_stop.count();
                    match self.pending_stop.append_preserving(component.packed(), false) {
                        Ok(index) if index == previous_count && self.pending_stop.count() == previous_count + 1 => {}
                        Ok(_) => {
                            world.owner_mut(component)?.flags = flags;
                            return Err("component stop queue rejected the append".into());
                        }
                        Err(error) => {
                            world.owner_mut(component)?.flags = flags;
                            return Err(error);
                        }
                    }
                    self.maximum_registered = self
                        .maximum_registered
                        .max(self.pending_stop.count() as i32);
                } else {
                    self.cancel_pending_start(identity)?;
                    world.owner_mut(component)?.flags &= !0x1000_0000;
                }
                let generation = world.owner(component)?.generation;
                self.release_progress.stop_callbacks.push(PendingStopCallback { component, generation, running: false });
                return self.request_stop(component, world);
            }
        }
        Ok(())
    }

    ///7ea340 drains registrations before startup, sorts each newly observed
    /// batch, and defers starts that register before their initialization pass.
    pub fn initialize_and_start(
        &mut self,
        world: &mut impl ComponentLifecycleWorld,
    ) -> Result<(), String> {
        loop {
            if self.startup_progress.restoring {
                if self.startup_progress.restore_next < self.startup_progress.deferred.len() {
                    let index = self.startup_progress.restore_next;
                    let value = self.startup_progress.deferred[index];
                    self.pending_start.insert_preserving(index as u32, value, false)?;
                    self.startup_progress.restore_next += 1;
                    continue;
                }
                let stalled = !self.startup_progress.deferred.is_empty() && self.registered.count() == 0;
                self.startup_progress.deferred.clear();
                self.startup_progress.restore_next = 0;
                self.startup_progress.restoring = false;
                self.startup_progress.start_loop = false;
                if stalled { return Err("component start is waiting for an unhosted initialization transition".into()); }
            }
            if !self.startup_progress.start_loop {
                self.initialize_pending(world)?;
                if self.pending_start.count() == 0 { return Ok(()); }
                self.startup_progress.start_loop = true;
            }
            if !self.startup_progress.start_active {
                if self.pending_start.count() == 0 {
                    self.startup_progress.restoring = true;
                    continue;
                }
                self.processed_start_count = self.pending_start.count() as i32;
                sort_batch(&mut self.pending_start, world)?;
                self.startup_progress.start_next = 0;
                self.startup_progress.start_active = true;
            }
            if self.startup_progress.start_next < self.processed_start_count as u32 {
                let value = *self.pending_start.values().get(self.startup_progress.start_next as usize)
                    .ok_or("start cursor exceeds component queue")?;
                let component = ComponentHandle::from_packed(value)?;
                if world.owner(component)?.flags & 3 == 1 {
                    self.startup_progress.deferred.try_reserve(1).map_err(|error| format!("deferred component start: {error}"))?;
                    self.startup_progress.deferred.push(value);
                } else { world.start(component, self)?; }
                self.startup_progress.start_next += 1;
                continue;
            }
            if self.pending_start.finish_prefix(self.processed_start_count as u32, 64)? {
                self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
            }
            self.processed_start_count = 0;
            self.startup_progress.start_active = false;
        }
    }

    /// Drain the native initialization queue while leaving the separate start
    /// queue owned by the registry until its callback host is available.
    pub fn initialize_pending(&mut self, world: &mut impl ComponentLifecycleWorld) -> Result<(), String> {
        while self.startup_progress.initialization_active || self.registered.count() != 0 {
            if !self.startup_progress.initialization_active {
                self.processed_initialization_count = self.registered.count() as i32;
                sort_batch(&mut self.registered, world)?;
                self.startup_progress.initialization_next = 0;
                self.startup_progress.initialization_step = 0;
                self.startup_progress.initialization_active = true;
            }
            while self.startup_progress.initialization_next < self.processed_initialization_count as u32 {
                let value = *self.registered.values().get(self.startup_progress.initialization_next as usize)
                    .ok_or("initialization cursor exceeds component queue")?;
                let component = ComponentHandle::from_packed(value)?;
                if self.startup_progress.initialization_step == 0 {
                    world.initialize(component, self)?;
                    self.startup_progress.initialization_step = 1;
                }
                if self.startup_progress.initialization_step == 1 {
                    if world.owner(component)?.flags & 3 < 2 {
                        self.request_stop(component, world)?;
                        self.startup_progress.initialization_step = 2;
                    } else { self.startup_progress.initialization_step = 3; }
                }
                if self.startup_progress.initialization_step == 2 {
                    world.initialization_failed(component, self)?;
                    self.startup_progress.initialization_step = 3;
                }
                self.startup_progress.initialization_next += 1;
                self.startup_progress.initialization_step = 0;
            }
            if self.registered.finish_prefix(self.processed_initialization_count as u32, 64)? {
                self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
            }
            self.processed_initialization_count = 0;
            self.startup_progress.initialization_active = false;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

mod startup_abort;
