//! Body activation notifications and island detachment (cd7ce0).
use super::{queue_attached_bodies, BodyStorage, DeactivationWorld};

/// Typed projection of the source's 16-byte command: header 03000010, opcode
/// 6, body ID and one activation byte. Uninitialized command padding is omitted.
#[derive(Clone, Copy, Debug)]
pub struct BodyActivationCommand {
    pub body: u32,
    pub activated: bool,
}

#[derive(Default)]
pub(crate) struct ActivationNotifyProgress {
    step: u8,
    members: Vec<u32>,
}
impl ActivationNotifyProgress {
    pub(crate) fn retained_bytes(&self) -> usize { self.members.capacity() * std::mem::size_of::<u32>() }
}

impl DeactivationWorld {
    /// Resume cd7ce0 without repeating an already delivered activation
    /// signal if a later island operation reports an error.
    pub(crate) fn notify_activation_resumable(
        &mut self, bodies: &mut BodyStorage, first: u32, phase: u32,
        progress: &mut ActivationNotifyProgress,
        mut notify: impl FnMut(&mut Self, &mut BodyStorage, &[u32], bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let motion = bodies.get(first).ok_or("notification body is missing")?.motion_id;
        self.state(motion).ok_or("notification motion state is missing")?;
        if progress.step == 0 {
            progress.members.clear();
            queue_attached_bodies(bodies, first, &mut progress.members)?;
            progress.members.retain(|&id| bodies.get(id).is_some_and(|body| body.broadphase_id != u32::MAX));
            progress.step = 1;
        }
        if progress.step == 1 {
            if matches!(phase, 1 | 0x40) {
                notify(self, bodies, &progress.members, true)?;
            } else {
                self.body_commands.try_reserve(progress.members.len()).map_err(|error| format!("activation commands: {error}"))?;
                for &body in &progress.members { self.body_commands.push(BodyActivationCommand { body, activated: true }); }
            }
            progress.step = 2;
        }
        let island = self.state(motion).ok_or("deactivation state is uninitialized")?.island();
        if !self.suppress_island_detach && island != -1 {
            progress.members.clear();
            queue_attached_bodies(bodies, first, &mut progress.members)?;
            let record = self.islands.get(island as usize).and_then(Option::as_ref).ok_or("activation island is missing")?;
            if record.queued == 0 {
                self.activation_queue.try_reserve(1).map_err(|error| format!("activation island queue: {error}"))?;
            }
            self.state_mut(motion).expect("validated activation state").set_island(-1);
            let record = self.island_mut(island as u32)?;
            for &body in &progress.members {
                if let Some(index) = record.bodies.iter().position(|&id| id == body) { record.bodies.swap_remove(index); }
            }
            record.wake = 1;
            if record.queued == 0 {
                self.activation_queue.push(island as u32);
                self.island_mut(island as u32)?.queued = 1;
            }
        }
        progress.step = 0;
        progress.members.clear();
        Ok(())
    }

    /// cd9ae0 queues a removed static body for the affected sleeping island.
    /// A false result requires the caller to run the real c91230 activation
    /// for a body that is not assigned to an island.
    pub(crate) fn queue_removed_static(
        &mut self, bodies: &BodyStorage, static_body: u32, other: u32,
    ) -> Result<bool, String> {
        let motion = bodies.get(other).ok_or("static-overlap body is missing")?.motion_id;
        let island = self.state(motion).ok_or("static-overlap motion state is missing")?.island();
        if island == -1 { return Ok(false); }
        if self.island_mut(island as u32)?.removed_bodies.last() != Some(&static_body) {
            self.queue_island(bodies, other, 1)?;
            self.island_mut(island as u32)?.removed_bodies.push(static_body);
        }
        Ok(true)
    }

    /// cd80c0(body, 1): retire a destroyed body's membership while keeping
    /// any other members and the island's queued wake ownership intact.
    pub(crate) fn detach_destroyed_body(
        &mut self, bodies: &BodyStorage, id: u32,
    ) -> Result<(), String> {
        self.detach_removed_body(bodies, id, true)
    }

    /// cd80c0 retains a removed body ID for contact maintenance when the
    /// caller requests ordinary removal (mode zero), but not on destruction.
    pub(crate) fn detach_removed_body(
        &mut self, bodies: &BodyStorage, id: u32, destroying: bool,
    ) -> Result<(), String> {
        let body = bodies.get(id).ok_or("destroyed island body is missing")?;
        let motion = body.motion_id;
        let island = self.state(motion).ok_or("destroyed motion state is missing")?.island();
        if island == -1 { return self.queue_island(bodies, id, u8::from(!destroying)); }
        if self.islands.get(island as usize).and_then(Option::as_ref).is_none() {
            return Err("destroyed body island is missing".into());
        }
        let mut current = body.next_attached_body_id;
        let mut published_remaining = false;
        let mut visited = 0usize;
        while current != id {
            let member = bodies.get(current).ok_or("motion body ring is broken")?;
            if member.broadphase_id != u32::MAX { published_remaining = true; break; }
            current = member.next_attached_body_id;
            visited += 1;
            if visited > bodies.capacity() { return Err("motion body ring contains a cycle".into()); }
        }
        // The ring and island are now known valid. The native queue action
        // still precedes detachment, without leaving a queued side effect on
        // a later host validation error.
        self.queue_island(bodies, id, u8::from(!destroying))?;
        if !published_remaining {
            self.state_mut(motion).ok_or("destroyed motion state is missing")?.set_island(-1);
        }
        let record = self.island_mut(island as u32)?;
        if let Some(index) = record.bodies.iter().position(|&body| body == id) {
            record.bodies.swap_remove(index);
            if !destroying { record.removed_bodies.push(id); record.wake = 1; }
            if record.bodies.is_empty() { record.wake = 1; }
        }
        Ok(())
    }

    /// World phases1/40 emit their actual signal before detaching island
    /// members; other phases queue commands for the command processor.
    pub fn notify_activation(
        &mut self,
        bodies: &mut BodyStorage,
        first: u32,
        deactivated: bool,
        phase: u32,
        mut notify: impl FnMut(&mut Self, &mut BodyStorage, &[u32], bool) -> Result<(), String>,
    ) -> Result<(), String> {
        let motion = bodies
            .get(first)
            .ok_or("notification body is missing")?
            .motion_id;
        self.state(motion)
            .ok_or("notification motion state is missing")?;
        let mut members = std::mem::take(&mut self.attached);
        members.clear();
        queue_attached_bodies(bodies, first, &mut members)?;
        if matches!(phase, 1 | 0x40) {
            members.retain(|&id| bodies.get(id).unwrap().broadphase_id != u32::MAX);
            let result = notify(self, bodies, &members, !deactivated);
            self.attached = members;
            result?;
        } else {
            for &id in &members {
                if bodies.get(id).unwrap().broadphase_id != u32::MAX {
                    self.body_commands.push(BodyActivationCommand {
                        body: id,
                        activated: !deactivated,
                    });
                }
            }
            self.attached = members;
        }
        let island = self
            .state(motion)
            .ok_or("deactivation state is uninitialized")?
            .island();
        if !self.suppress_island_detach && !deactivated && island != -1 {
            self.state_mut(motion)
                .ok_or("deactivation state is uninitialized")?
                .set_island(-1);
            // Re-read the whole ring after an immediate callback, as native
            // does. Unpublished members still participate in island removal.
            self.attached.clear();
            queue_attached_bodies(bodies, first, &mut self.attached)?;
            for i in 0..self.attached.len() {
                let body = self.attached[i];
                let island = self.island_mut(island as u32)?;
                if let Some(index) = island.bodies.iter().position(|&id| id == body) {
                    island.bodies.swap_remove(index);
                }
            }
            let record = self.island_mut(island as u32)?;
            record.wake = 1;
            if record.queued == 0 {
                self.activation_queue.push(island as u32);
                self.island_mut(island as u32)?.queued = 1;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
