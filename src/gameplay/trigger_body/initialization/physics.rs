//! Bind779530 to the same physics buffers and component pools as publication.
//! The level supplies entity/declaration ownership; this adapter neither creates
//! a collision world nor stores a second body/component lookup table.
use super::{
    initialize_trigger, LoadedTriggerShape, TriggerDependency, TriggerInitializationWorld,
};
use crate::{
    assets::physworld::body::BodyInfo,
    gameplay::{
        body_activation::ActivationOwners,
        body_storage::BodyActivation,
        component_state::entity::ComponentHandle,
        materials::MaterialOwner,
        physics_world::{
            body_creation::BodyCreationGeometry, storage::WorldStorage, BodyAddition,
            ComponentPublicationEvents, PublicationEvent,
        },
        signal::Signal,
        trigger_body::{TriggerCollisionSettings, TriggerComponent, TriggerComponentStore},
    },
};

/// Required level operations. Dependencies must use EntityComponents lookup
/// and real pool state; a declaration name alone is not reference admission.
pub(crate) trait TriggerLevelInitialization<G, F> {
    fn load_shape(
        &mut self,
        entity: usize,
        geometry: &mut G,
    ) -> Result<Option<LoadedTriggerShape>, String>;
    fn dependency(
        &mut self,
        entity: usize,
        kind: TriggerDependency,
        components: &TriggerComponentStore<'_>,
    ) -> Result<Option<ComponentHandle>, String>;
    fn position(&self, entity: usize) -> Result<[f32; 3], String>;
    fn collision_settings(&self, declaration: usize) -> Result<TriggerCollisionSettings, String>;
    fn trigger_material(&self, materials: &MaterialOwner<usize, usize>) -> Result<u16, String>;
    fn mark_entity_keyframed(&mut self, entity: usize) -> Result<(), String>;
    fn report_missing_shape(&mut self, entity: usize) -> Result<(), String>;
    fn release_failed(
        &mut self,
        reference: ComponentHandle,
        world: &mut WorldStorage,
        geometry: &mut G,
        events: &mut ComponentPublicationEvents<'_, usize, usize, F>,
    ) -> Result<(), String>;
}

pub(crate) struct TriggerPhysicsInitialization<'a, 'component, G, F, L> {
    pub world: &'a mut WorldStorage,
    pub geometry: &'a mut G,
    pub events: &'a mut ComponentPublicationEvents<'component, usize, usize, F>,
    pub level: &'a mut L,
}
impl<'component, G, F, L> TriggerPhysicsInitialization<'_, 'component, G, F, L>
where
    G: BodyCreationGeometry,
    L: TriggerLevelInitialization<G, F>,
    F: FnMut(
        &mut Signal<usize, usize>,
        usize,
        usize,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'component>,
    ) -> Result<(), String>,
{
    pub fn initialize(&mut self, reference: ComponentHandle) -> Result<(), String> {
        initialize_trigger(reference, self)
    }
}
impl<'component, G, F, L> TriggerInitializationWorld
    for TriggerPhysicsInitialization<'_, 'component, G, F, L>
where
    G: BodyCreationGeometry,
    L: TriggerLevelInitialization<G, F>,
    F: FnMut(
        &mut Signal<usize, usize>,
        usize,
        usize,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'component>,
    ) -> Result<(), String>,
{
    fn trigger(&self, reference: ComponentHandle) -> Result<&TriggerComponent, String> {
        self.events.components.trigger(reference)
    }
    fn trigger_mut(&mut self, reference: ComponentHandle) -> Result<&mut TriggerComponent, String> {
        self.events.components.trigger_mut(reference)
    }
    fn load_shape(&mut self, entity: usize) -> Result<Option<LoadedTriggerShape>, String> {
        self.level.load_shape(entity, self.geometry)
    }
    fn dependency(
        &mut self,
        entity: usize,
        kind: TriggerDependency,
    ) -> Result<Option<ComponentHandle>, String> {
        self.level.dependency(entity, kind, &self.events.components)
    }
    fn position(&self, entity: usize) -> Result<[f32; 3], String> {
        self.level.position(entity)
    }
    fn declaration(&self, reference: ComponentHandle) -> Result<usize, String> {
        self.events
            .components
            .trigger(reference)?
            .owner
            .declaration
            .ok_or_else(|| "trigger declaration is missing".into())
    }
    fn collision_settings(&self, declaration: usize) -> Result<TriggerCollisionSettings, String> {
        self.level.collision_settings(declaration)
    }
    fn trigger_material(&self) -> Result<u16, String> {
        self.level.trigger_material(&self.world.materials)
    }
    fn create_disabled_body(
        &mut self,
        _reference: ComponentHandle,
        info: &BodyInfo,
    ) -> Result<u32, String> {
        self.world.create_body(
            self.geometry,
            self.events,
            info,
            BodyAddition::Disabled,
            BodyActivation::Activate,
            None,
        )
    }
    fn set_body(&mut self, reference: ComponentHandle, body: u32) -> Result<(), String> {
        let movement = self.events.components.trigger(reference)?.movement_body;
        self.events
            .components
            .actors
            .bodies
            .get_mut(movement)
            .ok_or("trigger movement interface is missing")?
            .body_id = body;
        Ok(())
    }
    fn mark_entity_keyframed(&mut self, entity: usize) -> Result<(), String> {
        self.level.mark_entity_keyframed(entity)
    }
    fn report_missing_shape(&mut self, entity: usize) -> Result<(), String> {
        self.level.report_missing_shape(entity)
    }
    fn release_failed(&mut self, reference: ComponentHandle) -> Result<(), String> {
        self.level
            .release_failed(reference, self.world, self.geometry, self.events)
    }
}
