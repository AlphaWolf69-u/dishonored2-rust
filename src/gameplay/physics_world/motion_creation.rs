//! c908f0 motion creation on the shared motion/deactivation owners.
use super::BodyPublication;
use crate::gameplay::{
    body_activation::ActivationOwners, motion_info::MotionInfo, space_cells::SpaceCells,
};

pub trait MotionCreationCallbacks {
    fn motion_capacity_requested(&mut self, owners: ActivationOwners<'_>) -> Result<(), String>;
    fn motion_created(
        &mut self,
        owners: ActivationOwners<'_>,
        info: &MotionInfo,
        motion: u32,
    ) -> Result<(), String>;
}

impl BodyPublication<'_> {
    pub fn create_motion(
        &mut self,
        info: &MotionInfo,
        cells: &SpaceCells,
        callbacks: &mut impl MotionCreationCallbacks,
    ) -> Result<u32, String> {
        if !self.motions.has_free_slot() {
            callbacks.motion_capacity_requested(ActivationOwners {
                bodies: self.bodies,
                motions: self.motions,
                deactivation: self.deactivation,
                broadphase: self.broadphase,
            })?;
        }
        let motion = self.motions.reserve()?;
        self.motions.initialize(motion, info, cells)?;
        self.deactivation
            .initialize_motion_state(motion, info.enable_deactivation)?;
        callbacks.motion_created(
            ActivationOwners {
                bodies: self.bodies,
                motions: self.motions,
                deactivation: self.deactivation,
                broadphase: self.broadphase,
            },
            info,
            motion,
        )?;
        Ok(motion)
    }
}

#[cfg(test)]
mod tests;
