//!651b80/809880: canonical fluid contacts borrow the existing body/shape owners.
use super::*;
use crate::gameplay::{
    collision_user_data::CollisionUserData,
    entity_events::{EntityEvents, Event, Payload},
    entity_lifetime::{EntityId, LiveEntities},
    water_contacts::{WaterContactState, WaterContactWorld, WaterDensityWorld},
};

pub(super) struct ContactHost<'a> {
    pub world: &'a WorldStorage,
    pub geometry: &'a TriggerAssets,
    pub waters: &'a ComponentPool<WaterComponent>,
    pub rigid: [(
        i32,
        &'a ComponentPool<crate::app::rigid_body::RigidBodyComponent>,
    ); 2],
    pub water_class: i32,
    pub entities: &'a [PhysicsEntity],
    pub notifications: &'a mut std::collections::VecDeque<(EntityId, bool)>,
}
impl ContactHost<'_> {
    fn body(&self, index: u32) -> Result<&crate::gameplay::collision_body::CollisionBody, String> {
        self.world
            .bodies
            .get_by_index(index)
            .ok_or_else(|| "water contact body slot absent".into())
    }
    fn rigid(&self, index: u32) -> Result<&crate::app::rigid_body::RigidBodyComponent, String> {
        let body = self.body(index)?;
        let handle = CollisionUserData::decode(body.user_data)?.component;
        let pool = self
            .rigid
            .iter()
            .find(|(class, _)| *class == handle.class_id)
            .ok_or("water body is not PhysicsBase")?
            .1;
        let component = pool
            .get(handle.slot)
            .ok_or("water collision component absent")?;
        if component.body() != Some(body.id) {
            return Err("water collision component body generation differs".into());
        }
        Ok(component)
    }
    fn notify(&mut self, index: u32, entered: bool) -> Result<(), String> {
        let entity = self
            .rigid(index)?
            .owner
            .entity
            .ok_or("water body entity absent")?;
        let id = self
            .entities
            .get(entity)
            .ok_or("water body entity header absent")?
            .shared;
        self.notifications.push_back((id, entered));
        Ok(())
    }
}
impl WaterDensityWorld for ContactHost<'_> {
    fn prepare_body(&mut self, index: u32) -> Result<(), String> {
        self.notify(index, true)
    }
    fn density_override(&self, index: u32) -> Result<f32, String> {
        Ok(self
            .rigid(index)?
            .definition()
            .ok_or("water rigid declaration absent")?
            .settings
            .density_override
            .unwrap_or(0.))
    }
    fn body_mass(&self, index: u32) -> Result<f32, String> {
        Ok(self
            .world
            .bodies
            .mass_properties(self.body(index)?.id)
            .map(|mass| mass.mass)
            .unwrap_or(-1.))
    }
    fn shape_volume(&self, index: u32) -> Result<Option<[f32; 4]>, String> {
        let shape = self
            .geometry
            .shapes
            .get(self.body(index)?.shape)
            .and_then(Option::as_ref)
            .ok_or("water physical shape lease absent")?;
        Ok(shape
            .physics_property_if_present()?
            .map(|property| property.geometry.volume))
    }
    fn body_label(&self, index: u32) -> Result<String, String> {
        Ok(self
            .entities
            .get(
                self.rigid(index)?
                    .owner
                    .entity
                    .ok_or("water body entity absent")?,
            )
            .ok_or("water body header absent")?
            .name
            .clone())
    }
}
impl WaterContactWorld for ContactHost<'_> {
    fn water_state(&self, handle: ComponentHandle) -> Result<WaterContactState, String> {
        if handle.class_id != self.water_class {
            return Err("water contact class differs".into());
        }
        Ok(self
            .waters
            .get(handle.slot)
            .ok_or("water contact component absent")?
            .into())
    }
    //809880 false reports entity/model leaving water; it does not sleep physics.
    fn deactivate_body(&mut self, index: u32) -> Result<(), String> {
        self.notify(index, false)
    }
}
impl PhysicsScene {
    pub(super) fn water_body_contact(
        &mut self,
        world: &mut WorldStorage,
        geometry: &mut TriggerAssets,
        water: ComponentHandle,
        body: u32,
        entered: bool,
        _events: &mut EntityEvents,
    ) -> Result<(), String> {
        let data = CollisionUserData::decode(
            world
                .bodies
                .get(body)
                .ok_or("water contact body generation stale")?
                .user_data,
        )?;
        //4d69d0 RTTI admits PhysicsBase; Trigger and PhysicsWorld are not cast.
        if !self.is_rigid_class(data.component.class_id) {
            return Ok(());
        }
        self.contacts.contact(
            body,
            water,
            entered,
            &mut ContactHost {
                world,
                geometry,
                waters: &self.waters,
                rigid: [
                    (self.classes[8], &self.rigid_bodies),
                    (self.classes[12], &self.throwables),
                ],
                water_class: self.classes[1],
                entities: &self.entities,
                notifications: &mut self.water_notifications,
            },
        )
    }
    pub(super) fn notify_water_contacts(
        &mut self,
        water: ComponentHandle,
        stopped: bool,
    ) -> Result<(), String> {
        let shared = self.world.clone();
        let guard = shared.read().map_err(|_| "water contact world poisoned")?;
        let mut host = ContactHost {
            world: &guard.storage,
            geometry: &guard.geometry,
            waters: &self.waters,
            rigid: [
                (self.classes[8], &self.rigid_bodies),
                (self.classes[12], &self.throwables),
            ],
            water_class: self.classes[1],
            entities: &self.entities,
            notifications: &mut self.water_notifications,
        };
        if stopped {
            self.contacts.stop_component(water, &mut host)
        } else {
            self.contacts.notify_component(water, &mut host)
        }
    }
    pub(super) fn drain_water_notifications(
        &mut self,
        world: &mut bevy::prelude::World,
        live: &LiveEntities,
        events: &mut EntityEvents,
    ) -> Result<(), String> {
        //Keep an accepted callback until actual model mutation succeeds.
        while let Some(&(id, entered)) = self.water_notifications.front() {
            if live.get(id).is_some() {
                crate::app::model_motion::set_in_water(world, live, id, entered)?;
                events.post(Event {
                    id: if entered { 0x12c } else { 0x12d },
                    target: None,
                    payload: Payload::Entity(id),
                });
            }
            self.water_notifications.pop_front();
        }
        Ok(())
    }
}
