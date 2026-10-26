//! Collision-data velocity publication and command routing (18d020).
mod context;
pub use context::{VelocityDataStore, VelocityPublication};

/// The +20..+5f velocity subobject, initialized to zero by18cb90. Identity,
/// event lists and the uninitialized+18 field belong to their separate owners.
#[derive(Clone, Copy, Debug, Default)]
pub struct CollisionVelocities {
    pub scaled_linear: [f32; 4],
    pub scaled_angular: [f32; 4],
    pub requested_linear: [f32; 4],
    pub requested_angular: [f32; 4],
}

#[cfg(test)]
mod tests;

/// The physics owner resolves the live body reference, motion-property scale,
/// real immediate/queued setter, and each body's canonical collision payload.
pub trait BodyVelocityWorld {
    type Reference: Copy;
    fn body_id(&self, reference: Self::Reference) -> Result<u32, String>;
    fn velocity_scale(&self, body: u32) -> Result<f32, String>;
    fn set_velocity(
        &mut self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String>;
    /// Command030/type90200, carrying both scaled vectors and the body ID.
    fn queue_velocity(
        &mut self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String>;
    fn collision_velocities(&mut self, body: u32) -> Result<&mut CollisionVelocities, String>;
    fn next_attached_body(&self, body: u32) -> Result<u32, String>;
    fn body_capacity(&self) -> usize;
}

/// Publish cached requested/scaled velocities even if the physics setter
/// rejects activation or finds no change. The live first-body reference can
/// change during that setter; source ring termination reloads it each time.
pub fn publish_velocities<W: BodyVelocityWorld>(
    world: &mut W,
    reference: W::Reference,
    linear: [f32; 4],
    angular: [f32; 4],
    deferred: bool,
) -> Result<(), String> {
    let mut current = world.body_id(reference)?;
    let scale = world.velocity_scale(current)?;
    let scaled_linear = linear.map(|value| crate::float::multiply(value, scale));
    let scaled_angular = angular.map(|value| crate::float::multiply(scale, value));
    if deferred {
        world.queue_velocity(current, scaled_linear, scaled_angular)?;
    } else {
        world.set_velocity(current, scaled_linear, scaled_angular)?;
    }
    for _ in 0..world.body_capacity() {
        let data = world.collision_velocities(current)?;
        data.scaled_linear = scaled_linear;
        data.scaled_angular = scaled_angular;
        data.requested_linear[..3].copy_from_slice(&linear[..3]);
        data.requested_angular = angular;
        current = world.next_attached_body(current)?;
        if current == world.body_id(reference)? || current & 0xffffff == 0xffffff {
            return Ok(());
        }
    }
    Err("velocity publication attachment ring does not close".into())
}
