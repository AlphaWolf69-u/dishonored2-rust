//! Material publication and collision-history invalidation (c876a0/c87dc0).
use super::*;

impl BodyPublication<'_> {
    pub fn invalidate_collision_cache(
        &mut self,
        id: u32,
        mode: i32,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        let body = self
            .bodies
            .get_by_index(id & 0xffffff)
            .ok_or("invalidated collision body is missing")?;
        if body.broadphase_id == u32::MAX {
            return Ok(());
        }
        if body.flags & 2 == 0 {
            let bounds = body.bounds;
            if mode == 0 {
                self.wake_overlaps(bounds, callbacks)?;
            } else {
                let mut hits = Vec::new();
                self.broadphase
                    .overlap_bodies(bounds, self.packing, &mut hits)?;
                for hit in hits {
                    if self
                        .bodies
                        .get_by_index(hit & 0xffffff)
                        .ok_or("collision overlap body is missing")?
                        .flags
                        & 9
                        == 0
                    {
                        self.deactivation.queue_island(self.bodies, hit, 0)?;
                    }
                }
            }
        } else {
            let wake = body.flags & 9 == 0;
            self.invalidate_dynamic_collision_cache(id, mode, wake, None, callbacks)?;
        }
        self.bodies.invalidate_collision_history(id)
    }

    ///The dynamic branch of c87dc0, shared by material changes and091b50.
    ///The request retains `wake` and the existing activation controller when
    ///a real observer fails, so retry never selects a different branch merely
    ///because the first callback already changed the body's active bit.
    pub(crate) fn invalidate_dynamic_collision_cache(
        &mut self, id: u32, mode: i32, wake: bool,
        activation: Option<&mut crate::gameplay::body_activation::BodyActivationController>,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        let body = self.bodies.get_by_index(id & 0xffffff)
            .ok_or("invalidated dynamic collision body is missing")?;
        if body.flags & 2 == 0 {
            return Err("dynamic collision invalidation received a nondynamic body".into());
        }
        if wake {
            if mode == 0 {
                if let Some(activation) = activation {
                    self.activate_body_resumable(id, activation, callbacks)?;
                } else {
                    self.set_activation(id, false, callbacks)?;
                }
            } else {
                self.deactivation.queue_island(self.bodies, id, 0)?;
            }
        } else {
            self.deactivation.state_mut(body.motion_id)
                .ok_or("collision invalidation motion state is missing")?.restart();
        }
        Ok(())
    }

    /// A material change uses the same general world signal as a transform.
    /// Mode zero refreshes collision history first; other modes skip that call.
    pub fn set_material(
        &mut self,
        id: u32,
        material: u16,
        mode: i32,
        callbacks: &mut impl TransformCallbacks,
    ) -> Result<bool, String> {
        let body = self
            .bodies
            .get_by_index_mut(id & 0xffffff)
            .ok_or("material body is missing")?;
        if body.material_id == material {
            return Ok(false);
        }
        body.material_id = material;
        if mode == 0 {
            self.invalidate_collision_cache(id, 0, callbacks)?;
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
}
