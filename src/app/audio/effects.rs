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
    events: Vec<Option<PreparedEvent>>,
    free_events: Vec<u32>,
    retiring_events: std::collections::HashSet<u32>,
    dynamic_decodes: std::collections::HashSet<u32>,
    reported_playback: bool,
    trace_frame: u32,
    pending_media: std::collections::HashMap<crate::gameplay::sound::playing::PlayingId, Vec<super::graph::StreamHold>>,
    pending_events: std::collections::HashSet<u32>,
    failed_events: std::collections::HashMap<u32, String>,
}

/// Prepare dependencies, publish actual sound-object frames, then execute
/// accepted commands on the same output engine as ordinary gameplay audio.
pub(super) fn update(mut state: ResMut<EffectAudio>, sources: Option<Res<WorldEffectSources>>,
    sounds: Option<Res<WorldSounds>>, mut registry: ResMut<WwiseRegistry>,
    mix: Res<AudioMix>, identities: Res<super::objects::EffectObjects>,
    level: Option<ResMut<super::level::LevelAudio>>, mut scene: ResMut<super::spatial::Scene>,
    engine: Res<super::bevy_backend::Engine>, mut commands: Commands,
    playing: Query<(Entity, &super::AudioTarget, &super::graph::Playing)>,
    paused: Option<Res<crate::gameplay::game_clock_runtime::GamePaused>>, mut error: Local<Option<String>>,
    mut decodes: ResMut<super::SoundDecodeQueue>)
{
    let (Some(sources), Some(sounds), Some(mixer), Some(mut level)) = (sources, sounds, &mix.mixer, level) else { return };
    let mut bridge = sounds.lock();
    bridge.pump_notifications();
    let state = &mut *state;
    if !state.pending_events.is_empty() {
        let needed: std::collections::HashSet<u32> = sources.0.effects.values()
            .filter_map(|effect| effect.as_ref().ok()).flat_map(|effect| &effect.groups)
            .flat_map(|group| &group.sounds).flat_map(|sound| [&sound.start, &sound.stop])
            .flatten().map(|event| event.declaration.id).collect();
        decodes.retain_client(super::DecodeClient::Effects, &needed);
        decodes.poll();
        let completed: Vec<_> = state.pending_events.iter().filter_map(|&id| {
            if !needed.contains(&id) { return Some((id, None)); }
            decodes.result(id).map(|result| (id, Some(result)))
        }).collect();
        for (id, result) in completed {
            state.pending_events.remove(&id);
            decodes.release(id, super::DecodeClient::Effects);
            match result {
                Some(Ok(actions)) => {
                    if let hashbrown::hash_map::Entry::Vacant(entry) = registry.sources.entry(id) {
                        entry.insert(actions);
                        state.dynamic_decodes.insert(id);
                    }
                }
                Some(Err(error)) => { state.failed_events.insert(id, error); }
                None => {}
            }
        }
    }
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
            let source_started = crate::rendering::bevy_backend::world_effects::trace_lifecycle().then(std::time::Instant::now);
            // Virtualized handles retain declaration references without an
            // allocated object or playing voice. Never renumber those refs.
            let mut references = bridge.events.clone();
            let mut events = Vec::new();
            let mut waiting = false;
            let mut active_names = std::collections::HashSet::new();
            let mut active_ids = std::collections::HashSet::new();
            let mut appended = 0usize;
            for effect in sources.0.effects.values().filter_map(|effect| effect.as_ref().ok()) {
                for group in &effect.groups {
                    for event in group.sounds.iter().flat_map(|sound| [&sound.start, &sound.stop]).flatten() {
                        active_names.insert(event.name.clone());
                        active_ids.insert(event.declaration.id);
                        if references.contains_key(&event.name) { continue; }
                        let declaration = &event.declaration;
                        if !registry.sources.contains_key(&declaration.id)
                            && !state.failed_events.contains_key(&declaration.id)
                            && !declaration.game_defined && declaration.game_properties.is_none() {
                            waiting = true;
                            decodes.request(declaration.id, super::DecodeClient::Effects, &registry);
                            state.pending_events.insert(declaration.id);
                            continue;
                        }
                        references.insert(event.name.clone(), (|| {
                            let declaration = &event.declaration;
                            if declaration.game_defined || declaration.game_properties.is_some() {
                                return Err(format!("FX declaration {} requires its game sound handler", event.name));
                            }
                            if let Some(error) = state.failed_events.get(&declaration.id) { return Err(error.clone()); }
                            let actions = registry.sources.get(&declaration.id).ok_or("prepared FX sound event is absent")?;
                            if actions.iter().any(|action| match action {
                                SourceAction::Play { definition, .. } => !definition.can_route(),
                                SourceAction::Stop { .. } | SourceAction::SetState { .. } => false,
                            }) { return Err(format!("{} requires multichannel FX output conversion", event.name)); }
                            let reference = if let Some(index) = state.free_events.pop() { index } else {
                                let index = u32::try_from(state.events.len() + appended).map_err(|_| "too many FX sound declarations")?;
                                appended += 1;
                                index
                            };
                            events.push((reference, PreparedEvent { declaration: event.clone(), actions: actions.clone() }));
                            Ok(reference)
                        })());
                    }
                }
            }
            references.retain(|name, result| result.is_ok() || active_names.contains(name));
            bridge.events = references;
            for (index, event) in events {
                let index = index as usize;
                if index == state.events.len() {
                    bridge.declarations.push(Some(event.declaration.clone()));
                    state.events.push(Some(event));
                } else {
                    bridge.declarations[index] = Some(event.declaration.clone());
                    state.events[index] = Some(event);
                }
            }
            if !waiting {
                state.failed_events.retain(|id, _| active_ids.contains(id));
                for (index, event) in state.events.iter().enumerate() {
                    let Some(event) = event else { continue };
                    if active_names.contains(&event.declaration.name) { state.retiring_events.remove(&(index as u32)); }
                    else { state.retiring_events.insert(index as u32); }
                }
                state.source = Some(sources.0.clone());
                state.reported_playback = false;
                info!(declarations = bridge.declarations.len(), unavailable = bridge.events.values().filter(|event| event.is_err()).count(),
                    "World FX sound dependencies prepared");
            }
            // DH2_TRACE_FX_SOUND_LIMITS=1: each FX event's Wwise playback limits.
            if trace_fx_sound_limits() {
                for event in state.events.iter().flatten() {
                    let id = event.declaration.declaration.id;
                    eprintln!("FX_SOUND_LIMITS {} id={id:#010x} chains={:?}", event.declaration.name,
                        registry.library.play_limits(id).map_err(|error| error.to_string()));
                }
            }
            if let Some(started) = source_started {
                eprintln!("FX_LIFECYCLE audio_sources waiting={waiting} cpu_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
            }
        }
        if !state.retiring_events.is_empty() {
            let mut released = Vec::new();
            for &index in &state.retiring_events {
                let Some(event) = state.events.get(index as usize).and_then(Option::as_ref) else {
                    released.push(index); continue;
                };
                // These two owners are the bridge and its prepared action
                // table. FX definitions and active instances hold other Arcs.
                if Arc::strong_count(&event.declaration) > 2
                    || bridge.playing.references_declaration(index)
                    || bridge.notifications.iter().any(|note| note.event.declaration == index) {
                    continue;
                }
                let name = event.declaration.name.clone();
                let id = event.declaration.declaration.id;
                if crate::rendering::bevy_backend::world_effects::trace_lifecycle() {
                    eprintln!("FX_LIFECYCLE audio_retire={name} id={id:#010x}");
                }
                bridge.events.remove(&name);
                bridge.declarations[index as usize] = None;
                state.events[index as usize] = None;
                state.free_events.push(index);
                if !state.events.iter().flatten().any(|event| event.declaration.declaration.id == id)
                    && state.dynamic_decodes.remove(&id) {
                    registry.sources.remove(&id);
                }
                released.push(index);
            }
            for index in released { state.retiring_events.remove(&index); }
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
            let prepared = state.events.get(event.declaration as usize).and_then(Option::as_ref)
                .ok_or("accepted FX event declaration is absent")?;
            let object = bridge.objects.get(&event.object).ok_or("accepted FX event object is absent")?;
            let parameter = object.energy;
            let mut holds = Vec::new();
            let mut loading = false;
            if super::graph::preload_event_streams() {
                for action in prepared.actions.iter() {
                    let SourceAction::Play { definition, .. } = action else { continue };
                    for source in definition.streamed() {
                        match source.acquire() {
                            Ok(hold) => { loading |= !hold.is_ready(); holds.push(hold); },
                            Err(error) => warn!("FX event {} streamed media unavailable: {error}", prepared.declaration.name),
                        }
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
                let name = state.events.iter().flatten().find(|event| event.declaration.declaration.id == target.event)
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
