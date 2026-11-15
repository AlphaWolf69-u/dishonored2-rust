//! Per-entity release request ordering from 1408791f0.
//!
//! This is progress for an entity selected by the existing game-local owner,
//! not a second entity queue. That owner requests every entity first, drains
//! the shared component/event batch, then retires each entity.
use super::{component_state::entity::ComponentHandle, entity_lifetime::EntityId};

/// The live owners provide their actual ordered attachment and component
/// lists. Missing hierarchy/callback ownership is an error, not an empty list.
/// A callback returning an error must be safe to retry for the same identity.
pub trait EntityReleaseWorld {
    fn child_count(&self, entity: EntityId) -> Result<usize, String>;
    fn child_at(&self, entity: EntityId, index: usize) -> Result<EntityId, String>;
    fn detach_child(&mut self, entity: EntityId, child: EntityId) -> Result<(), String>;
    fn component_count(&self, entity: EntityId) -> Result<usize, String>;
    fn component_at(&self, entity: EntityId, index: usize) -> Result<(ComponentHandle, u32), String>;
    fn request_component_release(&mut self, entity: EntityId, component: ComponentHandle, generation: u32) -> Result<(), String>;
    /// Called only after the scene's shared component/event drain. The host
    /// verifies attachments, component and graph owners are actually detached.
    fn retire_entity(&mut self, entity: EntityId) -> Result<(), String>;
    /// Host audio/graph retirement may complete on a subsequent frame.
    fn poll_retire_entity(&mut self, entity: EntityId) -> Result<bool, String> {
        self.retire_entity(entity)?;
        Ok(true)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    BeginChildren, Children(usize), DetachChild { remaining: usize, child: EntityId },
    BeginComponents, Components(usize), RequestComponent { remaining: usize, component: ComponentHandle, generation: u32 },
    Requested, Retired,
}

pub struct EntityReleaseRequest {
    entity: EntityId,
    phase: Phase,
}

impl EntityReleaseRequest {
    pub fn new(entity: EntityId) -> Self { Self { entity, phase: Phase::BeginChildren } }
    fn for_world_shutdown(entity: EntityId) -> Self { Self { entity, phase: Phase::BeginComponents } }
    pub fn entity(&self) -> EntityId { self.entity }

    /// 8791f0 visits child attachments in reverse first, then requests each
    /// live component in reverse. The saved cursor advances only on success.
    pub fn request(&mut self, world: &mut impl EntityReleaseWorld) -> Result<(), String> {
        loop {
            match self.phase {
                Phase::BeginChildren => self.phase = Phase::Children(world.child_count(self.entity)?),
                Phase::Children(0) => self.phase = Phase::BeginComponents,
                Phase::Children(remaining) => {
                    let child = world.child_at(self.entity, remaining - 1)?;
                    self.phase = Phase::DetachChild { remaining, child };
                }
                Phase::DetachChild { remaining, child } => {
                    world.detach_child(self.entity, child)?;
                    self.phase = Phase::Children(remaining - 1);
                }
                Phase::BeginComponents => self.phase = Phase::Components(world.component_count(self.entity)?),
                Phase::Components(0) => self.phase = Phase::Requested,
                Phase::Components(remaining) => {
                    let (component, generation) = world.component_at(self.entity, remaining - 1)?;
                    self.phase = Phase::RequestComponent { remaining, component, generation };
                }
                Phase::RequestComponent { remaining, component, generation } => {
                    world.request_component_release(self.entity, component, generation)?;
                    self.phase = Phase::Components(remaining - 1);
                }
                Phase::Requested | Phase::Retired => return Ok(()),
            }
        }
    }

    /// The game-local owner must drain the shared release/event batch before
    /// this call; `retire_entity` checks the actual owners before removing ID.
    pub fn finish(&mut self, world: &mut impl EntityReleaseWorld) -> Result<(), String> {
        if self.poll_finish(world)? { Ok(()) } else { Err("entity retirement awaits host completion".into()) }
    }
    pub fn poll_finish(&mut self, world: &mut impl EntityReleaseWorld) -> Result<bool, String> {
        match self.phase {
            Phase::Requested => {
                if !world.poll_retire_entity(self.entity)? { return Ok(false); }
                self.phase = Phase::Retired;
                Ok(true)
            }
            Phase::Retired => Ok(true),
            _ => Err("entity release has not completed its component requests".into()),
        }
    }
}

/// idGameLocal destruction uses a different entry path from a queued remove.
/// The callbacks must be safe to resume after a reported error.
pub trait WorldShutdownWorld: EntityReleaseWorld {
    /// Check every required real owner and retry-safe callback before the
    /// first entity or component is mutated.
    fn validate_world_shutdown_ready(&mut self) -> Result<(), String>;
    /// 8a48d5 stops music before requesting any entity/component release.
    fn shutdown_early_managers(&mut self) -> Result<bool, String> { Ok(true) }
    /// Set the entity's native +0xd2 pending-removal bit before 140876180.
    fn mark_world_shutdown_entity(&mut self, entity: EntityId) -> Result<(), String>;
    /// 1407ea870 using the actual component owner. The shutdown caller sets
    /// EDX to 1, but the callee does not consume it as a second argument.
    fn drain_world_shutdown_components(&mut self) -> Result<(), String>;
    /// Host worker acknowledgements may retain the current drain phase.
    /// Synchronous owners keep their existing callback and completion rule.
    fn poll_world_shutdown_components(&mut self) -> Result<bool, String> {
        self.drain_world_shutdown_components()?;
        Ok(true)
    }
    /// Run the actual intermediate manager shutdown in native order. The
    /// host retains its own subphase when a callback can fail.
    fn shutdown_intermediate_managers(&mut self) -> Result<bool, String>;
    /// Read the next real occupied native managed slot in ascending order.
    fn next_world_entity(&self, from: usize) -> Result<Option<(usize, EntityId)>, String>;
    /// Run the actual entity destructor and remove this exact identity. The
    /// host must retain destructor subphase and reject an already reused slot.
    fn destroy_world_entity(&mut self, slot: usize, entity: EntityId) -> Result<(), String>;
    /// An unspawned entity's scoped fallback may await its real Stop callback.
    fn poll_destroy_world_entity(&mut self, slot: usize, entity: EntityId) -> Result<bool, String> {
        self.destroy_world_entity(slot, entity)?;
        Ok(true)
    }
    /// Finish the actual post-entity graph/event/FX manager teardown.
    fn shutdown_remaining_managers(&mut self) -> Result<(), String>;
}

#[derive(Clone, Copy)]
enum WorldShutdownPhase {
    Preflight, EarlyManagers, Mark(usize), Request(usize), Drain, Intermediate,
    FindEntity(usize), DestroyEntity { slot: usize, entity: EntityId },
    Remaining, Finished,
}

/// The source-ordered 1408a4830 world path, without queued-remove child
/// detachment or save-manager preparation. Actual typed callbacks and manager
/// owners remain in the host; missing owners must fail preflight.
pub struct WorldShutdownBatch {
    entities: Vec<EntityId>,
    current: Option<EntityReleaseRequest>,
    phase: WorldShutdownPhase,
}

impl WorldShutdownBatch {
    pub fn from_spawned_snapshot(snapshot: &[EntityId]) -> Result<Self, String> {
        let mut entities = Vec::new();
        entities.try_reserve_exact(snapshot.len())
            .map_err(|error| format!("world shutdown snapshot: {error}"))?;
        entities.extend_from_slice(snapshot);
        Ok(Self { entities, current: None, phase: WorldShutdownPhase::Preflight })
    }

    pub fn advance(&mut self, world: &mut impl WorldShutdownWorld) -> Result<bool, String> {
        loop {
            match self.phase {
                WorldShutdownPhase::Preflight => {
                    world.validate_world_shutdown_ready()?;
                    self.phase = WorldShutdownPhase::EarlyManagers;
                }
                WorldShutdownPhase::EarlyManagers => {
                    if !world.shutdown_early_managers()? { return Ok(false); }
                    self.phase = WorldShutdownPhase::Mark(0);
                }
                WorldShutdownPhase::Mark(index) if index == self.entities.len() => self.phase = WorldShutdownPhase::Drain,
                WorldShutdownPhase::Mark(index) => {
                    let entity = self.entities[index];
                    world.mark_world_shutdown_entity(entity)?;
                    self.current = Some(EntityReleaseRequest::for_world_shutdown(entity));
                    self.phase = WorldShutdownPhase::Request(index);
                }
                WorldShutdownPhase::Request(index) => {
                    self.current.as_mut().ok_or("world shutdown request absent")?.request(world)?;
                    self.current = None;
                    self.phase = WorldShutdownPhase::Mark(index + 1);
                }
                WorldShutdownPhase::Drain => {
                    if !world.poll_world_shutdown_components()? { return Ok(false); }
                    self.phase = WorldShutdownPhase::Intermediate;
                }
                WorldShutdownPhase::Intermediate => {
                    if !world.shutdown_intermediate_managers()? { return Ok(false); }
                    self.phase = WorldShutdownPhase::FindEntity(0);
                }
                WorldShutdownPhase::FindEntity(from) => {
                    self.phase = match world.next_world_entity(from)? {
                        Some((slot, _)) if slot < from || slot >= 0x4000 => return Err("invalid managed world-shutdown cursor".into()),
                        Some((slot, entity)) => WorldShutdownPhase::DestroyEntity { slot, entity },
                        None => WorldShutdownPhase::Remaining,
                    };
                }
                WorldShutdownPhase::DestroyEntity { slot, entity } => {
                    let next = slot.checked_add(1).ok_or("managed slot cursor overflow")?;
                    if !world.poll_destroy_world_entity(slot, entity)? { return Ok(false); }
                    self.phase = WorldShutdownPhase::FindEntity(next);
                }
                WorldShutdownPhase::Remaining => {
                    world.shutdown_remaining_managers()?;
                    self.phase = WorldShutdownPhase::Finished;
                }
                WorldShutdownPhase::Finished => return Ok(true),
            }
        }
    }
}

/// Operations belonging to the existing game-local release batch. The host
/// must make each callback resumable or idempotent before connecting it.
pub trait EntityReleaseBatchWorld: EntityReleaseWorld {
    /// 14065afa0: update the conditional name record and indexed game-local
    /// byte before the entity's attachments/components are requested.
    fn prepare_entity_release(&mut self, entity: EntityId) -> Result<(), String>;
    /// Drain the shared component release/event owner, including its real
    /// typed callbacks. This is not a separate component scheduler.
    fn drain_entity_component_releases(&mut self) -> Result<(), String>;
    fn poll_entity_component_releases(&mut self) -> Result<bool, String> {
        self.drain_entity_component_releases()?;
        Ok(true)
    }
}

#[derive(Clone, Copy)]
enum BatchPhase {
    Prepare(usize),
    Request(usize),
    Drain,
    Retire(usize),
    Finished,
}

/// Progress through one ordered snapshot removed from game-local's pending
/// list by its owner. New requests raised by callbacks stay on that list for
/// the next batch, as in 1408ab380; no entity order is inferred here.
pub struct EntityReleaseBatch {
    requests: Vec<EntityReleaseRequest>,
    phase: BatchPhase,
}

impl EntityReleaseBatch {
    pub fn from_snapshot(snapshot: &[EntityId]) -> Result<Self, String> {
        let mut requests = Vec::new();
        requests.try_reserve_exact(snapshot.len())
            .map_err(|error| format!("entity release batch allocation: {error}"))?;
        requests.extend(snapshot.iter().copied().map(EntityReleaseRequest::new));
        Ok(Self { requests, phase: BatchPhase::Prepare(0) })
    }

    pub fn advance(&mut self, world: &mut impl EntityReleaseBatchWorld) -> Result<(), String> {
        if self.poll_advance(world)? { Ok(()) } else { Err("entity release batch awaits host completion".into()) }
    }
    pub fn poll_advance(&mut self, world: &mut impl EntityReleaseBatchWorld) -> Result<bool, String> {
        loop {
            match self.phase {
                BatchPhase::Prepare(index) if index == self.requests.len() => {
                    self.phase = BatchPhase::Drain;
                }
                BatchPhase::Prepare(index) => {
                    world.prepare_entity_release(self.requests[index].entity())?;
                    self.phase = BatchPhase::Request(index);
                }
                BatchPhase::Request(index) => {
                    self.requests[index].request(world)?;
                    self.phase = BatchPhase::Prepare(index + 1);
                }
                BatchPhase::Drain => {
                    if !world.poll_entity_component_releases()? { return Ok(false); }
                    self.phase = BatchPhase::Retire(0);
                }
                BatchPhase::Retire(index) if index == self.requests.len() => {
                    self.phase = BatchPhase::Finished;
                }
                BatchPhase::Retire(index) => {
                    if !self.requests[index].poll_finish(world)? { return Ok(false); }
                    self.phase = BatchPhase::Retire(index + 1);
                }
                BatchPhase::Finished => return Ok(true),
            }
        }
    }
}
