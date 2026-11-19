//! arkCollisionFilter's actual body-pair ignores (093b00) and the admitted
//! component userdata virtuals consumed by093a40. No shape/entity-equality
//! shortcut substitutes for an authored self-collision matrix.
use crate::gameplay::{collision_body::CollisionBody, collision_user_data::CollisionUserData};
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct CollisionPairs {
    /// Source08f940 initializes the table pointer/count empty and mask-1.
    ///091b50/091e20 retain the actual per-pair reference count.
    ignored: HashMap<[u32; 2], u32>,
    no_matrix_classes: Vec<i32>,
}
fn key(first: u32, second: u32) -> [u32; 2] {
    if first & 0x00ff_ffff < second & 0x00ff_ffff {
        [first, second]
    } else {
        [second, first]
    }
}
impl CollisionPairs {
    /// Register only classes whose actual userdata+20 virtual is proven null.
    /// Rigidbody/Throwable, PhysicsWorld and Trigger share06d040 there; their
    /// +28 virtual181a40 returnsffffffff and is not called with a null matrix.
    pub(crate) fn register_without_matrix(&mut self, class: i32) {
        if !self.no_matrix_classes.contains(&class) {
            self.no_matrix_classes.push(class);
        }
    }
    pub(crate) fn accepts(&self, bodies: [&CollisionBody; 2]) -> Result<bool, String> {
        if self.ignored.get(&key(bodies[0].id, bodies[1].id)).is_some_and(|&count| count != 0) {
            return Ok(false);
        }
        for body in bodies {
            let component = CollisionUserData::decode(body.user_data)?.component;
            if !self.no_matrix_classes.contains(&component.class_id) {
                return Err(format!(
                    "collision userdata class {} has no admitted self-collision matrix policy",
                    component.class_id
                ));
            }
        }
        //093a40 tests a matrix only for equal component ownerentities. For
        // these admitted classes virtual+20 isnull, so both ownerrelations
        // permitcollision without loading/scanning either entity hierarchy.
        Ok(true)
    }
    ///Raw091b50 table mutation; the world request owns collision invalidation.
    ///Never infer an ignore from visual parenting or numerical joint existence.
    pub(crate) fn publish_ignore(&mut self, bodies: [u32; 2]) {
        let count = self.ignored.entry(key(bodies[0], bodies[1])).or_insert(0);
        *count = count.wrapping_add(1);
    }
    pub(crate) fn retire_ignore(&mut self, bodies: [u32; 2]) {
        let pair = key(bodies[0], bodies[1]);
        if let Some(count) = self.ignored.get_mut(&pair) {
            if *count == 0 { return; }
            if *count > 1 { *count -= 1; } else { self.ignored.remove(&pair); }
        }
    }
    pub(crate) fn retire_body(&mut self, body: u32) {
        if !self.ignored.is_empty() {
            self.ignored.retain(|pair, _| !pair.contains(&body));
        }
    }
    pub(crate) fn retire_bodies(&mut self, bodies: &[u32]) {
        if self.ignored.is_empty() || bodies.is_empty() { return; }
        if bodies.len() == 1 { self.retire_body(bodies[0]); return; }
        //One actual destruction batch, after its callback acknowledgement and
        //before any generation is recycled. No retained reverse-owner cache.
        let retired: std::collections::HashSet<_> = bodies.iter().copied().collect();
        self.ignored.retain(|pair, _| !pair.iter().any(|body| retired.contains(body)));
    }
}

#[derive(Clone, Copy, Debug)]
struct Invalidation {
    body: u32,
    wake: bool,
}
///Only continuation of one accepted851b80 body-batch request. Bodies and
///geometry stay with their existing owners; this retains no roster or cache.
#[derive(Default, Clone, Copy, Debug)]
pub(crate) struct CollisionIgnoreProgress {
    counts: Option<[usize; 2]>,
    next: usize,
    invalidation: Option<Invalidation>,
    complete: bool,
}
impl CollisionIgnoreProgress {
    pub(crate) fn accepted(&self) -> bool { self.counts.is_some() }
    pub(crate) fn complete(&self) -> bool { self.complete }
}
impl super::WorldStorage {
    pub(crate) fn register_collision_class_without_matrix(&mut self, class: i32) {
        self.collision_pairs.register_without_matrix(class);
    }

    ///851b80/091b50: actual PhysicsBase body batches, including the canonical
    ///worldspawn PhysicsWorld list for a NULL endpoint. The caller supplies
    ///those borrowed witnesses; anchor body0 is not a replacement world list.
    pub(crate) fn publish_collision_ignores<G: super::BodyCreationGeometry,
        E: super::PublicationEvents + super::BodyCreationCallbacks>(
        &mut self, geometry: &mut G, events: &mut E, first: &[u32], second: &[u32],
        progress: &mut CollisionIgnoreProgress,
    ) -> Result<(), String> {
        if progress.complete { return Ok(()); }
        if !self.simulation_idle() { return Err("collision ignore requires the actual idle solver boundary".into()); }
        let counts = [first.len(), second.len()];
        let total = counts[0].checked_mul(counts[1]).ok_or("collision ignore batch size overflow")?;
        if let Some(accepted) = progress.counts {
            if accepted != counts { return Err("collision ignore source batches changed during continuation".into()); }
        } else {
            if !self.publication_idle() { return Err("collision ignore awaits actual body publication".into()); }
            //A malformed/stale source is rejected before any reference changes.
            for &body in first.iter().chain(second) {
                if body & 0x00ff_ffff != 0x00ff_ffff {
                    self.bodies.get(body).ok_or("collision ignore source body generation stale")?;
                }
            }
            progress.counts = Some(counts);
        }
        while progress.next < total {
            let pair = [first[progress.next / counts[1]], second[progress.next % counts[1]]];
            if progress.invalidation.is_none() {
                if pair.iter().any(|body| body & 0x00ff_ffff == 0x00ff_ffff) {
                    progress.next += 1;
                    continue;
                }
                let a = self.bodies.get(pair[0]).ok_or("collision ignore first endpoint generation changed")?;
                let b = self.bodies.get(pair[1]).ok_or("collision ignore second endpoint generation changed")?;
                //091b50 selects the dynamic body, choosing the smaller
                //actual bfloat16 bounding radius when both are dynamic.
                let selected = match (a.flags & 2 != 0, b.flags & 2 != 0) {
                    (false, false) => None,
                    (true, false) => Some(a),
                    (false, true) => Some(b),
                    (true, true) => Some(if f32::from_bits(u32::from(b.packed_radius) << 16)
                        < f32::from_bits(u32::from(a.packed_radius) << 16) { b } else { a }),
                };
                let invalidation = selected.filter(|body| body.broadphase_id != u32::MAX)
                    .map(|body| Invalidation { body: body.id, wake: body.flags & 9 == 0 });
                if let Some(selected) = invalidation {
                    self.bodies.validate_collision_invalidation(selected.body)?;
                    let motion = self.bodies.get(selected.body).unwrap().motion_id;
                    self.deactivation.state(motion).ok_or("collision ignore motion owner absent")?;
                }
                self.collision_pairs.publish_ignore(pair);
                progress.invalidation = invalidation;
                if invalidation.is_none() { progress.next += 1; continue; }
            }
            let invalidation = progress.invalidation.ok_or("collision ignore continuation lost invalidation")?;
            let mut activation = std::mem::take(&mut self.activation_scratch);
            let result = self.edit_bodies(geometry, events, |creation| {
                creation.world.invalidate_dynamic_collision_cache(invalidation.body, 0,
                    invalidation.wake, Some(&mut activation), creation.callbacks)?;
                creation.world.bodies.invalidate_collision_history(invalidation.body)
            });
            self.activation_scratch = activation;
            result?;
            progress.invalidation = None;
            progress.next += 1;
        }
        progress.complete = true;
        Ok(())
    }
}
