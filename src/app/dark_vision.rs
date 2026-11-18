//! Dark Vision's loaded-world owner: the player's arkCpntDarkVision
//! (`cpntDarkVision` on the player body), started and stopped by the power
//! (14043d3c0), updated each frame (14049ff50) and publishing the renderer
//! record the dark-vision, distortion and post-process passes read.
use std::sync::Arc;
use bevy::prelude::*;
use crate::{
    assets::{archive::Catalog, wwise::SoundEventDeclaration},
    gameplay::{
        powers::{
            dark_vision::{DarkVisionAttributes, DarkVisionEffect, DarkVisionFrame, DarkVisionPlayer, DarkVisionSettings},
            world_effects::{DarkVision, DarkVisionState, DarkVisionWorld},
        },
        sound::{Emitter, Request},
    },
};

#[derive(Resource)]
pub(super) struct DarkVisionOwner {
    settings: Option<DarkVisionSettings>,
    sounds: Vec<(String, Arc<SoundEventDeclaration>)>,
    state: DarkVisionState,
    effect: Option<DarkVisionEffect>,
    pending_sounds: Vec<Request>,
    debug_started: bool,
}


impl DarkVisionOwner {
    /// The player body's cpntDarkVision with its sounds' declarations. A
    /// character without the component has no Dark Vision owner.
    pub fn prepare(catalog: &Catalog, character: &str) -> Result<Self, String> {
        let read = |path: &str| -> Result<Option<String>, String> {
            Ok(catalog.read(path).map_err(|e| e.to_string())?.map(crate::assets::entities::owned_declaration_text))
        };
        let body = format!("models/characters/player/{character}/body.def");
        let settings = crate::assets::entities::player::entity_component(&body, "cpntdarkvision", read)?
            .map(|fields| DarkVisionSettings::read(&fields))
            .transpose()?;
        let mut sounds = Vec::new();
        if let Some(settings) = &settings {
            for name in [&settings.pulse_sound, &settings.stop_sound].into_iter().flatten() {
                if sounds.iter().any(|(path, _): &(String, _)| path == name) {
                    continue;
                }
                let source = crate::assets::fx::SoundEventAsset { delay: [0.; 2], event: Some(name.clone()) }
                    .resolve(catalog)?
                    .ok_or_else(|| format!("Dark Vision sound {name} has no declaration"))?;
                sounds.push((name.clone(), source));
            }
        }
        Ok(Self { settings, sounds, state: DarkVisionState::Stopped, effect: None, pending_sounds: Vec::new(), debug_started: false })
    }

    pub fn available(&self) -> bool {
        self.settings.is_some()
    }

    fn post(&mut self, name: Option<&str>) {
        let Some(name) = name else { return };
        if let Some((_, sound)) = self.sounds.iter().find(|(path, _)| path == name) {
            self.pending_sounds.push(Request::Post { event: sound.id, emitter: Emitter::PlayerBody });
        }
    }

    /// The component's queued start/stop take effect at its next update.
    fn settle(&mut self) {
        match self.state {
            DarkVisionState::PendingStart => {
                if let Some(settings) = &self.settings {
                    self.effect = Some(DarkVisionEffect::new(settings.clone()));
                }
                self.state = DarkVisionState::Started;
            }
            DarkVisionState::PendingStop => {
                self.effect = None;
                self.state = DarkVisionState::Stopped;
                let stop = self.settings.as_ref().and_then(|s| s.stop_sound.clone());
                self.post(stop.as_deref());
            }
            _ => {}
        }
    }
}

impl DarkVisionWorld for DarkVisionOwner {
    fn component_state(&self) -> Option<DarkVisionState> {
        self.settings.as_ref().map(|_| self.state)
    }
    /// 14043d530: the declaration gate and the Outsider mark's suppression
    /// roster; no mark suppression owner exists, so nothing blocks.
    fn blocked(&self) -> bool {
        false
    }
    fn request_start(&mut self) {
        self.state = DarkVisionState::PendingStart;
    }
    fn request_stop(&mut self) {
        self.state = DarkVisionState::PendingStop;
    }
}

/// Power id 9's cast: payment, then the start/stop request (14043d3c0).
pub(super) fn cast(world: &mut World, pay: impl FnMut()) -> bool {
    world.get_resource_mut::<DarkVisionOwner>()
        .is_some_and(|mut owner| DarkVision::cast(&mut *owner, pay))
}

/// 14043d490: the mana cost the cast will take.
pub(super) fn mana_factor(world: &World, normal: f32) -> f32 {
    world.get_resource::<DarkVisionOwner>().map_or(0., |owner| DarkVision::mana_cost(owner, normal))
}

/// One frame of the running component, after the player's power actions.
pub(super) fn update(world: &mut World, player: Option<&crate::gameplay::player::Player>, dt: f32) {
    let Some(mut owner) = world.remove_resource::<DarkVisionOwner>() else { return };
    // debug_: a diagnostic start through the component's own request, for
    // windowless captures without player input.
    if !owner.debug_started && owner.settings.is_some()
        && std::env::var_os("DH2_DEBUG_DARK_VISION_START").is_some_and(|value| value == "1") {
        owner.debug_started = true;
        owner.request_start();
    }
    owner.settle();
    let mut record = None;
    if let Some(effect) = owner.effect.as_mut() {
        let attributes = player.map(|p| DarkVisionAttributes::read(&p.attributes)).unwrap_or_default();
        let state = player.map(|p| DarkVisionPlayer {
            position: p.feet().to_array(),
            velocity: p.velocity().to_array(),
            playing: true,
            veto: false,
        });
        let (frame, step) = effect.update(dt, state.as_ref(), &attributes);
        record = Some(frame);
        if step.reveal_started {
            let sound = owner.settings.as_ref().and_then(|s| s.pulse_sound.clone());
            owner.post(sound.as_deref());
        }
        if step.stop {
            owner.state = DarkVisionState::PendingStop;
        }
    }
    for request in owner.pending_sounds.drain(..) {
        world.write_message(super::audio::SoundEvent(request));
    }
    world.insert_resource(DarkVisionFrame(record));
    world.insert_resource(owner);
}
