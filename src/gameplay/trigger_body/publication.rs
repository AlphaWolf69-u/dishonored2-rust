//! Trigger779af0 composed with real component, physics and resource owners.
use super::{transform::TriggerTransformWorld, TriggerDataStore};
use crate::gameplay::{
    collision_user_data::velocity::{publish_velocities, VelocityPublication},
    component_state::entity::ComponentHandle,
    motion_properties::MotionPropertiesLibrary,
    physics_world::{
        commands::PhysicsCommands, BodyPublication, PublicationContext, PublicationEvents,
        PublicationGeometry,
    },
    shape_transform::ShapeMatrix,
    space_cells::SpaceCells,
};

#[cfg(test)]
mod tests;

pub struct TriggerPublication<'a, 'world, 'resources, G, E, S, K> {
    pub reference: ComponentHandle,
    pub world: &'a mut BodyPublication<'world>,
    pub callbacks: &'a mut PublicationContext<'resources, G, E>,
    pub properties: &'a MotionPropertiesLibrary<S, K>,
    pub commands: &'a PhysicsCommands,
    pub cells: &'a SpaceCells,
    pub grow_one_at_a_time: bool,
}

impl<G: PublicationGeometry, E: PublicationEvents + TriggerDataStore, S, K> TriggerTransformWorld
    for TriggerPublication<'_, '_, '_, G, E, S, K>
{
    fn deferred(&mut self) -> bool {
        self.callbacks.events.deferred()
    }
    fn body_id(&self) -> Result<u32, String> {
        self.callbacks.events.referenced_body(self.reference)
    }
    fn set_transform(&mut self, body: u32, frame: ShapeMatrix) -> Result<(), String> {
        self.world
            .set_transform(body, frame, 0, self.cells, self.callbacks)?;
        Ok(())
    }
    fn queue_transform(&mut self, body: u32, frame: ShapeMatrix) -> Result<(), String> {
        self.commands
            .enqueue_transform(body, frame, 0, self.grow_one_at_a_time)
    }
    fn reset_motion(&mut self, deferred: bool) -> Result<(), String> {
        publish_velocities(
            &mut VelocityPublication {
                world: self.world,
                callbacks: self.callbacks,
                properties: self.properties,
                commands: self.commands,
                grow_one_at_a_time: self.grow_one_at_a_time,
            },
            self.reference,
            [0.; 4],
            [0.; 4],
            deferred,
        )
    }
    fn update_water_plane(&mut self, frame: ShapeMatrix) -> Result<(), String> {
        let Some(handle) = self.callbacks.events.water_component(self.reference)? else {
            return Ok(());
        };
        let body = self.callbacks.events.referenced_body(self.reference)?;
        let water = self.callbacks.events.water_mut(handle)?;
        let geometry = &mut self.callbacks.geometry;
        let bodies = &*self.world.bodies;
        // The phase gate runs before shape/body dereference. Use the requested
        // frame even when the body move is queued and its stored pose is older.
        water.update_plane(|| {
            let body = bodies
                .get_by_index(body & 0xffffff)
                .ok_or("water-plane body is missing")?;
            geometry.bounds(body.shape, frame)
        })?;
        Ok(())
    }
}
