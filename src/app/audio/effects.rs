//! Installed sound dependencies for world FX, using the ordinary bank registry.
use super::{AudioMix, PcmAudio, SourceAction, WwiseRegistry};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};
use crate::rendering::{
    bevy_backend::{world_effects::WorldEffectSources, world_sounds::WorldSounds},
    particles::world_sources::WorldFxSources,
};

#[derive(Resource, Default)]
pub(super) struct EffectAudio {
    source: Option<Arc<WorldFxSources>>,
    events: HashMap<u32, Result<Arc<Vec<SourceAction>>, String>>,
}

/// Prepare on the main side, before extraction. FX posts will use these same
/// decoded definitions and the shared output clock, not another media player.
pub(super) fn prepare(mut state: ResMut<EffectAudio>, sources: Option<Res<WorldEffectSources>>,
    sounds: Option<Res<WorldSounds>>, mut registry: ResMut<WwiseRegistry>,
    mut pcm: ResMut<Assets<PcmAudio>>, mix: Res<AudioMix>)
{
    let (Some(sources), Some(sounds), Some(mixer)) = (sources, sounds, &mix.mixer) else { return };
    if state.source.as_ref().is_some_and(|old| Arc::ptr_eq(old, &sources.0)) { return; }
    let mut events = HashMap::new();
    for effect in sources.0.effects.values().filter_map(|effect| effect.as_ref().ok()) {
        for group in &effect.groups {
            for event in group.sounds.iter().flat_map(|sound| [&sound.start, &sound.stop]).flatten() {
                events.entry(event.id).or_insert_with(|| {
                    if event.game_defined || event.game_properties.is_some() {
                        return Err(format!("FX event {:#010x} requires its game-defined sound handler", event.id));
                    }
                    registry.prepare(event.id, &mut pcm, mixer, true).map_err(|error| error.to_string())?;
                    let actions = registry.sources.get(&event.id).ok_or("prepared FX sound event is absent")?;
                    Ok(Arc::new(actions.clone()))
                });
            }
        }
    }
    sounds.lock().events = events.iter().map(|(&event, result)|
        (event, result.as_ref().map(|_| ()).map_err(Clone::clone))).collect();
    state.events = events;
    state.source = Some(sources.0.clone());
}
