//! Deactivated-island body linkage and merging (cd8310/cd9670).
use super::{queue_attached_bodies, MotionDeactivation};
use crate::gameplay::{body_storage::BodyStorage, motion_storage::MotionStorage};
#[path = "notifications.rs"]
mod notifications;
pub use notifications::BodyActivationCommand;

#[derive(Clone, Copy, Debug)]
pub struct IslandCacheLink {
    pub island: u32,
    pub index: u32,
}

#[derive(Debug, Default)]
pub struct DeactivatedIsland {
    pub id: u32,
    pub bodies: Vec<u32>,
    /// Handles into the world's collision-cache link records (+28).
    pub cache_links: Vec<usize>,
    /// Opaque 16-byte cache records (+40), selected by unsigned priority +60.
    pub cache_records: Vec<[u32; 4]>,
    pub cache_priority: u32,
    pub pairs: Vec<[u32; 2]>,
    pub pair_step: u32,
    pub wake: u8,
    pub queued: u8,
}

pub struct DeactivationWorld {
    /// Allocated motion slots. Uncreated motions have no initialized state.
    pub states: Vec<Option<MotionDeactivation>>,
    pub islands: Vec<Option<DeactivatedIsland>>,
    pub free_islands: Vec<u32>,
    pub activation_queue: Vec<u32>,
    pub cache_links: Vec<IslandCacheLink>,
    pub body_commands: Vec<BodyActivationCommand>,
    pub pending_bodies: Vec<u32>,
    pub suppress_island_detach: bool,
    attached: Vec<u32>,
}

impl DeactivationWorld {
    /// cdcdb0 marks the body's assigned island and queues it only once. Zero
    /// wake flags still enqueue an unqueued island without changing its wake.
    pub fn queue_island(
        &mut self,
        bodies: &BodyStorage,
        body: u32,
        wake_flags: u8,
    ) -> Result<(), String> {
        let motion = bodies
            .get_by_index(body & 0xffffff)
            .ok_or("queued island body is missing")?
            .motion_id;
        let island = self
            .state(motion as u32)
            .ok_or("queued island motion state is missing")?
            .island();
        if island != -1 {
            let record = self.island_mut(island as u32)?;
            record.wake |= wake_flags;
            let enqueue = record.queued == 0;
            if enqueue {
                self.activation_queue.push(island as u32);
                self.island_mut(island as u32)?.queued = 1;
            }
        }
        Ok(())
    }

    pub fn new(states: Vec<MotionDeactivation>) -> Self {
        Self {
            states: states.into_iter().map(Some).collect(),
            islands: Vec::new(),
            free_islands: Vec::new(),
            activation_queue: Vec::new(),
            cache_links: Vec::new(),
            body_commands: Vec::new(),
            pending_bodies: Vec::new(),
            suppress_island_detach: false,
            attached: Vec::new(),
        }
    }

    /// c8c250 allocates the motion-capacity table but initializes ONLYanchor0.
    pub fn for_motion_capacity(capacity: usize) -> Result<Self, String> {
        if capacity == 0 || capacity >= 0x7fff_ffff {
            return Err("deactivation motion capacity must include its anchor".into());
        }
        let mut world = Self::new(Vec::new());
        world
            .states
            .try_reserve_exact(capacity)
            .map_err(|e| format!("deactivation state allocation failed: {e}"))?;
        world.states.resize(capacity, None);
        world.states[0] = Some(MotionDeactivation::new(0, 0));
        Ok(world)
    }

    pub fn state(&self, motion: u32) -> Option<&MotionDeactivation> {
        self.states.get(motion as usize)?.as_ref()
    }
    pub fn state_mut(&mut self, motion: u32) -> Option<&mut MotionDeactivation> {
        self.states.get_mut(motion as usize)?.as_mut()
    }

    /// c908f0 grows the logical count to include the new ID, then callsce90e0.
    /// Retained records preserve bytes that the source constructor leaves alone.
    pub fn initialize_motion_state(&mut self, motion: u32, enabled: bool) -> Result<(), String> {
        let length = (motion as usize)
            .checked_add(1)
            .ok_or("motion state ID overflow")?;
        if length > self.states.len() {
            if length > self.states.capacity() {
                let capacity = self
                    .states
                    .capacity()
                    .checked_mul(2)
                    .ok_or("motion state capacity overflow")?
                    .max(length);
                self.states
                    .try_reserve_exact(capacity - self.states.len())
                    .map_err(|e| format!("deactivation state growth failed: {e}"))?;
            }
            self.states.resize(length, None);
        }
        let mode = u8::from(!enabled);
        if let Some(state) = &mut self.states[motion as usize] {
            state.initialize(motion, mode);
        } else {
            self.states[motion as usize] = Some(MotionDeactivation::new(motion, mode));
        }
        Ok(())
    }

    fn island_mut(&mut self, index: u32) -> Result<&mut DeactivatedIsland, String> {
        self.islands
            .get_mut(index as usize)
            .and_then(Option::as_mut)
            .ok_or_else(|| format!("missing deactivation island {index}"))
    }

    fn allocate(&mut self) -> Result<u32, String> {
        let index = if let Some(index) = self.free_islands.pop() {
            if index as usize >= self.islands.len() {
                return Err("free deactivation island exceeds storage".into());
            }
            index
        } else {
            let index =
                u32::try_from(self.islands.len()).map_err(|_| "too many deactivation islands")?;
            self.islands.push(None);
            index
        };
        self.islands[index as usize] = Some(DeactivatedIsland {
            id: index,
            ..Default::default()
        });
        Ok(index)
    }

    fn add_attached(
        &mut self,
        bodies: &BodyStorage,
        first: u32,
        island: u32,
    ) -> Result<(), String> {
        self.attached.clear();
        queue_attached_bodies(bodies, first, &mut self.attached)?;
        for i in 0..self.attached.len() {
            let id = self.attached[i];
            if bodies
                .get(id)
                .ok_or("attached island body disappeared")?
                .broadphase_id
                != u32::MAX
            {
                self.island_mut(island)?.bodies.push(id);
            }
        }
        Ok(())
    }

    fn append_pair(&mut self, island: u32, pair: [u32; 2], step: u32) -> Result<(), String> {
        let island = self.island_mut(island)?;
        if island.pair_step != step {
            island.pairs.clear();
            island.pair_step = step;
        }
        island.pairs.push(pair);
        Ok(())
    }

    /// cd9670. Source cache-link records are retargeted before their handles
    /// are appended; generation validation applies to transferred body states.
    pub fn merge(
        &mut self,
        bodies: &BodyStorage,
        destination: u32,
        source: u32,
    ) -> Result<(), String> {
        if destination == source {
            return Ok(());
        }
        let destination_id = self.island_mut(destination)?.id;
        let mut removed = self
            .islands
            .get_mut(source as usize)
            .and_then(Option::take)
            .ok_or("merge source deactivation island is missing")?;
        for &handle in &removed.cache_links {
            let index = self.island_mut(destination)?.cache_links.len() as u32;
            let cache = self
                .cache_links
                .get_mut(handle)
                .ok_or("island cache link is missing")?;
            *cache = IslandCacheLink {
                island: destination_id,
                index,
            };
            self.island_mut(destination)?.cache_links.push(handle);
        }
        for &id in &removed.bodies {
            if let Some(body) = bodies.get(id).filter(|body| body.flags & 3 != 0) {
                self.state_mut(body.motion_id)
                    .ok_or("merged body motion state is missing")?
                    .set_island(destination_id as i32);
            }
        }
        let enqueue = self.island_mut(destination)?.queued == 0 && removed.queued != 0;
        if enqueue {
            self.activation_queue.push(destination);
        }
        let target = self.island_mut(destination)?;
        target.bodies.append(&mut removed.bodies);
        target.pairs.append(&mut removed.pairs);
        target.queued |= removed.queued;
        target.wake |= removed.wake;
        if target.cache_priority < removed.cache_priority {
            target.cache_records = std::mem::take(&mut removed.cache_records);
            target.cache_priority = removed.cache_priority;
        } else if target.cache_priority == removed.cache_priority {
            target.cache_records.append(&mut removed.cache_records);
        }
        if removed.id == source {
            self.free_islands.push(source);
        }
        Ok(())
    }

    /// cd8310. Solver IDs distinguish static (0), inactive (7fffffff) and
    /// active motions. Original pair and attachment order is retained.
    pub fn link_bodies(
        &mut self,
        bodies: &BodyStorage,
        motions: &MotionStorage,
        first: u32,
        second: u32,
        step: u32,
    ) -> Result<(), String> {
        let motion_a = bodies
            .get_by_index(first & 0xffffff)
            .ok_or("first linked body is missing")?
            .motion_id;
        let motion_b = bodies
            .get_by_index(second & 0xffffff)
            .ok_or("second linked body is missing")?
            .motion_id;
        let solver_a = motions
            .get(motion_a)
            .ok_or("first linked motion is missing")?
            .solver_id;
        let solver_b = motions
            .get(motion_b)
            .ok_or("second linked motion is missing")?
            .solver_id;
        let island_a = self
            .state(motion_a)
            .ok_or("first deactivation state is missing")?
            .island() as u32;
        let island_b = self
            .state(motion_b)
            .ok_or("second deactivation state is missing")?
            .island() as u32;
        let active = |solver| solver != 0 && solver != 0x7fff_ffff;
        if (solver_a == 0 && solver_b != 0x7fff_ffff) || (solver_b == 0 && active(solver_a)) {
            return Ok(());
        }
        let pair = [first, second];
        if solver_a == 0 && island_b == u32::MAX {
            let island = self.allocate()?;
            self.state_mut(motion_b)
                .ok_or("deactivation state is uninitialized")?
                .set_island(island as i32);
            return self.add_attached(bodies, second, island);
        }
        if solver_b == 0 && island_a == u32::MAX {
            let island = self.allocate()?;
            self.state_mut(motion_a)
                .ok_or("deactivation state is uninitialized")?
                .set_island(island as i32);
            return self.add_attached(bodies, first, island);
        }
        if solver_a == 0 || solver_b == 0 {
            return self.append_pair(!island_b ^ island_a, pair, step);
        }
        if active(solver_a) == active(solver_b) {
            if active(solver_a) {
                return Ok(());
            }
            match (island_a == u32::MAX, island_b == u32::MAX) {
                (true, true) => {
                    let island = self.allocate()?;
                    self.add_attached(bodies, first, island)?;
                    self.add_attached(bodies, second, island)?;
                    self.append_pair(island, pair, step)?;
                    self.state_mut(motion_a)
                        .ok_or("deactivation state is uninitialized")?
                        .set_island(island as i32);
                    self.state_mut(motion_b)
                        .ok_or("deactivation state is uninitialized")?
                        .set_island(island as i32);
                }
                (false, false) if island_a == island_b => self.append_pair(island_a, pair, step)?,
                (false, false) => {
                    self.merge(bodies, island_a, island_b)?;
                    // Native appends before checking the step on this branch;
                    // a changed step consequently discards the new pair too.
                    let island = self.island_mut(island_a)?;
                    island.pairs.push(pair);
                    if island.pair_step != step {
                        island.pairs.clear();
                        island.pair_step = step;
                    }
                }
                _ => {
                    let island = !island_b ^ island_a;
                    self.add_attached(
                        bodies,
                        if island_a == u32::MAX { first } else { second },
                        island,
                    )?;
                    self.append_pair(island, pair, step)?;
                    self.state_mut(motion_a)
                        .ok_or("deactivation state is uninitialized")?
                        .set_island(island as i32);
                    self.state_mut(motion_b)
                        .ok_or("deactivation state is uninitialized")?
                        .set_island(island as i32);
                }
            }
        } else {
            let (body, motion, mut island) = if active(solver_a) {
                (second, motion_b, island_b)
            } else {
                (first, motion_a, island_a)
            };
            if island == u32::MAX {
                island = self.allocate()?;
                self.state_mut(motion)
                    .ok_or("deactivation state is uninitialized")?
                    .set_island(island as i32);
                self.add_attached(bodies, body, island)?;
            }
            self.append_pair(island, pair, step)?;
            let queued = self.island_mut(island)?.queued;
            if queued == 0 {
                self.activation_queue.push(island);
            }
            let island = self.island_mut(island)?;
            island.queued = 1;
            island.wake = 1;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "islands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "island_queue_tests.rs"]
mod queue_tests;
