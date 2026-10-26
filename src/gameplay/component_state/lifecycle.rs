//! Component initialization/start batches7ea340, preserving callback mutation.
use super::{entity::ComponentHandle, ComponentOwner, ComponentRegistry};
use crate::storage::PointerList;
use std::collections::HashMap;

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

fn sort_batch(list: &mut PointerList, world: &impl ComponentLifecycleWorld) -> Result<(), String> {
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
        self.request_stop(component, world)?;
        let phase = world.owner(component)?.flags & 3;
        if phase != 3 {
            if phase == 1 && self.cancel_initialization(component.packed() as usize)? {
                world.shutdown(component, self)?;
                world.released(component, self)?;
            } else {
                world.owner_mut(component)?.flags |= 3;
                self.pending_release.append(component.packed(), false)?;
                self.maximum_registered = self
                    .maximum_registered
                    .max(self.pending_release.count() as i32);
            }
        }
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
        let identity = component.packed() as usize;
        let phase = (world.owner(component)?.flags >> 2) & 3;
        if phase != 3 {
            if phase == 1 && self.cancel_pending_start(identity)? {
                world.owner_mut(component)?.flags &= !12;
            }
            let flags = world.owner(component)?.flags;
            if flags & 12 == 8 {
                world.owner_mut(component)?.flags = flags | 12;
                if flags & 0x1000_0000 == 0 {
                    self.pending_stop.append(component.packed(), false)?;
                    self.maximum_registered = self
                        .maximum_registered
                        .max(self.pending_stop.count() as i32);
                } else {
                    self.cancel_pending_start(identity)?;
                    world.owner_mut(component)?.flags &= !0x1000_0000;
                }
                world.stop_queued(component, self)?;
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
        let mut deferred = Vec::new();
        while self.registered.count() != 0 || self.pending_start.count() != 0 {
            while self.registered.count() != 0 {
                self.processed_initialization_count = self.registered.count() as i32;
                sort_batch(&mut self.registered, world)?;
                let mut index = 0;
                while index < self.processed_initialization_count {
                    let value = *self
                        .registered
                        .values()
                        .get(index as usize)
                        .ok_or("initialization cursor exceeds component queue")?;
                    let component = ComponentHandle::from_packed(value)?;
                    world.initialize(component, self)?;
                    if world.owner(component)?.flags & 3 < 2 {
                        self.request_stop(component, world)?;
                        world.initialization_failed(component, self)?;
                    }
                    index += 1;
                }
                if self
                    .registered
                    .finish_prefix(self.processed_initialization_count as u32, 64)?
                {
                    self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
                }
                self.processed_initialization_count = 0;
            }
            while self.pending_start.count() != 0 {
                self.processed_start_count = self.pending_start.count() as i32;
                sort_batch(&mut self.pending_start, world)?;
                let mut index = 0;
                while index < self.processed_start_count {
                    let value = *self
                        .pending_start
                        .values()
                        .get(index as usize)
                        .ok_or("start cursor exceeds component queue")?;
                    let component = ComponentHandle::from_packed(value)?;
                    if world.owner(component)?.flags & 3 == 1 {
                        deferred.push(value);
                    } else {
                        world.start(component, self)?;
                    }
                    index += 1;
                }
                if self
                    .pending_start
                    .finish_prefix(self.processed_start_count as u32, 64)?
                {
                    self.queue_reset_count = self.queue_reset_count.wrapping_add(1);
                }
                self.processed_start_count = 0;
            }
            for (index, value) in deferred.drain(..).enumerate() {
                self.pending_start.insert(index as u32, value, false)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
