//! Attached-body deactivation updates (c92a10), on shared world ownership.
use super::*;

#[cfg(test)]
mod tests;

fn validate_ring(bodies: &BodyStorage, first: u32) -> Result<(), String> {
    let mut id = first;
    for _ in 0..bodies.capacity() {
        id = bodies
            .get(id)
            .ok_or("attached-transform body is missing")?
            .next_attached_body_id;
        if id == first {
            return Ok(());
        }
    }
    Err("attached-transform body ring does not close".into())
}

impl BodyPublication<'_> {
    pub fn prepare_deactivation_contacts(
        &self,
        ids: &[u32],
        pairs: &mut Vec<[u32; 2]>,
    ) -> Result<bool, String> {
        crate::gameplay::deactivation::prepare_contacts(self.bodies, ids, pairs, |bounds, hits| {
            self.broadphase.overlap_bodies(bounds, self.packing, hits)
        })
    }

    /// c92a10 mode0 activates immediately. Its other path calls c93410 with
    /// zero input bodies (which returns true), queues an inactive body's island
    /// and complete attachment ring, then visits each member's constraints.
    pub fn update_attached_bodies(
        &mut self,
        first: u32,
        mode: i32,
        callbacks: &mut impl PublicationCallbacks,
    ) -> Result<(), String> {
        if !self.enable_deactivation
            || self
                .bodies
                .get(first)
                .ok_or("attached-transform first body is missing")?
                .broadphase_id
                == u32::MAX
        {
            return Ok(());
        }
        if mode == 0 {
            self.activate(first, callbacks)?;
            return Ok(());
        }
        validate_ring(self.bodies, first)?;
        let mut pairs = Vec::new();
        let prepared = self.prepare_deactivation_contacts(&[], &mut pairs)?;
        if self.bodies.get(first).unwrap().flags & 9 == 0 {
            validate_ring(self.bodies, first)?;
            if !prepared {
                self.activate(first, callbacks)?;
            }
            self.deactivation.queue_island(self.bodies, first, 0)?;
            queue_attached_bodies(self.bodies, first, &mut self.deactivation.pending_bodies)?;
        } else if !prepared {
            self.activate(first, callbacks)?;
        }
        let mut id = first;
        for _ in 0..self.bodies.capacity() {
            self.constraints.activate_neighbors(
                self.enable_deactivation,
                self.bodies,
                id,
                |first, second| {
                    self.deactivation.link_bodies(
                        self.bodies,
                        self.motions,
                        first,
                        second,
                        self.step_index,
                    )
                },
            )?;
            id = self
                .bodies
                .get(id)
                .ok_or("attached-transform member disappeared")?
                .next_attached_body_id;
            if id == first {
                return Ok(());
            }
        }
        Err("attached-transform body ring does not close after constraints".into())
    }
}
