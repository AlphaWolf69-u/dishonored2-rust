//! Trigger779980's start phase on the canonical component and physics owners.
use super::TriggerComponentStore;
use crate::gameplay::{
    body_activation::ActivationOwners,
    body_storage::BodyActivation,
    collision_user_data::velocity::VelocityDataStore,
    component_state::{
        declaration::DeclarationScheduling, entity::ComponentHandle, prepare_admission,
    },
    physics_world::{
        body_creation::BodyCreationGeometry, storage::WorldStorage, BodyAddition,
        BodyEventCallback, ComponentPublicationEvents, PublicationEvent,
    },
    signal::Signal,
};

/// Admission precedes body publication. Publication observers may change the
/// movement interface, so subscription reads its body ID again afterward.
/// The keyframed branch requires the level's real reference-actor attachment.
pub(crate) fn start_trigger<'a, G: BodyCreationGeometry, F>(
    world: &mut WorldStorage,
    geometry: &mut G,
    events: &mut ComponentPublicationEvents<'a, usize, usize, F>,
    reference: ComponentHandle,
    scheduling: &DeclarationScheduling,
    entity_priority: i32,
    keyframed: bool,
    attach_actor: impl FnOnce(&mut TriggerComponentStore<'a>, ComponentHandle) -> Result<(), String>,
) -> Result<(), String>
where
    F: FnMut(
        &mut Signal<usize, usize>,
        usize,
        usize,
        ActivationOwners<'_>,
        PublicationEvent<'_>,
        &mut TriggerComponentStore<'a>,
    ) -> Result<(), String>,
{
    let trigger = events.components.trigger_mut(reference)?;
    // The inherited virtual60 (7e73f0) is declaration priority <8.
    if prepare_admission(
        &mut trigger.owner.flags,
        u8::from(scheduling.priority < 8),
        scheduling.priority,
        entity_priority,
    ) {
        events.components.triggers.schedule(reference.slot)?;
    }
    let body = events.components.referenced_body(reference)?;
    world.add_bodies(
        geometry,
        events,
        &[body],
        BodyAddition::Immediate,
        BodyActivation::Activate,
    )?;
    let body = events.components.referenced_body(reference)?;
    world
        .body_events
        .get_or_create(world.bodies.capacity(), body, 0)?
        .subscribe(reference, BodyEventCallback::TriggerContact);
    if keyframed {
        attach_actor(&mut events.components, reference)?;
    }
    Ok(())
}
