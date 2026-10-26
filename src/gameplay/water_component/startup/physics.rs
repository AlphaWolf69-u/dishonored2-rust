//!4d6850's physics/material boundaries borrow the level's canonical owners.
use super::{start_water, WaterDeclaration, WaterStartupWorld};
use crate::gameplay::{
    body_activation::ActivationOwners,
    collision_user_data::velocity::VelocityDataStore,
    component_state::{entity::ComponentHandle, prepare_admission},
    physics_world::{
        body_creation::BodyCreationGeometry, storage::WorldStorage, ComponentPublicationEvents,
        PublicationEvent,
    },
    signal::Signal,
    trigger_body::TriggerComponentStore,
    water_component::WaterComponent,
};

pub(crate) trait WaterLevelStartup<G, F> {
    /// Declaration LOD priority and the entity's current LOD priority. These
    /// are independent of the component factory initialization rank.
    fn priorities(&self, reference: ComponentHandle) -> Result<[i32; 2], String>;
    fn trigger(
        &mut self,
        entity: usize,
        components: &TriggerComponentStore<'_>,
    ) -> Result<ComponentHandle, String>;
    fn material_registered(
        &mut self,
        signal: &mut Signal<usize, usize>,
        subscriber: usize,
        callback: usize,
        material: u16,
    );
    ///651d30/650bf0 must visit the real retained water-contact manager.
    fn notify_contacts(
        &mut self,
        reference: ComponentHandle,
        world: &mut WorldStorage,
        geometry: &mut G,
        events: &mut ComponentPublicationEvents<'_, usize, usize, F>,
    ) -> Result<(), String>;
}

pub(crate) struct WaterPhysicsStartup<'a, 'component, G, F, L> {
    pub world: &'a mut WorldStorage,
    pub geometry: &'a mut G,
    pub events: &'a mut ComponentPublicationEvents<'component, usize, usize, F>,
    pub level: &'a mut L,
}
impl<'component, G, F, L> WaterPhysicsStartup<'_, 'component, G, F, L>
where
    G: BodyCreationGeometry,
    L: WaterLevelStartup<G, F>,
    F: FnMut(
        &mut Signal<usize, usize>,
        usize,
        usize,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'component>,
    ) -> Result<(), String>,
{
    pub fn start(
        &mut self,
        reference: ComponentHandle,
        declaration: &WaterDeclaration,
    ) -> Result<(), String> {
        start_water(reference, declaration, self)
    }
}
impl<'component, G, F, L> WaterStartupWorld for WaterPhysicsStartup<'_, 'component, G, F, L>
where
    G: BodyCreationGeometry,
    L: WaterLevelStartup<G, F>,
    F: FnMut(
        &mut Signal<usize, usize>,
        usize,
        usize,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'component>,
    ) -> Result<(), String>,
{
    fn water(&self, reference: ComponentHandle) -> Result<&WaterComponent, String> {
        if reference.class_id != self.events.components.water_class {
            return Err("component is not water".into());
        }
        self.events
            .components
            .waters
            .get(reference.slot)
            .ok_or_else(|| "water component is missing".into())
    }
    fn water_mut(&mut self, reference: ComponentHandle) -> Result<&mut WaterComponent, String> {
        if reference.class_id != self.events.components.water_class {
            return Err("component is not water".into());
        }
        self.events
            .components
            .waters
            .get_mut(reference.slot)
            .ok_or_else(|| "water component is missing".into())
    }
    fn admit(&mut self, reference: ComponentHandle) -> Result<(), String> {
        let [priority, entity_priority] = self.level.priorities(reference)?;
        if prepare_admission(
            &mut self.water_mut(reference)?.owner.flags,
            u8::from(priority < 8),
            priority,
            entity_priority,
        ) {
            self.events.components.waters.schedule(reference.slot)?;
        }
        Ok(())
    }
    fn trigger(&mut self, entity: usize) -> Result<ComponentHandle, String> {
        self.level.trigger(entity, &self.events.components)
    }
    fn trigger_body(&self, trigger: ComponentHandle) -> Result<u32, String> {
        self.events.components.referenced_body(trigger)
    }
    fn shape_bounds(&mut self, body: u32) -> Result<[[f32; 4]; 2], String> {
        let body = self
            .world
            .bodies
            .get_by_index(body & 0xffffff)
            .ok_or("water trigger body is missing")?;
        self.geometry.bounds(body.shape, body.transform)
    }
    fn material_flags(&self, material: usize) -> Result<u8, String> {
        Ok(self
            .world
            .materials
            .get(material)
            .ok_or("water material declaration is missing")?
            .flags)
    }
    fn resolve_material(&mut self, material: usize) -> Result<u16, String> {
        self.world
            .materials
            .resolve(material, |signal, subscriber, callback, material| {
                self.level
                    .material_registered(signal, subscriber, callback, material)
            })
    }
    fn set_material(&mut self, body: u32, material: u16) -> Result<(), String> {
        self.world
            .set_material(self.geometry, self.events, body, material, 0)?;
        Ok(())
    }
    fn notify_contacts(&mut self, reference: ComponentHandle) -> Result<(), String> {
        self.level
            .notify_contacts(reference, self.world, self.geometry, self.events)
    }
}
