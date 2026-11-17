//! Canonical AIWorld-targeted danger messages, using the existing event FIFO.
pub(super) mod creation;
use bevy::prelude::World;
use crate::gameplay::{ai_world::{SphereDanger, Worlds}, entity_events::{EntityEvents, Event, Listener, Payload},
    entity_lifetime::{EntityId, LiveEntities}};

/// 41e220/41e330 attach/detach the actual AIDangerZone receiver.
pub(super) fn bind(entity: EntityId, checked: crate::gameplay::pickable_component::CheckedComponent, events: &mut EntityEvents) {
    for event in [0x161, 0x163, 0x165] {
        events.register(event, Some(entity), Listener::AIWorldDanger { world: entity, handle: checked.handle, generation: checked.generation });
    }
}
pub(super) fn unbind(entity: EntityId, checked: crate::gameplay::pickable_component::CheckedComponent, events: &mut EntityEvents) {
    for event in [0x161, 0x163, 0x165] {
        events.unregister(event, Some(entity), Listener::AIWorldDanger { world: entity, handle: checked.handle, generation: checked.generation });
    }
}

pub(super) fn post_grenade_danger(world: &World, live: &LiveEntities, events: &mut EntityEvents,
    entity: EntityId, mode: i32, velocity: [f32; 3], position: [f32; 3], radius: f32) -> Result<(), String> {
    if live.get(entity).is_none() { return Err("grenade danger source is stale".into()); }
    let recipient = world.get_resource::<Worlds>().ok_or("canonical AIWorld owner is absent")?.recipient(live, entity)?;
    if live.get(recipient).is_none() { return Err("grenade AIWorld recipient is stale".into()); }
    let event = match mode { 3 => 0x161, 0 => 0x163, _ => return Err("unsupported grenade danger operation".into()) };
    events.post(Event { id: event, target: Some(recipient), payload: Payload::AIDangerSphere(
        SphereDanger { entity, mode, velocity, position, radius }) });
    Ok(())
}

pub(super) fn remove_grenade_danger(world: &World, live: &LiveEntities, events: &mut EntityEvents,
    entity: EntityId) -> Result<(), String> {
    if live.get(entity).is_none() { return Err("grenade danger source is stale".into()); }
    let recipient = world.get_resource::<Worlds>().ok_or("canonical AIWorld owner is absent")?.recipient(live, entity)?;
    if live.get(recipient).is_none() { return Err("grenade AIWorld recipient is stale".into()); }
    events.post(Event { id: 0x165, target: Some(recipient), payload: Payload::Entity(entity) });
    Ok(())
}

/// The existing central event dispatcher supplies the matched, generation-bound
/// receiver. No global fallback or acknowledgement substitutes for its mutation.
pub(super) fn dispatch_owner(scene: &mut super::physics_scene::PhysicsScene, live: &LiveEntities,
    owner: &Worlds, registry: &mut crate::gameplay::component_state::ComponentRegistry,
    listener: &Listener, event: &Event) -> Result<bool, String> {
    let Listener::AIWorldDanger { world: recipient, handle, generation } = *listener else { return Ok(false) };
    if event.target != Some(recipient) { return Err("AI danger listener target differs from its actual owner".into()); }
    let checked = owner.danger(recipient)?.ok_or("AIWorld has no initialized Danger component")?;
    if checked.handle != handle || checked.generation != generation { return Err("AI danger component generation differs from its subscription".into()); }
    let zones = scene.ai_danger_rows(live, recipient, checked)?;
    match (event.id, &event.payload) {
        (0x161, Payload::AIDangerSphere(value)) => zones.add(*value),
        (0x163, Payload::AIDangerSphere(value)) => zones.update(*value)?,
        (0x165, Payload::Entity(entity)) => zones.remove(*entity),
        _ => return Err("AI danger callback has an invalid source message".into()),
    }
    if event.id == 0x161 { scene.resume_ai_danger(checked.handle, registry)?; }
    Ok(true)
}
