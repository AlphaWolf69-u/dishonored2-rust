//! Prototype-owned event preparation, using the same bank/source owner as FX
//! and ordinary component posts. Wwise 1410f1910/141122400 shares bank
//! sources; 141160cc0 releases a file source and its stream state. Readiness
//! therefore includes decoded resident media and streaming headers/prefetch,
//! never an eager acquisition of every possible branch of a streamed graph.
use super::{DecodeClient, SoundDecodeQueue, WwiseRegistry};
use crate::assets::wwise::SoundEventDeclaration;
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

struct Scope {
    generation: u64,
    // Keep the actual typed resources alive, not just their numeric IDs.
    declarations: Vec<Arc<SoundEventDeclaration>>,
}
enum State {
    Pending,
    Ready,
    Failed(String),
}
struct Event {
    users: usize,
    state: State,
}

#[derive(Resource, Default)]
pub(super) struct EventReservations {
    scopes: HashMap<Arc<str>, Scope>,
    events: HashMap<u32, Event>,
    pending: usize,
    closed: bool,
}

/// `owner` must identify the actual prototype generation/lease. No sound
/// object, dummy FX instance, or voice is created by preparation.
pub(in crate::app) fn reserve_events(
    world: &mut World,
    owner: Arc<str>,
    declarations: impl IntoIterator<Item = Arc<SoundEventDeclaration>>,
) -> Result<(), String> {
    let mut declarations = declarations.into_iter().collect::<Vec<_>>();
    declarations.sort_unstable_by_key(|event| event.id);
    declarations.dedup_by(|left,right|Arc::ptr_eq(left,right));
    if !world.contains_resource::<EventReservations>()
        || !world.contains_resource::<WwiseRegistry>()
        || !world.contains_resource::<SoundDecodeQueue>()
    {
        return Err("sound preparation owner is absent".into());
    }
    world.resource_scope(|world, mut state: Mut<EventReservations>| {
        world.resource_scope(|world, mut decodes: Mut<SoundDecodeQueue>| {
            let mut registry = world.resource_mut::<WwiseRegistry>();
            if state.closed {
                return Err("prototype sound admission is closed".into());
            }
            if state.scopes.get(&owner).is_some_and(|scope| {
                scope.generation == registry.media_generation
                    && scope
                        .declarations
                        .iter()
                        .map(|event| event.id)
                        .eq(declarations.iter().map(|event| event.id))
            }) {
                return Ok(());
            }
            release(&mut state, &mut decodes, &mut registry, &owner);
            for declaration in &declarations {
                // c4550 game-defined declarations own descriptors/cookies,
                //not Wwise media. Keep their Arc inthis scope without an ID0 decode.
                if declaration.game_defined { continue; }
                if let Some(event) = state.events.get_mut(&declaration.id) {
                    event.users += 1;
                    continue;
                }
                let prepared = registry.sources.contains_key(&declaration.id);
                let status = if prepared {
                    registry.claim_dynamic_source(declaration.id, DecodeClient::Prototype);
                    State::Ready
                } else {
                    if registry.preloaded {
                        decodes.request(declaration.id, DecodeClient::Prototype, &registry);
                    }
                    state.pending += 1;
                    State::Pending
                };
                state.events.insert(
                    declaration.id,
                    Event {
                        users: 1,
                        state: status,
                    },
                );
            }
            state.scopes.insert(
                owner,
                Scope {
                    generation: registry.media_generation,
                    declarations,
                },
            );
            Ok(())
        })
    })
}

pub(in crate::app) fn events_ready(world: &World, owner: &str) -> Result<bool, String> {
    let state = world
        .get_resource::<EventReservations>()
        .ok_or("sound preparation owner is absent")?;
    let Some(scope) = state.scopes.get(owner) else {
        return Ok(false);
    };
    let registry = world
        .get_resource::<WwiseRegistry>()
        .ok_or("sound bank owner is absent")?;
    if scope.generation != registry.media_generation {
        return Err("prototype sound bank generation expired".into());
    }
    let mut ready = true;
    for declaration in &scope.declarations {
        if declaration.game_defined { continue; }
        match state.events.get(&declaration.id).map(|event| &event.state) {
            Some(State::Ready) if registry.sources.contains_key(&declaration.id) => {}
            Some(State::Failed(error)) => {
                return Err(format!("prototype sound {:#010x}: {error}", declaration.id));
            }
            _ => ready = false,
        }
    }
    Ok(ready)
}

fn release(
    state: &mut EventReservations,
    decodes: &mut SoundDecodeQueue,
    registry: &mut WwiseRegistry,
    owner: &str,
) {
    let Some(scope) = state.scopes.remove(owner) else {
        return;
    };
    for declaration in scope.declarations {
        if declaration.game_defined { continue; }
        let Some(event) = state.events.get_mut(&declaration.id) else {
            continue;
        };
        event.users -= 1;
        if event.users != 0 {
            continue;
        }
        if matches!(event.state, State::Pending) {
            state.pending -= 1;
        }
        state.events.remove(&declaration.id);
        decodes.release(declaration.id, DecodeClient::Prototype);
        registry.release_dynamic_source(declaration.id, DecodeClient::Prototype);
    }
}

pub(in crate::app) fn release_events(world: &mut World, owner: &str) -> Result<(), String> {
    if !world.contains_resource::<EventReservations>() {
        return Ok(());
    }
    world.resource_scope(|world, mut state: Mut<EventReservations>| {
        world.resource_scope(|world, mut decodes: Mut<SoundDecodeQueue>| {
            release(
                &mut state,
                &mut decodes,
                &mut world.resource_mut::<WwiseRegistry>(),
                owner,
            );
        });
    });
    Ok(())
}

pub(super) fn advance(
    mut state: ResMut<EventReservations>,
    mut decodes: ResMut<SoundDecodeQueue>,
    mut registry: ResMut<WwiseRegistry>,
) {
    if state.pending == 0 || !registry.preloaded {
        return;
    }
    decodes.poll();
    let mut completed = 0;
    for (&id, event) in &mut state.events {
        if !matches!(event.state, State::Pending) {
            continue;
        }
        // Another real consumer may already have published the same decode.
        if registry.sources.contains_key(&id) {
            registry.claim_dynamic_source(id, DecodeClient::Prototype);
            decodes.release(id, DecodeClient::Prototype);
            event.state = State::Ready;
            completed += 1;
        } else if let Some(result) = decodes.result(id) {
            event.state = match result {
                Ok(actions) => {
                    registry.publish_dynamic_source(id, actions, DecodeClient::Prototype);
                    State::Ready
                }
                Err(error) => State::Failed(error),
            };
            decodes.release(id, DecodeClient::Prototype);
            completed += 1;
        } else {
            decodes.request(id, DecodeClient::Prototype, &registry);
        }
    }
    state.pending -= completed;
}

pub(super) fn close(world: &mut World) {
    world.resource_scope(|world, mut state: Mut<EventReservations>| {
        state.closed = true;
        world.resource_scope(|world, mut decodes: Mut<SoundDecodeQueue>| {
            let mut registry = world.resource_mut::<WwiseRegistry>();
            for (id, _) in state.events.drain() {
                decodes.release(id, DecodeClient::Prototype);
                registry.release_dynamic_source(id, DecodeClient::Prototype);
            }
            state.scopes.clear();
            state.pending = 0;
        });
    });
}

/// Canceled jobs keep their bank/session lease until the worker acknowledges
/// completion. Dropping a caller is not an acquisition-completion signal.
pub(super) fn drain(world: &mut World) -> Result<bool, String> {
    let mut decodes = world.resource_mut::<SoundDecodeQueue>();
    decodes.poll();
    Ok(!decodes
        .0
        .values()
        .any(|job| job.clients == 0 && job.task.is_some()))
}
