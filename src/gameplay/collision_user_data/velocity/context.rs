//! Concrete18d020 bindings to the shared world, property table and command queue.
use super::{BodyVelocityWorld, CollisionVelocities};
use crate::gameplay::{
    motion_properties::MotionPropertiesLibrary,
    physics_world::{commands::PhysicsCommands, BodyPublication, TransformCallbacks},
};

/// References and collision payloads belong to the same observer/component
/// owner. An activation observer may replace a reference; each read stays live.
pub trait VelocityDataStore {
    type Reference: Copy;
    fn referenced_body(&self, reference: Self::Reference) -> Result<u32, String>;
    fn velocities(&mut self, user_data: u64) -> Result<&mut CollisionVelocities, String>;
}

pub struct VelocityPublication<'a, 'world, C, S, K> {
    pub world: &'a mut BodyPublication<'world>,
    pub callbacks: &'a mut C,
    pub properties: &'a MotionPropertiesLibrary<S, K>,
    pub commands: &'a PhysicsCommands,
    pub grow_one_at_a_time: bool,
}

impl<C: TransformCallbacks + VelocityDataStore, S, K> BodyVelocityWorld
    for VelocityPublication<'_, '_, C, S, K>
{
    type Reference = C::Reference;
    fn body_id(&self, reference: Self::Reference) -> Result<u32, String> {
        self.callbacks.referenced_body(reference)
    }
    fn velocity_scale(&self, body: u32) -> Result<f32, String> {
        self.properties
            .body_velocity_scale(self.world.bodies, self.world.motions, body)
    }
    fn set_velocity(
        &mut self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String> {
        self.world
            .set_velocity(body, linear, angular, 0, self.callbacks)?;
        Ok(())
    }
    fn queue_velocity(
        &mut self,
        body: u32,
        linear: [f32; 4],
        angular: [f32; 4],
    ) -> Result<(), String> {
        self.commands
            .enqueue_velocity(body, linear, angular, self.grow_one_at_a_time)
    }
    fn collision_velocities(&mut self, body: u32) -> Result<&mut CollisionVelocities, String> {
        let data = self
            .world
            .bodies
            .get_by_index(body & 0xffffff)
            .ok_or("velocity payload body is missing")?
            .user_data;
        self.callbacks.velocities(data)
    }
    fn next_attached_body(&self, body: u32) -> Result<u32, String> {
        Ok(self
            .world
            .bodies
            .get_by_index(body & 0xffffff)
            .ok_or("velocity attachment body is missing")?
            .next_attached_body_id)
    }
    fn body_capacity(&self) -> usize {
        self.world.bodies.capacity()
    }
}
