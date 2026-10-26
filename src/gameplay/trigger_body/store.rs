//! Borrowed typed pools and the level's single movement-body interface table.
use super::TriggerComponent;
use crate::gameplay::{
    collision_user_data::{
        velocity::{CollisionVelocities, VelocityDataStore},
        CollisionUserData,
    },
    component_state::{entity::ComponentHandle, pool::ComponentPool, ComponentOwner},
    physics_actor::PhysicsActorLinks,
    water_component::WaterComponent,
};

///8a7820 reads two world bytes and the component manager's+c34 byte. The level
/// supplies their current values; constructor defaults are not assumed here.
pub struct DeferredPhysicsState {
    pub world_flags: [u8; 2],
    pub component_flag: u8,
}
impl DeferredPhysicsState {
    pub fn is_deferred(&self) -> bool {
        self.world_flags[0] != 0 || self.world_flags[1] != 0 || self.component_flag != 0
    }
}

pub trait TriggerDataStore: VelocityDataStore<Reference = ComponentHandle> {
    fn deferred(&self) -> bool;
    fn water_component(&self, trigger: ComponentHandle) -> Result<Option<ComponentHandle>, String>;
    fn water_mut(&mut self, water: ComponentHandle) -> Result<&mut WaterComponent, String>;
}

/// Class IDs come from installed metadata. Unsupported collision component
/// classes remain errors; they are never interpreted as trigger payloads.
pub struct TriggerComponentStore<'a> {
    pub trigger_class: i32,
    pub water_class: i32,
    pub triggers: &'a mut ComponentPool<TriggerComponent>,
    pub waters: &'a mut ComponentPool<WaterComponent>,
    pub actors: &'a mut PhysicsActorLinks,
    pub deferred: &'a mut DeferredPhysicsState,
}
impl TriggerComponentStore<'_> {
    /// The same owners used by entity registration and collision-hit resolution.
    /// Other component classes require their actual typed pool in the level.
    pub fn component_owner(&self, handle: ComponentHandle) -> Option<&ComponentOwner> {
        if handle.class_id == self.trigger_class {
            self.triggers.get(handle.slot).map(|value| &value.owner)
        } else if handle.class_id == self.water_class {
            self.waters.get(handle.slot).map(|value| &value.owner)
        } else {
            None
        }
    }
    pub fn trigger(&self, handle: ComponentHandle) -> Result<&TriggerComponent, String> {
        if handle.class_id != self.trigger_class {
            return Err("component is not a trigger".into());
        }
        self.triggers
            .get(handle.slot)
            .ok_or("trigger component is missing".into())
    }
    pub fn trigger_mut(
        &mut self,
        handle: ComponentHandle,
    ) -> Result<&mut TriggerComponent, String> {
        if handle.class_id != self.trigger_class {
            return Err("component is not a trigger".into());
        }
        self.triggers
            .get_mut(handle.slot)
            .ok_or("trigger component is missing".into())
    }
}
impl VelocityDataStore for TriggerComponentStore<'_> {
    type Reference = ComponentHandle;
    fn referenced_body(&self, trigger: ComponentHandle) -> Result<u32, String> {
        let trigger = self.trigger(trigger)?;
        Ok(self
            .actors
            .bodies
            .get(trigger.movement_body)
            .ok_or("trigger movement interface is missing")?
            .body_id)
    }
    fn velocities(&mut self, user_data: u64) -> Result<&mut CollisionVelocities, String> {
        Ok(&mut self
            .trigger_mut(CollisionUserData::decode(user_data)?.component)?
            .velocities)
    }
}
impl TriggerDataStore for TriggerComponentStore<'_> {
    fn deferred(&self) -> bool {
        self.deferred.is_deferred()
    }
    fn water_component(&self, trigger: ComponentHandle) -> Result<Option<ComponentHandle>, String> {
        Ok(self.trigger(trigger)?.water)
    }
    fn water_mut(&mut self, water: ComponentHandle) -> Result<&mut WaterComponent, String> {
        if water.class_id != self.water_class {
            return Err("component is not water".into());
        }
        self.waters
            .get_mut(water.slot)
            .ok_or("water component is missing".into())
    }
}
