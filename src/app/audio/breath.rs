//! Admission and teardown of4c0b50/4c0c60's actual player Audio events.
//! Definitions are prepared with the player; media uses EventReservations.
use crate::gameplay::sound::{Emitter, Request};
use bevy::prelude::*;
use std::sync::Arc;

pub(in crate::app) fn prepare_player_breath(world: &mut World) -> Result<(), String> {
    let Some(id) = super::super::player_lifetime::id(world) else {
        return Ok(());
    };
    world.resource_scope(|world, mut state: Mut<super::super::player::PlayerState>| {
        let Some(audio) = state
            .player
            .as_mut()
            .and_then(|player| player.breath_audio_mut())
        else {
            return Ok(());
        };
        if audio.host_ready {
            return Ok(());
        }
        if audio.host_scope.is_none() {
            let scope: Arc<str> = format!("player-breath:{id:?}").into();
            super::reserve_events(
                world,
                scope.clone(),
                audio.start.iter().chain(audio.stop.iter()).cloned(),
            )?;
            audio.host_scope = Some(scope);
            return Ok(());
        }
        audio.host_ready = super::events_ready(world, audio.host_scope.as_ref().unwrap())?;
        Ok(())
    })
}
pub(in crate::app) fn player_breath_ready(world: &World) -> bool {
    world
        .get_resource::<super::super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
        .and_then(|player| player.breath_audio())
        .is_none_or(|audio| audio.host_ready)
}
pub(in crate::app) fn retire_player_breath(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<super::super::player::PlayerState>() {
        return Ok(());
    }
    let scope = world.resource_scope(|world, mut state: Mut<super::super::player::PlayerState>| {
        let Some(player) = state.player.as_mut() else {
            return None;
        };
        let emitter = Emitter::PlayerJoint {
            entity: None,
            part: 1,
            joint: player.body.skeleton.camera as u16,
        };
        let Some(audio) = player.breath_audio_mut() else {
            return None;
        };
        if let Some(event) = audio.stop() {
            world.write_message(super::SoundEvent(Request::Post { event, emitter }));
        }
        audio.host_scope.clone()
    });
    // Transfer the accepted stop through the existing FIFO while its prepared
    // action graph remains leased. Output voices retain their own source tails.
    if let Some(scope) = scope {
        super::poll_object_retirements(world)?;
        super::release_events(world, &scope)?;
        if let Some(audio) = world
            .resource_mut::<super::super::player::PlayerState>()
            .player
            .as_mut()
            .and_then(|player| player.breath_audio_mut())
        {
            audio.host_scope = None;
        }
    }
    Ok(())
}
