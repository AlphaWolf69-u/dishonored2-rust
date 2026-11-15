//! Active-body lists (caf8d0/cafc00), separate from allocated-body counts.
use super::{BodyActivation, BodyStorage};
use crate::gameplay::motion_storage::MotionStorage;

pub(crate) struct AttachmentCellCursor { current: u32, visited: usize }
impl AttachmentCellCursor {
    pub(crate) fn new(first: u32) -> Self { Self { current: first, visited: 0 } }
}

impl BodyStorage {
    /// c88590 updates active-body cell marks and invalidates every member's
    /// cached cell association after moving an attached motion between cells.
    pub fn update_attachment_cell(&mut self, first: u32, cell: u8) -> Result<(), String> {
        let mut cursor = AttachmentCellCursor::new(first);
        self.update_attachment_cell_resumable(first, cell, &mut cursor)
    }

    pub(crate) fn update_attachment_cell_resumable(&mut self, first: u32, cell: u8,
        cursor: &mut AttachmentCellCursor) -> Result<(), String> {
        while cursor.visited < self.capacity() {
            let id = cursor.current;
            let body = self.get_mut(id).ok_or("cell-update attachment body is missing")?;
            let active = body.flags & 8 != 0;
            body.status |= 0x20;
            let next = body.next_attached_body_id;
            if active { self.allocation_marks[(id & 0xffffff) as usize] = cell; }
            cursor.current = next;
            cursor.visited += 1;
            if cursor.current == first { return Ok(()); }
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
        if self.active_body_ids[index] != id { return Err("active body index identifies another body".into()); }
        let moved = *self.active_body_ids.last().unwrap();
        if index + 1 < self.active_body_ids.len() && self.get(moved).is_none() {
            return Err("moved active body is missing".into());
        }
        self.active_body_ids.swap_remove(index);
        if index < self.active_body_ids.len() {
            self.get_mut(moved).expect("validated moved active body").active_list_index = index as u32;
        }
        self.allocation_marks[(id & 0xffffff) as usize] = u8::MAX;
        let body = self.get_mut(id).unwrap();
        body.flags &= !8;
        body.status |= 0x220;
        body.active_list_index = u32::MAX;
        Ok(())
    }

    /// cafae0 removes each published member from the active list, then removes
    /// its motion from the solver. The ring itself remains attached.
    pub(crate) fn remove_active_attached(
        &mut self, first: u32, motions: &mut MotionStorage,
    ) -> Result<(), String> {
        let motion = self.get(first).ok_or("active attachment body is missing")?.motion_id;
        let mut id = first;
        for _ in 0..self.capacity() {
            let body = self.get(id).ok_or("active attachment ring contains a stale body")?;
            let next = body.next_attached_body_id;
            if body.broadphase_id != u32::MAX { self.remove_active(id)?; }
            id = next;
            if id == first { return motions.remove_from_solver(motion); }
        }
        Err("active attachment ring does not close".into())
    }

    /// c93740 reconciles a motion after publication changes. It stays in its
    /// solver/island while at least one attached body is still in broadphase.
    pub(crate) fn reconcile_removed_motion(
        &self, motion: u32, motions: &mut MotionStorage,
        deactivation: &mut crate::gameplay::deactivation::DeactivationWorld,
    ) -> Result<(), String> {
        if motion == 0 { return Ok(()); }
        let record = motions.get(motion).ok_or("removed body motion is missing")?;
        let first = record.first_body_id;
        let solver = record.solver_id;
        let mut id = first;
        for _ in 0..self.capacity() {
            let body = self.get(id).ok_or("removed motion ring contains a stale body")?;
            if body.broadphase_id != u32::MAX { return Ok(()); }
            id = body.next_attached_body_id;
            if id == first {
                if solver == 0x7fff_ffff {
                    deactivation.state_mut(motion).ok_or("removed motion state is missing")?.set_island(-1);
                } else {
                    motions.remove_from_solver(motion)?;
                }
                return Ok(());
            }
        }
        Err("removed motion ring does not close".into())
    }

    /// c927c0's shared-motion branch. The caller handles the distinct
    /// last-body path and destruction observers before retiring body storage.
    pub(crate) fn unlink_shared_motion(
        &mut self, id: u32, motions: &mut MotionStorage,
        deactivation: &mut crate::gameplay::deactivation::DeactivationWorld,
        progress: &mut u8,
    ) -> Result<(), String> {
        let body = self.get(id).ok_or("detached body is missing")?;
        let motion = body.motion_id;
        let next = body.next_attached_body_id;
        if motion == 0 || motion == 0x7fff_ffff || (*progress == 0 && next == id) {
            return Err("shared-motion detach requires a multi-body motion".into());
        }
        if *progress == 0 {
            motions.get(motion).ok_or("detached body motion is missing")?;
            let mut previous = next;
            let mut found = false;
            for _ in 0..self.capacity() {
                let member = self.get(previous).ok_or("detached motion ring is broken")?;
                if member.motion_id != motion { return Err("detached motion ring changes owner".into()); }
                if member.next_attached_body_id == id { found = true; break; }
                previous = member.next_attached_body_id;
            }
            if !found { return Err("detached motion ring does not close".into()); }
            self.get_mut(previous).unwrap().next_attached_body_id = next;
            motions.get_mut(motion).unwrap().first_body_id = previous;
            self.get_mut(id).unwrap().next_attached_body_id = id;
            *progress = 1;
        }
        if *progress == 1 { self.reconcile_removed_motion(motion, motions, deactivation)?; *progress = 2; }
        self.get_mut(id).unwrap().motion_id = 0x7fff_ffff;
        *progress = 3;
        Ok(())
    }

    /// c8f6b0/c927c0's last-body branch. The callback dispatches the actual
    /// motion-destroyed observers before the motion slot is released.
    pub(crate) fn unlink_last_motion(
        &mut self, id: u32, motions: &mut MotionStorage,
        deactivation: &mut crate::gameplay::deactivation::DeactivationWorld,
        progress: &mut u8,
        mut destroyed: impl FnMut(&mut BodyStorage, &mut MotionStorage,
            &mut crate::gameplay::deactivation::DeactivationWorld, u32) -> Result<(), String>,
    ) -> Result<(), String> {
        let body = self.get(id).ok_or("detached body is missing")?;
        let motion = body.motion_id;
        let flags = body.flags;
        if motion == 0 || motion == 0x7fff_ffff || body.next_attached_body_id != id {
            return Err("last-motion detach requires the sole dynamic body".into());
        }
        if body.broadphase_id != u32::MAX && body.flags & 1 == 0 {
            return Err("last-motion body is still published with a dynamic motion kind".into());
        }
        if *progress == 0 {
            if motions.get(motion).ok_or("detached body motion is missing")?.solver_id != 0x7fff_ffff {
                motions.remove_from_solver(motion)?;
            }
            *progress = 1;
        }
        if *progress == 1 {
            if flags & 9 == 0 { deactivation.detach_destroyed_body(self, id)?; }
            *progress = 2;
        }
        if *progress == 2 { destroyed(self, motions, deactivation, motion)?; *progress = 3; }
        if *progress == 3 {
            motions.get_mut(motion).ok_or("destroyed motion disappeared")?.first_body_id = 0x00ff_ffff;
            *progress = 4;
        }
        if *progress == 4 { motions.release_detached(motion)?; *progress = 5; }
        self.get_mut(id).ok_or("detached body disappeared")?.motion_id = 0x7fff_ffff;
        *progress = 6;
        Ok(())
    }
}

#[cfg(test)]
#[path = "activation_tests.rs"]
pub(super) mod tests;
