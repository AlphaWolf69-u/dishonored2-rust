//! Installed sound dependencies for world FX, using the ordinary bank registry.
use super::{AudioMix, SourceAction, WwiseRegistry};
use bevy::prelude::*;
use std::sync::Arc;
use crate::rendering::{
    bevy_backend::{world_effects::WorldEffectSources, world_sounds::WorldSounds},
    particles::world_sources::{WorldFxSources, WorldSoundEvent},
};

fn trace_fx_sound_limits() -> bool {
    static TRACE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *TRACE.get_or_init(|| std::env::var("DH2_TRACE_FX_SOUND_LIMITS").as_deref() == Ok("1"))
}

struct PreparedEvent {
    declaration: Arc<WorldSoundEvent>,
    actions: Arc<Vec<SourceAction>>,
}

#[derive(Resource, Default)]
pub(super) struct EffectAudio {
    source: Option<Arc<WorldFxSources>>,
    events: Vec<PreparedEvent>,
    reported_playback: bool,
    trace_frame: u32,
    pending_media: std::collections::HashMap<crate::gameplay::sound::playing::PlayingId, Vec<super::graph::StreamHold>>,
}

/// Prepare dependencies, publish actual sound-object frames, then execute
/// accepted commands on the same output engine as ordinary gameplay audio.
pub(super) fn update(mut state: ResMut<EffectAudio>, sources: Option<Res<WorldEffectSources>>,
    sounds: Option<Res<WorldSounds>>, mut registry: ResMut<WwiseRegistry>,
    mix: Res<AudioMix>, identities: Res<super::objects::EffectObjects>,
    level: Option<ResMut<super::level::LevelAudio>>, mut scene: ResMut<super::spatial::Scene>,
    engine: Res<super::bevy_backend::Engine>, mut commands: Commands,
    playing: Query<(Entity, &super::AudioTarget, &super::graph::Playing)>,
    paused: Option<Res<crate::gameplay::game_clock_runtime::GamePaused>>, mut error: Local<Option<String>>)
{
    let (Some(sources), Some(sounds), Some(mixer), Some(mut level)) = (sources, sounds, &mix.mixer, level) else { return };
    let mut bridge = sounds.lock();
    bridge.pump_notifications();
    let state = &mut *state;
    // Released/cancelled tickets cease owning their completed reads even if
    // there is no replacement event to prepare this frame.
    state.pending_media.retain(|id, _| bridge.queued.iter().any(|ticket| ticket.id() == *id));
    let result = (|| -> Result<(), String> {
        bridge.identities = Some(identities.clone());
        bridge.acoustics = Some(level.world.clone());
        bridge.energy = mix.physics_scale;
        bridge.listeners = [None; 8];
        bridge.motion = [None; 8];
        if let Some(listener) = scene.listener() {
            let zone = level.world.lock().unwrap_or_else(|error| error.into_inner()).zone_at(listener.position.to_array())?;
            bridge.listeners[0] = Some(crate::gameplay::sound::acoustic_world::AdmissionListener { position: listener.position, zone });
            bridge.motion[0] = scene.listener_motion();
        }
        bridge.ready = engine.activity().is_some() && mix.physics_scale.is_some() && scene.listener().is_some();
        if state.source.as_ref().is_none_or(|old| !Arc::ptr_eq(old, &sources.0)) {
            // Virtualized handles retain declaration references without an
            // allocated object or playing voice. Never renumber those refs.
            let mut references = bridge.events.clone();
            let mut events = Vec::new();
            for effect in sources.0.effects.values().filter_map(|effect| effect.as_ref().ok()) {
                for group in &effect.groups {
                    for event in group.sounds.iter().flat_map(|sound| [&sound.start, &sound.stop]).flatten() {
                        references.entry(event.name.clone()).or_insert_with(|| {
                            let declaration = &event.declaration;
                            if declaration.game_defined || declaration.game_properties.is_some() {
                                return Err(format!("FX declaration {} requires its game sound handler", event.name));
                            }
                            registry.prepare(declaration.id).map_err(|error| error.to_string())?;
                            let actions = registry.sources.get(&declaration.id).ok_or("prepared FX sound event is absent")?;
                            if actions.iter().any(|action| match action {
                                SourceAction::Play { definition, .. } => !definition.can_route(),
                                SourceAction::Stop { .. } | SourceAction::SetState { .. } => false,
                            }) { return Err(format!("{} requires multichannel FX output conversion", event.name)); }
                            let reference = u32::try_from(state.events.len() + events.len()).map_err(|_| "too many FX sound declarations")?;
                            events.push(PreparedEvent { declaration: event.clone(), actions: Arc::new(actions.clone()) });
                            Ok(reference)
                        });
                    }
                }
            }
            bridge.events = references;
            bridge.declarations.extend(events.iter().map(|event| event.declaration.clone()));
            state.events.extend(events);
            state.source = Some(sources.0.clone());
            state.reported_playback = false;
            info!(declarations = bridge.declarations.len(), unavailable = bridge.events.values().filter(|event| event.is_err()).count(),
                "World FX sound dependencies prepared");
            // DH2_TRACE_FX_SOUND_LIMITS=1: each FX event's Wwise playback limits.
            if trace_fx_sound_limits() {
                for event in &state.events {
                    let id = event.declaration.declaration.id;
                    eprintln!("FX_SOUND_LIMITS {} id={id:#010x} chains={:?}", event.declaration.name,
                        registry.library.play_limits(id).map_err(|error| error.to_string()));
                }
            }
        }
        if trace_fx_sound_limits() && !bridge.released.is_empty() {
            eprintln!("FX_SOUND_RELEASED {} frame={}", bridge.released.len(), state.trace_frame);
        }
        for emitter in std::mem::take(&mut bridge.released) {
            for (entity, target, _) in &playing {
                if target.emitter == emitter { commands.entity(entity).try_despawn(); }
            }
            level.release_object(emitter);
        }
        let detail = crate::platform::console::shared().read().cvars.int("s_environmentalDetailLevel")
            .ok_or("environmental audio detail is absent")?;
        for (&emitter, object) in &mut bridge.objects {
            let frozen = object.scale == 0.0 || paused.as_ref().is_some_and(|paused| paused.0);
            object.publish_frame(frozen);
            if object.has_events() {
                let settings = crate::gameplay::sound::component::Settings {
                    occlusion_at_emitter: object.source_occlusion,
                    ..Default::default()
                };
                let evaluation = level.evaluate(emitter, object.position, object.front, object.range,
                    true, settings, true, object.range <= 0.0, &scene, detail)?;
                object.audible_mask = if evaluation.inaudible { 0 } else { 1 };
                let propagated = evaluation.propagated.unwrap_or(super::spatial::Propagated {
                    positions: Vec::new(), levels: Default::default(), auxiliary: Vec::new(),
                });
                scene.publish(emitter, super::spatial::Position { position: object.position, front: Some(object.front) },
                    propagated, object.scale, object.scale == 0.0);
            }
        }
        let mut spawned = Vec::new();
        let mut submitted = 0u32;
        // A ticket whose streamed media is still being read stays queued.
        let mut loading_tickets = Vec::new();
        for ticket in std::mem::take(&mut bridge.queued) {
            let previous_media = state.pending_media.remove(&ticket.id());
            let event = ticket.event();
            let prepared = state.events.get(event.declaration as usize).ok_or("accepted FX event declaration is absent")?;
            let object = bridge.objects.get(&event.object).ok_or("accepted FX event object is absent")?;
            let parameter = object.energy;
            let mut holds = Vec::new();
            let mut loading = false;
            for action in prepared.actions.iter() {
                let SourceAction::Play { definition, .. } = action else { continue };
                for source in definition.streamed() {
                    match source.acquire() {
                        Ok(Some(hold)) => holds.push(hold),
                        Ok(None) => loading = true,
                        Err(error) => warn!("FX event {} streamed media unavailable: {error}", prepared.declaration.name),
                    }
                }
            }
            drop(previous_media);
            if loading {
                state.pending_media.insert(ticket.id(), holds);
                loading_tickets.push(ticket);
                continue;
            }
            let playing_id = engine.playing_id();
            for action in prepared.actions.iter() {
                match action {
                    SourceAction::SetState { action, target_path } => {
                        if let Err(error) = engine.post_state(*action, target_path.clone(), prepared.declaration.declaration.id, event.object) {
                            warn!("FX event {} Set State unavailable: {error}", prepared.declaration.name);
                        }
                    },
                    SourceAction::Stop { action, target_path } => {
                        if let Err(error) = engine.post_stop(action.clone(), target_path.clone(), prepared.declaration.declaration.id, event.object) {
                            warn!("FX event {} Stop unavailable: {error}", prepared.declaration.name);
                        }
                    },
                    SourceAction::Play { settings, definition } => {
                        // The registration already exists. A failed action ends
                        // its ownership; it cannot fabricate another playing ID.
                        match definition.prepare(registry.scheduled_selections.clone(), event.object, mixer,
                            parameter, &scene, None, *settings)
                        {
                            Ok((mut source, control)) => {
                                source.retain_event(ticket.clone());
                                match engine.submit(source, playing_id, prepared.declaration.declaration.id) {
                                    Ok(voice) => {
                                        submitted += 1;
                                        if trace_fx_sound_limits() {
                                            eprintln!("FX_SOUND_VOICE {} object={:?} position={:?} range={}", prepared.declaration.name, event.object,
                                                object.position, prepared.declaration.declaration.max_distance);
                                        }
                                        let target = super::AudioTarget { event: prepared.declaration.declaration.id,
                                            emitter: event.object, paused: false, world_paused: false };
                                        let entity = commands.spawn((voice, target.clone(), control.clone(), super::StreamedMedia(holds.clone()))).id();
                                        spawned.push((entity, target, control));
                                    }
                                    Err(message) => warn!("FX event {} output failed: {message}", prepared.declaration.name),
                                }
                            }
                            Err(message) => warn!("FX event {} action failed: {message}", prepared.declaration.name),
                        }
                    }
                }
            }
        }
        bridge.queued.splice(0..0, loading_tickets);
        // DH2_TRACE_FX_SOUND_LIMITS=1: FX voices playing at once, every 2 s.
        if trace_fx_sound_limits() && state.trace_frame % 120 == 0 {
            let mut voices = std::collections::BTreeMap::<String, Vec<String>>::new();
            for (_, target, voice) in &playing {
                let name = state.events.iter().find(|event| event.declaration.declaration.id == target.event)
                    .map_or_else(|| format!("event:{}", target.event), |event| event.declaration.name.clone());
                let levels = voice.debug_levels().iter().map(|(id, volume, peak)| format!("{id}:{volume:.1}dB*{peak:.3}")).collect::<Vec<_>>().join("+");
                let chain = voice.debug_chain().iter().map(u32::to_string).collect::<Vec<_>>().join(">");
                voices.entry(name).or_default().push(format!("[{chain}] {levels}"));
            }
            let total = voices.values().map(Vec::len).sum::<usize>();
            let objects = bridge.objects.values().filter(|object| object.has_events()).collect::<Vec<_>>();
            eprintln!("FX_SOUND_PLAYING total={total} objects_with_events={} audible={} listener={:?}", objects.len(),
                objects.iter().filter(|object| object.audible_mask != 0).count(), scene.listener().map(|l| l.position));
            for (name, levels) in voices { eprintln!("FX_SOUND_PLAYING {name} n={} {}", levels.len(), levels.join(" ")); }
        }
        state.trace_frame = state.trace_frame.wrapping_add(1);
        if submitted != 0 && !state.reported_playback {
            info!(voices = submitted, "World FX sound playback submitted to shared output");
            state.reported_playback = true;
        }
        Ok(())
    })();
    match result {
        Ok(()) => *error = None,
        Err(message) => {
            bridge.ready = false;
            if error.as_ref() != Some(&message) { warn!("World FX audio unavailable: {message}"); *error = Some(message); }
        }
    }
}
