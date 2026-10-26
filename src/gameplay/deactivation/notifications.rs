//! Body activation notifications and island detachment (cd7ce0).
use super::{queue_attached_bodies, BodyStorage, DeactivationWorld};

/// Typed projection of the source's 16-byte command: header 03000010, opcode
/// 6, body ID and one activation byte. Uninitialized command padding is omitted.
#[derive(Clone, Copy, Debug)]
pub struct BodyActivationCommand {
    pub body: u32,
    pub activated: bool,
}

impl DeactivationWorld {
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
