//! Body-change records and delayed addition queues (caf710/cafcb0/c8e930).
use super::{BodyStorage, PackedBodyBounds};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyActivation {
    Activate,
    KeepDeactivated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BodyChange {
    pub body_index: u32,
    pub queue_index: u32,
    pub flags: u16,
}
impl Default for BodyChange {
    /// caf710 leaves allocator padding unspecified.
    fn default() -> Self {
        Self {
            body_index: u32::MAX,
            queue_index: u32::MAX,
            flags: 0,
        }
    }
}
impl BodyStorage {
    pub(super) fn cancel_pending_addition(&mut self, body: u32) -> Result<(), String> {
        let slot = (body & 0xffffff) as usize;
        let change_index = *self
            .change_indices
            .get(slot)
            .ok_or("pending body exceeds storage")?;
        if change_index == u32::MAX {
            return Ok(());
        }
        let change = self
            .changes
            .get(change_index as usize)
            .ok_or("pending body change is missing")?;
        if change.flags & 3 == 0 {
            return Ok(());
        }
        let index = change.queue_index as usize;
        let queue = if change.flags & 1 != 0 {
            &mut self.queued_active
        } else {
            &mut self.queued_deactivated
        };
        if index >= queue.len() {
            return Err("pending body queue index exceeds its queue".into());
        }
        let moved = *queue.last().unwrap();
        queue.swap_remove(index);
        if index < queue.len() {
            let moved_change = self.change_indices[(moved & 0xffffff) as usize];
            self.changes
                .get_mut(moved_change as usize)
                .ok_or("moved pending body change is missing")?
                .queue_index = index as u32;
        }
        let change = &mut self.changes[change_index as usize];
        change.queue_index = u32::MAX;
        change.flags &= !3;
        Ok(())
    }

    fn change(&mut self, body_id: u32) -> Result<&mut BodyChange, String> {
        let index = (body_id & 0xff_ffff) as usize;
        let entry = self
            .change_indices
            .get_mut(index)
            .ok_or("body change index exceeds storage")?;
        if *entry == u32::MAX {
            *entry = self.changes.len() as u32;
            self.changes.push(BodyChange {
                body_index: index as u32,
                ..BodyChange::default()
            });
        }
        self.changes
            .get_mut(*entry as usize)
            .ok_or("body change lookup points outside its records".into())
    }
    /// cafcb0 retains duplicate additions and full generation-tagged IDs. Each
    /// record's queue index is overwritten on another addition, including one
    /// into the other queue; its queue flags accumulate.
    pub fn queue_additions(
        &mut self,
        bodies: &[u32],
        activation: BodyActivation,
    ) -> Result<(), String> {
        for &id in bodies {
            let (index, flag) = match activation {
                BodyActivation::Activate => (self.queued_active.len(), 1),
                BodyActivation::KeepDeactivated => (self.queued_deactivated.len(), 2),
            };
            let change = self.change(id)?;
            change.queue_index = index as u32;
            change.flags |= flag;
            match activation {
                BodyActivation::Activate => self.queued_active.push(id),
                BodyActivation::KeepDeactivated => self.queued_deactivated.push(id),
            }
        }
        Ok(())
    }
    /// Static addition's change record in c8e930, after actual bounds packing.
    pub(crate) fn mark_bounds_changed(&mut self, body: u32) -> Result<(), String> {
        self.change(body)?.flags |= 4;
        Ok(())
    }

    /// c87dc0 invalidates collision history only for a published body. Keep
    /// unrelated change flags and reuse the existing per-body change record.
    pub(crate) fn invalidate_collision_history(&mut self, id: u32) -> Result<(), String> {
        let index = (id & 0xffffff) as usize;
        let body = self.get_by_index(index as u32).ok_or("invalidated body is missing")?;
        let mark_change = body.flags & 1 != 0;
        self.get_by_index_mut(index as u32).unwrap().status |= 1;
        self.previous_bounds[index] = PackedBodyBounds {
            words: [0xff7f_ffff, 0x7fff_ff7f, 0, 0],
        };
        if mark_change {
            self.mark_bounds_changed(id)?;
        }
        Ok(())
    }
    /// c8e930 resets this cache for both static and dynamic additions before
    /// publishing them to the broadphase. It is distinct from the current AABB.
    pub(crate) fn reset_previous_bounds(&mut self, body: u32) -> Result<(), String> {
        *self
            .previous_bounds
            .get_mut((body & 0xff_ffff) as usize)
            .ok_or("previous body bounds index exceeds storage")? = PackedBodyBounds {
            words: [0xff7f_ffff, 0x7fff_ff7f, 0, 0],
        };
        Ok(())
    }
    pub fn changes(&self) -> &[BodyChange] {
        &self.changes
    }
    pub fn queued_additions(&self, activation: BodyActivation) -> &[u32] {
        match activation {
            BodyActivation::Activate => &self.queued_active,
            BodyActivation::KeepDeactivated => &self.queued_deactivated,
        }
    }

    /// cafe10 clears only queued-addition state. Bounds/other change flags,
    /// change records, their index map and allocated queue capacities survive.
    pub fn reset_queued_additions(&mut self) {
        self.queued_active.clear();
        self.queued_deactivated.clear();
        for change in &mut self.changes {
            change.flags &= !3;
            change.queue_index = u32::MAX;
        }
    }

    /// c8f000: process active additions first, then the current deactivated
    /// queue, clear queue state and finalize the broadphase. The required world
    /// callback performs actual immediate addition (c8e930 mode1).
    pub fn flush_additions(
        &mut self,
        mut add: impl FnMut(&mut Self, &[u32], BodyActivation) -> Result<(), String>,
        finalize_broadphase: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        for activation in [BodyActivation::Activate, BodyActivation::KeepDeactivated] {
            // The world operation mutates body/change storage and can invoke
            // callbacks. Snapshot this batch without prematurely clearing it.
            let batch = self.queued_additions(activation).to_vec();
            if !batch.is_empty() {
                add(self, &batch, activation)?;
            }
        }
        self.reset_queued_additions();
        finalize_broadphase()
    }
}

#[cfg(test)]
#[path = "changes_tests.rs"]
mod tests;
