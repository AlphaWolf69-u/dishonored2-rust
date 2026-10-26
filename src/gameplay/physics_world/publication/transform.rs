//! World pose publication (c85860/c88590) on the shared physics owners.
use super::*;
use crate::gameplay::{shape_transform::matches_transform, space_cells::SpaceCells};

#[cfg(test)]
mod tests;

/// Resource/event operations retained by the physics world. These are required
/// live owners, not substitutes for missing geometry or deactivation behavior.
pub trait TransformCallbacks: PublicationCallbacks {
    /// Full12816e0 body bounds/cache refresh, before distance accumulation.
    fn refresh_transform_cache(
        &mut self,
        owners: ActivationOwners<'_>,
        body: u32,
    ) -> Result<(), String>;
    /// c92a10's actual deactivation/attachment update, with the source mode.
    fn update_attached_transform(
        &mut self,
        owners: ActivationOwners<'_>,
        body: u32,
        mode: i32,
    ) -> Result<(), String>;
    /// Emit the static (+898) or general (+8c0) world Signal, respectively.
    fn transform_changed(
        &mut self,
        owners: ActivationOwners<'_>,
        body: u32,
        static_only: bool,
    ) -> Result<(), String>;
}

impl BodyPublication<'_> {
    /// c85860 rejects an unchanged XYZ pose using immutable141ad2734. A real
    /// change retains the body's cached W lanes and publishes static/general
    /// observer signals only after all motion/broadphase updates have finished.
    pub fn set_transform(
        &mut self,
        id: u32,
        frame: ShapeMatrix,
        mode: i32,
        cells: &SpaceCells,
        callbacks: &mut impl TransformCallbacks,
    ) -> Result<bool, String> {
        let body = self.bodies.get(id).ok_or("transformed body is missing")?;
        if matches_transform(body.transform, frame, f32::from_bits(0x3400_0000)) {
            return Ok(false);
        }
        let orientation =
            crate::assets::physworld::basis_quaternion([frame[0], frame[1], frame[2]]);
        self.bodies.get_mut(id).unwrap().set_transform(frame);
        self.update_motion_transform(id, Some(orientation), false, mode, cells, callbacks)?;
        if self
            .bodies
            .get(id)
            .ok_or("transformed body disappeared")?
            .motion_id
            == 0
        {
            callbacks.transform_changed(
                ActivationOwners {
                    bodies: self.bodies,
                    motions: self.motions,
                    deactivation: self.deactivation,
                    broadphase: self.broadphase,
                },
                id,
                true,
            )?;
        }
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

    /// Complete c88590 sequencing. Geometry, activation, cell lists and distance
    /// counters use the same owners as body creation and world contact queries.
    pub fn update_motion_transform(
        &mut self,
        id: u32,
        orientation: Option<[f32; 4]>,
        retain_center: bool,
        mode: i32,
        cells: &SpaceCells,
        callbacks: &mut impl TransformCallbacks,
    ) -> Result<(), String> {
        let body = self
            .bodies
            .get_mut(id)
            .ok_or("motion-transform body is missing")?;
        body.status |= 0x80;
        let motion_id = body.motion_id;
        if motion_id == 0 {
            body.refresh_inverse_frame();
            if body.broadphase_id != u32::MAX {
                self.bodies.mark_bounds_changed(id)?;
            }
            let body = self.bodies.get(id).unwrap();
            let previous = body.bounds;
            let bounds = callbacks.bounds(body.selected_shape(3)?, body.transform)?;
            let bounds = self.packing.pack_bounds(bounds, self.tolerance);
            if bounds.words != self.bodies.get(id).unwrap().bounds.words {
                self.bodies.get_mut(id).unwrap().bounds = bounds;
                if self.bodies.get(id).unwrap().broadphase_id != u32::MAX {
                    self.broadphase.mark_bodies([id]);
                    self.bodies.get_mut(id).unwrap().status |= 0x10;
                    if mode == 0 {
                        self.wake_overlaps_except(previous, None, callbacks)?;
                        let bounds = self
                            .bodies
                            .get(id)
                            .ok_or("transformed static body disappeared")?
                            .bounds;
                        self.wake_overlaps_except(bounds, Some(id), callbacks)?;
                    }
                }
            }
        } else {
            let body = self.bodies.get(id).unwrap();
            let motion = self
                .motions
                .get_mut(motion_id)
                .ok_or("transformed motion is missing")?;
            let distances = motion.update_body_pose(body, orientation, retain_center);
            let cell = cells.select(motion.pose.center)?;
            if cell != motion.cell {
                let active = motion.solver_id != 0 && motion.solver_id != 0x7fff_ffff;
                if active {
                    self.motions.remove_from_solver(motion_id)?;
                }
                self.motions.get_mut(motion_id).unwrap().cell = cell;
                if active {
                    self.motions.add_to_solver(motion_id)?;
                }
                let first = self.motions.get(motion_id).unwrap().first_body_id;
                self.bodies.update_attachment_cell(first, cell)?;
            }
            let motions = &mut *self.motions;
            let deactivation = &mut *self.deactivation;
            let broadphase = &mut *self.broadphase;
            self.bodies.accumulate_transform_distance(
                id,
                [distances[0]; 4],
                [distances[1]; 4],
                self.step.contact_distance_scale,
                |bodies, id| {
                    callbacks.refresh_transform_cache(
                        ActivationOwners {
                            bodies,
                            motions,
                            deactivation,
                            broadphase,
                        },
                        id,
                    )
                },
            )?;
            if self.enable_deactivation {
                callbacks.update_attached_transform(
                    ActivationOwners {
                        bodies: self.bodies,
                        motions: self.motions,
                        deactivation: self.deactivation,
                        broadphase: self.broadphase,
                    },
                    id,
                    mode,
                )?;
            }
            self.broadphase.mark_bodies([id]);
        }
        let body = self
            .bodies
            .get_mut(id)
            .ok_or("transformed body disappeared before cache invalidation")?;
        if body.broadphase_id != u32::MAX {
            body.tim_angle = u16::MAX;
            body.max_tim_distance = u16::MAX;
            body.angular_tim_distance = u16::MAX;
        }
        Ok(())
    }
}
