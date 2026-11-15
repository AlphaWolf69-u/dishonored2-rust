//! bevy_backend: unpublished scene command batches own their reserved ECS IDs.
//!
//! Bevy0.19 RemoteAllocator::alloc reserves IDs but has no return-on-drop.
//! EntityAllocator::free is the supported return path. Workers cannot borrow
//! World, so cancellation hands reservations back to that exact world's owner.
//! Prediction: only discarded, unpublished batch IDs return; rendered output,
//! active identities and installed resource lifetimes do not change. No timing
//! or byte saving is claimed without measurements.
use bevy::{ecs::{entity::RemoteAllocator, world::WorldId}, prelude::*};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};

#[derive(Resource)]
struct Returns {
    world: WorldId,
    queue: Arc<ReturnQueue>,
}
impl FromWorld for Returns {
    fn from_world(world: &mut World) -> Self {
        Self { world: world.id(), queue: Arc::new(ReturnQueue::default()) }
    }
}
#[derive(Default)]
struct ReturnQueue {
    pending: AtomicBool,
    batches: Mutex<Vec<Cancelled>>,
}
struct Cancelled {
    allocator: RemoteAllocator,
    ids: Vec<Entity>,
}

/// One command queue's reservations, never a separate entity allocator.
/// The publication cursor follows the queue's FIFO spawn order. A partially
/// applied queue therefore returns only its unacknowledged suffix, even if a
/// successfully spawned prefix was subsequently despawned by another command.
pub(crate) struct Reservations {
    world: WorldId,
    allocator: RemoteAllocator,
    returns: Arc<ReturnQueue>,
    ids: Vec<Entity>,
    published: Arc<AtomicUsize>,
}
impl Reservations {
    pub(crate) fn new(world: &World) -> Self {
        let owner = world.resource::<Returns>();
        assert_eq!(owner.world, world.id(), "scene reservation owner belongs to another World");
        Self { world: world.id(), allocator: world.entity_allocator().build_remote_allocator(),
            returns: owner.queue.clone(), ids: Vec::new(), published: Arc::new(AtomicUsize::new(0)) }
    }
    pub(crate) fn reserve(&mut self) -> (Entity, Receipt) {
        assert!(!self.allocator.is_closed(), "scene preparation outlived its entity allocator");
        // Allocate tracking capacity before taking an ID from Bevy's free list.
        self.ids.reserve(1);
        let id = self.allocator.alloc();
        self.ids.push(id);
        (id, Receipt { through: self.ids.len(), published: self.published.clone() })
    }
    pub(crate) fn validate(&self, world: &World) {
        assert_eq!(self.world, world.id(), "scene command publication changed World");
        assert!(world.entity_allocator().has_remote_allocator(&self.allocator),
            "scene command publication outlived its entity allocator");
    }
}
impl Drop for Reservations {
    fn drop(&mut self) {
        let published = self.published.load(Ordering::Acquire);
        debug_assert!(published <= self.ids.len());
        if published >= self.ids.len() { return; }
        let mut ids = std::mem::take(&mut self.ids);
        drop(ids.drain(..published));
        let mut batches = self.returns.batches.lock().unwrap_or_else(|error| error.into_inner());
        batches.push(Cancelled { allocator: self.allocator.clone(), ids });
        self.returns.pending.store(true, Ordering::Release);
    }
}

pub(crate) struct Receipt {
    through: usize,
    published: Arc<AtomicUsize>,
}
impl Receipt {
    /// Called only after the actual World::spawn_at succeeded. The single
    /// command queue applies spawn closures serially, in reservation order.
    pub(crate) fn published(self) { self.published.store(self.through, Ordering::Release); }
}

pub(crate) fn install(app: &mut App) {
    app.init_resource::<Returns>().add_systems(Last, drain_returns);
}
/// Independent windowless acquisition hosts use the same per-world owner.
/// Initialize before immutable allocator capture and lifetime-baseline snapshots.
pub(crate) fn initialize(world: &mut World) {
    world.init_resource::<Returns>();
    drain(world);
}
fn drain_returns(world: &mut World) { drain(world); }

/// Run after cancelled workers/queues have dropped and before world teardown.
/// Allocator resets invalidate a former generation; their IDs must never enter
/// the new allocator's free list. No ID from another World can be returned.
pub(crate) fn drain(world: &mut World) -> usize {
    let Some(owner) = world.get_resource::<Returns>() else { return 0 };
    assert_eq!(owner.world, world.id(), "scene reservation return changed World");
    if !owner.queue.pending.load(Ordering::Acquire) { return 0; }
    let batches = {
        let mut batches = owner.queue.batches.lock().unwrap_or_else(|error| error.into_inner());
        owner.queue.pending.store(false, Ordering::Release);
        std::mem::take(&mut *batches)
    };
    let mut returned = 0;
    for batch in batches {
        if !world.entity_allocator().has_remote_allocator(&batch.allocator) { continue; }
        for id in batch.ids {
            // A bundle hook can unwind after publishing but before its receipt.
            // Never return an occupied index, nor an already retired generation.
            // contains() alone is unsuitable: Bevy includes unspawned IDs there.
            if world.entities().is_index_spawned(id.index())
                || world.entities().resolve_from_index(id.index()) != id {
                error!("Unacknowledged scene reservation changed during publication: {id:?}");
                continue;
            }
            world.entity_allocator_mut().free(id);
            returned += 1;
        }
    }
    returned
}
