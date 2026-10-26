//! arkCpntAudio deferred startup and retained default sound objects.
use bevy::prelude::*;
use std::collections::HashMap;
use crate::gameplay::sound::{acoustic_world::AcousticWorld, admission::{Admission, Query as AdmissionQuery},
    component::StartupAction, paths::{self, ListenerState}, positions, propagation, Emitter, Request};
use super::spatial::{Position, Propagated, Scene};

#[derive(Default)]
struct Object {
    source: Option<Vec3>,
    zone: Option<usize>,
    listener: ListenerState,
    audible_mask: u32,
    error: Option<String>,
}

#[derive(Resource)]
pub(in crate::app) struct LevelAudio {
    world: AcousticWorld,
    objects: HashMap<Emitter, Object>,
    listener_position: Option<Vec3>,
    listener_zone: Option<usize>,
}
impl LevelAudio {
    pub(in crate::app) fn load(catalog: &crate::archive::Catalog, entities: &str, base: &std::path::Path) -> Result<Self, String> {
        let path = format!("{}.soundpropa", entities.trim_end_matches(".entities"));
        let bytes = catalog.read(&path).map_err(|error| error.to_string())?
            .ok_or_else(|| format!("missing acoustic resource {path}"))?;
        let graph = crate::assets::sound_propagation::Propagation::parse(&bytes).map_err(|error| error.to_string())?;
        Ok(Self { world: AcousticWorld::new(graph, base)?, objects: HashMap::new(),
            listener_position: None, listener_zone: None })
    }
}

pub(super) fn update(mut commands: Commands, level: Option<ResMut<LevelAudio>>,
    physics: Option<ResMut<super::super::physics_scene::PhysicsScene>>,
    bound: Option<Res<super::super::model_motion::BoundModels>>, mut scene: ResMut<Scene>,
    clock: Option<Res<crate::gameplay::game_clock_runtime::ClockRuntime>>,
    playing: Query<(Entity, &super::AudioTarget)>, mut events: MessageWriter<super::SoundEvent>)
{
    let (Some(mut level), Some(mut physics), Some(listener), Some(bound), Some(clock)) = (level, physics, scene.listener(), bound, clock) else { return };
    let Ok(clock) = clock.clock() else { return };
    // Default world entity timespace 0 maps to group 3 (877eb0, 8abb10).
    let Some(rates) = clock.rates(3) else { return };
    let level = &mut *level;
    if level.listener_position != Some(listener.position) {
        match level.world.zone_at(listener.position.to_array()) {
            Ok(zone) => { level.listener_zone = zone; level.listener_position = Some(listener.position); }
            Err(error) => { error!("Audio listener zone query failed: {error}"); return; }
        }
    }
    let result = physics.visit_audio(|handle, binding, audio| {
        if !audio.started || !audio.startup.deferred { return Ok(()); }
        let emitter = Emitter::Component { handle: handle.packed(), generation: audio.owner.generation };
        scene.set_rate(emitter, binding.settings.rate(rates[1]), rates[1] == 0.0);
        let object = level.objects.entry(emitter).or_default();
        let mut position = Vec3::from_array(binding.frame.position);
        let mut front = Vec3::from_array(binding.frame.basis[0]);
        if let Some(delta) = bound.entity_delta(&binding.entity_id) {
            let rotate = |value: Vec3| Vec3::from_array(std::array::from_fn(|row|
                delta[row][0] * value.x + delta[row][1] * value.y + delta[row][2] * value.z));
            position = rotate(position) + Vec3::new(delta[0][3], delta[1][3], delta[2][3]);
            front = rotate(front);
        }
        let result = (|| -> Result<(), String> {
            if object.source != Some(position) {
                object.zone = level.world.zone_at(position.to_array())?;
                object.source = Some(position);
            }
            let query = AdmissionQuery { source: position, listener: listener.position, range: audio.startup.max_distance,
                vertical_scale: 1.0, propagation: binding.propagation,
                source_zone: object.zone, listener_zone: level.listener_zone };
            let origin = propagation::admission(&level.world.graph, &query, 1.0)?;
            // Retained allocated objects update their listener mask even while
            // their startup list is stopped, so reentry can restart that list.
            if audio.allocated || origin != Admission::Inaudible {
                object.listener.update(&level.world.graph, &paths::Query {
                    source: position, listener: listener.position, source_zone: object.zone,
                    listener_zone: level.listener_zone, range: query.range, propagation: binding.propagation,
                    reflection_normal: scene.reflection_normal(),
                    source_occlusion_scale: binding.settings.source_occlusion_scale(),
                }, query.range, false, 1, &mut object.audible_mask)?;
            }
            // A zero owner clock freezes the component update (8abb10).
            if rates[1] != 0.0 {
                match audio.startup.update(audio.allocated, object.audible_mask == 0, origin == Admission::Inaudible) {
                    StartupAction::Stop => {
                        // 1400c5d80 stops every playing ID, zero milliseconds,
                        // while preserving the allocated default object slot.
                        for (entity, target) in &playing {
                            if target.emitter == emitter { commands.entity(entity).try_despawn(); }
                        }
                    }
                    StartupAction::Post => {
                        audio.allocated = true; // 140708340 precedes PostEvent.
                        for event in binding.events.iter().flatten() {
                            events.write(super::SoundEvent(Request::Post { event: event.id, emitter }));
                        }
                        audio.startup.posted(audio.allocated);
                    }
                    StartupAction::None => {}
                }
            }
            if audio.allocated && object.audible_mask != 0 {
                let results = &object.listener.results;
                let closest = results.closest.and_then(|index| results.paths.get(index));
                let positions = positions::for_mixing(&[results], closest, query.range, front)?
                    .into_iter().map(|position| Position { position: position.position, front: Some(position.front) }).collect();
                let levels = object.listener.levels(scene.minimum(), binding.settings.level_scale()).into();
                scene.publish(emitter, Position { position, front: Some(front) }, Propagated {
                    positions, levels,
                }, binding.settings.rate(rates[1]), rates[1] == 0.0);
            }
            Ok(())
        })();
        match result {
            Ok(()) => object.error = None,
            Err(error) => {
                if object.error.as_ref() != Some(&error) {
                    error!("Audio component {} {}: {error}", binding.entity_id, binding.slot);
                    object.error = Some(error);
                }
            }
        }
        Ok(())
    });
    if let Err(error) = result { error!("Level audio component lookup failed: {error}"); }
}
