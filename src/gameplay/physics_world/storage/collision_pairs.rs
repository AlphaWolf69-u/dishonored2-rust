//! arkCollisionFilter's actual body-pair ignores (093b00) and the admitted
//! component userdata virtuals consumed by093a40. No shape/entity-equality
//! shortcut substitutes for an authored self-collision matrix.
use crate::gameplay::{collision_body::CollisionBody, collision_user_data::CollisionUserData};
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct CollisionPairs {
    /// Source08f940 initializes the table pointer/count empty and mask-1.
    /// Rust's empty HashSet similarly owns no allocation until a real request.
    ignored: HashSet<[u32; 2]>,
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
        if self.ignored.contains(&key(bodies[0].id, bodies[1].id)) {
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
    /// Mirrors a successful actual filter-table insertion/removal request.
    /// Current typed producers issue none; never infer ignores from proximity,
    /// owner equality or a projectile's visual parent.
    pub(crate) fn publish_ignore(&mut self, bodies: [u32; 2]) {
        self.ignored.insert(key(bodies[0], bodies[1]));
    }
    pub(crate) fn retire_ignore(&mut self, bodies: [u32; 2]) {
        self.ignored.remove(&key(bodies[0], bodies[1]));
    }
    pub(crate) fn retire_body(&mut self, body: u32) {
        if !self.ignored.is_empty() {
            self.ignored.retain(|pair| !pair.contains(&body));
        }
    }
}
impl super::WorldStorage {
    pub(crate) fn register_collision_class_without_matrix(&mut self, class: i32) {
        self.collision_pairs.register_without_matrix(class);
    }
}
