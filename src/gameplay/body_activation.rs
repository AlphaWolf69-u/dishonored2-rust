//! World activation sequencing (c91230), using owned body/motion/island state.
use super::{
    body_storage::BodyStorage,
    deactivation::{queue_attached_bodies, DeactivationWorld},
    motion_storage::MotionStorage,
};
#[path = "body_activation/context.rs"]
mod context;
pub use context::{ActivationContext, ActivationOwners, ActivationSignals};

/// Required world boundaries: the real activation signal and broadphase update.
/// Implementations receive mutable owners so callback changes precede the next
/// native operation. No default/no-op implementation is provided.
pub trait ActivationCallbacks {
    fn notify(
        &mut self,
        deactivation: &mut DeactivationWorld,
        bodies: &mut BodyStorage,
        motions: &mut MotionStorage,
        ids: &[u32],
        activated: bool,
    ) -> Result<(), String>;
    fn update_broadphase(
        &mut self,
        bodies: &mut BodyStorage,
        ids: &[u32],
        stride: u32,
    ) -> Result<(), String>;
}

#[derive(Default)]
pub struct BodyActivationController {
    affected: Vec<u32>,
    attached: Vec<u32>,
}

impl BodyActivationController {
    /// Return false for the original mode/body admission rejection. A failure
    /// in a required owner operation is an error, not a successful activation.
    pub fn set(
        &mut self,
        bodies: &mut BodyStorage,
        motions: &mut MotionStorage,
        deactivation: &mut DeactivationWorld,
        first: u32,
        deactivate: bool,
        phase: u32,
        callbacks: &mut impl ActivationCallbacks,
    ) -> Result<bool, String> {
        let body = bodies.get(first).ok_or("activation body is missing")?;
        let motion_id = body.motion_id;
        let state = deactivation
            .state(motion_id)
            .ok_or("activation motion state is missing")?;
        if (!deactivate
            && (body.flags & 9 != 0 || body.broadphase_id == u32::MAX || state.mode() == 2))
            || (deactivate && state.mode() == 1)
        {
            return Ok(false);
        }
        self.affected.clear();
        if !deactivate {
            bodies.activate_attached(first, motions)?;
            let motion_id = bodies
                .get(first)
                .ok_or("activated body disappeared")?
                .motion_id;
            deactivation
                .state_mut(motion_id)
                .ok_or("activated motion state disappeared")?
                .restart();
            deactivation.notify_activation(
                bodies,
                first,
                false,
                phase,
                |world, bodies, ids, activated| {
                    callbacks.notify(world, bodies, motions, ids, activated)
                },
            )?;
            self.attached.clear();
            queue_attached_bodies(bodies, first, &mut self.attached)?;
            for &id in &self.attached {
                let body = bodies.get(id).unwrap();
                if body.broadphase_id != u32::MAX {
                    self.affected.push(body.id);
                }
            }
        } else {
            let solver = motions
                .get(motion_id)
                .ok_or("deactivated motion is missing")?
                .solver_id;
            if solver != 0 && solver != 0x7fff_ffff {
                motions.remove_from_solver(motion_id)?;
            }
            self.attached.clear();
            queue_attached_bodies(bodies, first, &mut self.attached)?;
            for &id in &self.attached {
                if bodies.get(id).unwrap().flags & 8 != 0 {
                    bodies.remove_active(id)?;
                    self.affected.push(id);
                }
            }
            queue_attached_bodies(bodies, first, &mut deactivation.pending_bodies)?;
            deactivation.notify_activation(
                bodies,
                first,
                true,
                phase,
                |world, bodies, ids, activated| {
                    callbacks.notify(world, bodies, motions, ids, activated)
                },
            )?;
        }
        if !self.affected.is_empty() {
            callbacks.update_broadphase(bodies, &self.affected, 4)?;
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "body_activation/tests.rs"]
mod tests;
