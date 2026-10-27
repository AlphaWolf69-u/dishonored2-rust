//! Installed sound dependencies for world FX, using the ordinary bank registry.
use super::{AudioMix, PcmAudio, SourceAction, WwiseRegistry};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};
use crate::rendering::{
    bevy_backend::{world_effects::WorldEffectSources, world_sounds::WorldSounds},
    particles::world_sources::{WorldFxSources, WorldSoundEvent},
};

struct PreparedEvent {
    declaration: Arc<WorldSoundEvent>,
    actions: Arc<Vec<SourceAction>>,
}

#[derive(Resource, Default)]
pub(super) struct EffectAudio {
    source: Option<Arc<WorldFxSources>>,
    events: Vec<PreparedEvent>,
}

/// Prepare on the main side, before extraction. FX posts will use these same
/// decoded definitions and the shared output clock, not another media player.
pub(super) fn prepare(mut state: ResMut<EffectAudio>, sources: Option<Res<WorldEffectSources>>,
    sounds: Option<Res<WorldSounds>>, mut registry: ResMut<WwiseRegistry>,
    mut pcm: ResMut<Assets<PcmAudio>>, mix: Res<AudioMix>)
{
    let (Some(sources), Some(sounds), Some(mixer)) = (sources, sounds, &mix.mixer) else { return };
    if state.source.as_ref().is_some_and(|old| Arc::ptr_eq(old, &sources.0)) { return; }
    // Retained declaration references cannot be replaced while family slots
    // still own them. The audio owner clears those slots before the next load.
    {
        let mut bridge = sounds.lock();
        if !bridge.groups.is_empty() {
            for group in bridge.groups.values_mut() { group.clear = true; }
            return;
        }
    }
    let mut references = HashMap::new();
    let mut events = Vec::new();
    for effect in sources.0.effects.values().filter_map(|effect| effect.as_ref().ok()) {
        for group in &effect.groups {
            for event in group.sounds.iter().flat_map(|sound| [&sound.start, &sound.stop]).flatten() {
                references.entry(event.name.clone()).or_insert_with(|| {
                    let declaration = &event.declaration;
                    if declaration.game_defined || declaration.game_properties.is_some() {
                        return Err(format!("FX declaration {} requires its game sound handler", event.name));
                    }
                    registry.prepare(declaration.id, &mut pcm, mixer, true).map_err(|error| error.to_string())?;
                    let actions = registry.sources.get(&declaration.id).ok_or("prepared FX sound event is absent")?;
                    let reference = u32::try_from(events.len()).map_err(|_| "too many FX sound declarations")?;
                    events.push(PreparedEvent { declaration: event.clone(), actions: Arc::new(actions.clone()) });
                    Ok(reference)
                });
            }
        }
    }
    sounds.lock().events = references;
    state.events = events;
    state.source = Some(sources.0.clone());
}
