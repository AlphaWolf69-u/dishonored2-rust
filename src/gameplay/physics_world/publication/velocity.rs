//! Body velocity publication (c85dc0) and ring bounds refresh (c88dd0).
use super::*;

#[cfg(test)]
mod tests;

impl BodyPublication<'_> {
    /// World-space requested velocities; angular storage remains motion-local.
    /// Matching XYZ velocities return before activation and bounds work. W is
    /// ignored for admission but all four lanes are stored on a real change.
    pub fn set_velocity(
        &mut self,
        id: u32,
        linear: [f32; 4],
        angular: [f32; 4],
        mode: i32,
        callbacks: &mut impl TransformCallbacks,
    ) -> Result<bool, String> {
        let body = self.bodies.get(id).ok_or("velocity body is missing")?;
        if body.flags & 2 == 0 {
            return Ok(false);
        }
        let motion_id = body.motion_id;
        let motion = self
            .motions
            .get(motion_id)
            .ok_or("velocity motion is missing")?;
        let same = |first: [f32; 4], second: [f32; 4]| {
            (0..3).all(|i| {
                crate::float::subtract(first[i], second[i]).abs() <= f32::from_bits(0x3400_0000)
            })
        };
        if same(linear, motion.linear_velocity) && same(angular, motion.world_angular_velocity()) {
            return Ok(false);
        }
        if body.flags & 9 == 0
            && body.broadphase_id != u32::MAX
            && mode == 0
            && !self.activate(id, callbacks)?
        {
            return Ok(false);
        }
        // Activation callbacks may change the pose; retain the captured motion
        // identity, and read its current orientation for angular conversion.
        let motion = self
            .motions
            .get_mut(motion_id)
            .ok_or("velocity motion disappeared")?;
        motion.linear_velocity = linear;
        motion.set_world_angular_velocity(angular);
        self.refresh_velocity_bounds(id, motion_id, callbacks)?;
        callbacks.transform_changed(
            ActivationOwners {
                bodies: self.bodies,
                motions: self.motions,
                deactivation: self.deactivation,
                broadphase: self.broadphase,
            },
            id,
            false,
        )?;
        Ok(true)
    }

    /// Refresh all attached members using the same captured motion. Inactive
    /// members are marked together after the traversal; active ones stay in the
    /// solver's regular update path. Source scratch allocation is a host Vec.
    pub fn refresh_velocity_bounds(
        &mut self,
        first: u32,
        motion_id: u32,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        let mut inactive = Vec::new();
        let mut current = first;
        for _ in 0..self.bodies.capacity() {
            let motion = self
                .motions
                .get(motion_id)
                .ok_or("bounds-refresh motion is missing")?;
            let body = self
                .bodies
                .get_mut(current)
                .ok_or("bounds-refresh attachment body is missing")?;
            let quality = self
                .qualities
                .get(body.quality_id)
                .ok_or("bounds-refresh body quality is missing")?;
            body.try_update_swept_bounds(
                motion,
                quality.requested_flags,
                self.tolerance,
                self.step,
                self.packing,
                |shape, frame| callbacks.bounds(shape, frame),
            )?;
            if body.flags & 8 == 0 {
                inactive.push(current);
            }
            current = body.next_attached_body_id;
            if current == first {
                if !inactive.is_empty() {
                    self.broadphase.mark_bodies(inactive);
                }
                return Ok(());
            }
        }
        Err("bounds-refresh attachment ring does not close".into())
    }
}
