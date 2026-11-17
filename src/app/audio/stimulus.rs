//! Contact FX439480 -> actual posted-sound cookie6dd740 ->6dfb40/6dfa50.
//! This consumes audio notifications, never FX completion or guessed timers.
use crate::{
    gameplay::{
        entity_events::{EntityEvents, Event, Payload},
        entity_lifetime::LiveEntities,
    },
    rendering::bevy_backend::world_sounds::WorldSounds,
};
use bevy::prelude::*;

pub(super) fn scheduled(world: &mut World) -> Result<(), bevy::ecs::error::BevyError> {
    update(world).map_err(Into::into)
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    let Some(sounds) = world.get_resource::<WorldSounds>().cloned() else {
        return Ok(());
    };
    loop {
        // Keep the actual descriptor/cookie at the head if a consumer fails.
        // No callback can silently acknowledge a packet it did not deliver.
        let packet = {
            let mut state = sounds.lock();
            state.pump_notifications();
            let Some(packet) = state.stimuli.front() else {
                return Ok(());
            };
            (
                packet.owner,
                packet.object,
                packet.position,
                packet.distance,
                packet.descriptor.clone(),
            )
        };
        let (owner, object, position, distance, descriptor) = packet;
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("sound stimulus live identities absent")?;
        let distance = if live.get(owner).is_none() {
            None
        } else {
            world
                .get_resource::<super::super::physics_scene::PhysicsScene>()
                .ok_or("sound stimulus scene owner absent")?
                .contact_sound_distance(live, owner, &descriptor, distance)
                .map_err(|error|format!("{error}; object={object:?} position={position:?} descriptor={descriptor:?}"))?
        };
        if let Some((distance, emit_general)) = distance {
            let payload = Payload::SoundStimulus {
                object,
                position: position.to_array(),
                context: descriptor.context,
                distance,
                attention_meter: descriptor.attention_meter,
                vertical_attenuation: descriptor.vertical_attenuation,
                call_for_help: descriptor.call_for_help,
            };
            let mut events = world
                .get_resource_mut::<EntityEvents>()
                .ok_or("sound stimulus event owner absent")?;
            if descriptor.call_for_help {
                events.post(Event {
                    id: 0xc6,
                    target: Some(owner),
                    payload: payload.clone(),
                });
            }
            if emit_general {
                events.post(Event {
                    id: 0x29,
                    target: Some(owner),
                    payload,
                });
            }
        }
        sounds.lock().stimuli.pop_front();
    }
}
