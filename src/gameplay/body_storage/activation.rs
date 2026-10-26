//! Active-body lists (caf8d0/cafc00), separate from allocated-body counts.
use super::{BodyActivation, BodyStorage};
use crate::gameplay::motion_storage::MotionStorage;

impl BodyStorage {
    /// c88590 updates active-body cell marks and invalidates every member's
    /// cached cell association after moving an attached motion between cells.
    pub fn update_attachment_cell(&mut self, first: u32, cell: u8) -> Result<(), String> {
        let mut id = first;
        for _ in 0..self.capacity() {
            let body = self.get_mut(id).ok_or("cell-update attachment body is missing")?;
            let active = body.flags & 8 != 0;
            body.status |= 0x20;
            let next = body.next_attached_body_id;
            if active { self.allocation_marks[(id & 0xffffff) as usize] = cell; }
            id = next;
            if id == first { return Ok(()); }
        }
        Err("cell-update attachment ring does not close".into())
    }

    pub fn active_body_ids(&self) -> &[u32] {
        &self.active_body_ids
    }

    /// Activate published ring members, requeue pending unpublished members,
    /// then publish their inactive motion into its solver cell.
    pub fn activate_attached(
        &mut self,
        first: u32,
        motions: &mut MotionStorage,
    ) -> Result<(), String> {
        let motion_id = self
            .get(first)
            .ok_or("activation body is missing")?
            .motion_id;
        let motion = motions
            .get(motion_id)
            .ok_or("activation motion is missing")?;
        let cell = motion.cell;
        let solver = motion.solver_id;
        let mut id = first;
        for _ in 0..self.capacity() {
            let body = self
                .get(id)
                .ok_or("activation attachment contains a stale body")?;
            let next = body.next_attached_body_id;
            let published = body.broadphase_id != u32::MAX;
            let pending = self.change_indices[(id & 0xffffff) as usize];
            let pending =
                pending != u32::MAX && self.changes[pending as usize].queue_index != u32::MAX;
            if body.flags & 8 == 0 && (published || pending) {
                if !published {
                    self.cancel_pending_addition(id)?;
                    self.queue_additions(&[id], BodyActivation::Activate)?;
                } else {
                    let index = self.active_body_ids.len() as u32;
                    let body = self.get_mut(id).unwrap();
                    body.flags |= 8;
                    body.status |= 0x220;
                    body.active_list_index = index;
                    self.active_body_ids.push(id);
                    self.allocation_marks[(id & 0xffffff) as usize] = cell;
                }
            }
            id = next;
            if id == first {
                if solver == 0x7fff_ffff {
                    motions.add_to_solver(motion_id)?;
                }
                return Ok(());
            }
        }
        Err("activation attachment ring does not close".into())
    }

    /// cafc00 removes a current active body, preserving the last member's slot.
    pub fn remove_active(&mut self, id: u32) -> Result<(), String> {
        let index = self
            .get(id)
            .ok_or("active body is missing")?
            .active_list_index as usize;
        if index >= self.active_body_ids.len() {
            return Err("active body index exceeds its list".into());
        }
        let moved = *self.active_body_ids.last().unwrap();
        self.active_body_ids.swap_remove(index);
        if index < self.active_body_ids.len() {
            self.get_mut(moved)
                .ok_or("moved active body is missing")?
                .active_list_index = index as u32;
        }
        self.allocation_marks[(id & 0xffffff) as usize] = u8::MAX;
        let body = self.get_mut(id).unwrap();
        body.flags &= !8;
        body.status |= 0x220;
        body.active_list_index = u32::MAX;
        Ok(())
    }
}

#[cfg(test)]
#[path = "activation_tests.rs"]
pub(super) mod tests;
