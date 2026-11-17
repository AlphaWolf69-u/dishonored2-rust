//! Host rollback for a fallible constructor. Source callbacks are synchronous;
//!remove only the failed actor's registrations and adjust the real batch
//!cursors, preserving unrelated completed callbacks, queue order and deferrals.
use super::*;
impl ComponentRegistry {
    pub(crate) fn abort_entity_startup(
        &mut self,
        entity: usize,
        targets: &[(ComponentHandle, u32)],
        world: &impl ComponentLifecycleWorld,
    ) -> Result<(), String> {
        for &(handle, generation) in targets {
            let owner = world.owner(handle)?;
            if owner.entity != Some(entity) || owner.generation != generation {
                return Err("startup abort target generation differs".into());
            }
        }
        self.abort_components_startup(targets, world)
    }

    /// Host failure recovery may unwind one constructor and its actual
    /// constructor-owned children together. Adjust the accepted queue cursor
    /// once, without entering another callback between those withdrawals.
    pub(crate) fn abort_components_startup(
        &mut self,
        targets: &[(ComponentHandle, u32)],
        world: &impl ComponentLifecycleWorld,
    ) -> Result<(), String> {
        for &(handle, generation) in targets {
            let owner = world.owner(handle)?;
            if owner.entity.is_none() || owner.generation != generation {
                return Err("startup abort target generation differs".into());
            }
        }
        let target = |value: u64| targets.iter().any(|(handle, _)| handle.packed() == value);
        let entered = self.interrupted_startup_component()?;
        if entered.is_some_and(|handle| !target(handle.packed())) {
            return Err("startup abort would cancel another actor's entered callback".into());
        }
        let removed_before = |values: &[u64], boundary: usize| {
            values
                .iter()
                .take(boundary)
                .filter(|&&value| target(value))
                .count() as u32
        };
        let initialized_count = removed_before(
            self.registered.values(),
            self.processed_initialization_count.max(0) as usize,
        );
        let initialized_next = removed_before(
            self.registered.values(),
            self.startup_progress.initialization_next as usize,
        );
        let started_count = removed_before(
            self.pending_start.values(),
            self.processed_start_count.max(0) as usize,
        );
        let started_next = removed_before(
            self.pending_start.values(),
            self.startup_progress.start_next as usize,
        );
        let restored = removed_before(
            &self.startup_progress.deferred,
            self.startup_progress.restore_next,
        ) as usize;
        let deferred = self
            .startup_progress
            .deferred
            .iter()
            .copied()
            .filter(|&value| !target(value))
            .collect::<Vec<_>>();
        // Allocate replacement storage before changing any accepted source cursor.
        let mut registered = PointerList::owned_with_capacity(self.registered.capacity())?;
        for &value in self.registered.values() {
            if !target(value) {
                registered.append_preserving(value, false)?;
            }
        }
        let mut starts = PointerList::owned_with_capacity(self.pending_start.capacity())?;
        for &value in self.pending_start.values() {
            if !target(value) {
                starts.append_preserving(value, false)?;
            }
        }
        self.registered = registered;
        self.pending_start = starts;
        self.processed_initialization_count -= initialized_count as i32;
        self.startup_progress.initialization_next -= initialized_next;
        self.processed_start_count -= started_count as i32;
        self.startup_progress.start_next -= started_next;
        self.startup_progress.restore_next -= restored;
        self.startup_progress.deferred = deferred;
        if entered.is_some() && self.startup_progress.initialization_active {
            self.startup_progress.initialization_step = 0;
        }
        Ok(())
    }
}
