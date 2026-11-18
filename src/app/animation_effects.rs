//!8755c0 -> normalA7 ->64bc20. AnimSet crossings create actual idEntity /
//!CpntFX actors through the canonical factory, never player-as-SELF effects.
use crate::{
    assets::{animation_rig::animset_effects::Effect, runtime_entity::Definition},
    gameplay::{
        entity_lifetime::{EntityId, LiveEntities},
        player::midnight::AnimationEffectReceipt,
        sound::Emitter,
    },
};
use bevy::{
    prelude::*,
    tasks::{block_on, futures_lite::future::poll_once, IoTaskPool, Task},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[derive(Clone, Hash, PartialEq, Eq)]
struct Parameters {
    path: Arc<str>,
    target_tag: Arc<str>,
    follow: bool,
}
impl From<&Effect> for Parameters {
    fn from(effect: &Effect) -> Self {
        Self {
            path: effect.effect.clone(),
            target_tag: effect.target_joint.clone(),
            follow: effect.update_position,
        }
    }
}
struct Entry {
    definition: Arc<Definition>,
    scope: Arc<str>,
    requested: bool,
}
struct Preparation {
    revision: [u64; 4],
    cancel: Arc<AtomicBool>,
    task: Mutex<Task<Result<Vec<(Parameters, Arc<Definition>)>, String>>>,
}
struct Delivery {
    receipt: AnimationEffectReceipt,
    source: Option<EntityId>,
}
#[derive(Resource, Default)]
pub(super) struct Effects {
    revision: Option<[u64; 4]>,
    pending: Option<Preparation>,
    entries: HashMap<Parameters, Entry>,
    deliveries: VecDeque<Delivery>,
    next_scope: u64,
    error: Option<String>,
    closing: bool,
    closed: bool,
}
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<Effects>() {
        return Err("previous animation FX acquisition owner has not retired".into());
    }
    world.insert_resource(Effects::default());
    Ok(())
}
fn capture(world: &mut World, owner: &mut Effects) {
    if owner.closed {
        return;
    }
    if let Some(mut state) = world.get_resource_mut::<super::player::PlayerState>() {
        if let Some(player) = &mut state.player {
            let identities: [Option<EntityId>; 5] =
                std::array::from_fn(|target| player.animation_entity(target));
            for receipt in player.drain_animation_effects() {
                let source = match receipt.emitter {
                    Emitter::Equipment { entity, .. } => Some(entity),
                    _ => identities.get(receipt.target).copied().flatten(),
                };
                owner.deliveries.push_back(Delivery { receipt, source });
            }
        }
    }
}

///Only changed canonical animation-bank metadata is visited. Asset work runs
///on the IO worker; factory reservations own every decoded/GPU dependency.
pub(super) fn prepare(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<Effects>() else { return Ok(()); };
    let result = (|| -> Result<(), String> {
        capture(world, &mut owner);
        if owner.closed {
            return Ok(());
        }
        if let Some(error) = &owner.error {
            return Err(error.clone());
        }
        if let Some(pending) = &mut owner.pending {
            if let Some(result) = block_on(poll_once(
                pending
                    .task
                    .get_mut()
                    .map_err(|_| "animation FX recipe worker poisoned")?,
            )) {
                let revision = pending.revision;
                let prepared = result?;
                let desired = prepared
                    .iter()
                    .map(|(key, _)| key.clone())
                    .collect::<HashSet<_>>();
                let pinned = owner
                    .deliveries
                    .iter()
                    .map(|delivery| Parameters::from(&delivery.receipt.effect))
                    .collect::<HashSet<_>>();
                for (key, entry) in &owner.entries {
                    if !desired.contains(key) && !pinned.contains(key) && entry.requested {
                        super::runtime_entities::release_for(
                            world,
                            &entry.scope,
                            &entry.definition,
                        )?;
                    }
                }
                owner
                    .entries
                    .retain(|key, _| desired.contains(key) || pinned.contains(key));
                for (key, definition) in prepared {
                    if owner.closing && !pinned.contains(&key) {
                        continue;
                    }
                    if owner.entries.contains_key(&key) {
                        continue;
                    }
                    owner.next_scope = owner.next_scope.wrapping_add(1);
                    let scope = Arc::from(format!("animation-fx:{}", owner.next_scope));
                    owner.entries.insert(
                        key,
                        Entry {
                            definition,
                            scope,
                            requested: false,
                        },
                    );
                }
                owner.revision = Some(revision);
                owner.pending = None;
            }
        }
        if owner.pending.is_none() {
            let player = world
                .get_resource::<super::player::PlayerState>()
                .and_then(|state| state.player.as_ref());
            if let Some(player) = player {
                let revision = player.animation_effect_revision();
                if (!owner.closing && owner.revision != Some(revision))
                    || owner.deliveries.iter().any(|delivery| {
                        !owner
                            .entries
                            .contains_key(&Parameters::from(&delivery.receipt.effect))
                    })
                {
                    let mut keys = HashSet::new();
                    if !owner.closing {
                        player.visit_animation_effects(|_, effect| {
                            keys.insert(Parameters::from(effect));
                        });
                    }
                    keys.extend(
                        owner
                            .deliveries
                            .iter()
                            .map(|delivery| Parameters::from(&delivery.receipt.effect)),
                    );
                    let catalog = world
                        .resource::<super::SceneData>()
                        .catalog
                        .clone()
                        .ok_or("animation FX catalog absent")?;
                    let cancel = Arc::new(AtomicBool::new(false));
                    let cancelled = cancel.clone();
                    let task = IoTaskPool::get().spawn(async move {
                        let mut definitions = Vec::with_capacity(keys.len());
                        for key in keys {
                            if cancelled.load(Ordering::Acquire) {
                                return Err("animation FX preparation cancelled".into());
                            }
                            let parameters = Arc::new(crate::assets::fx_component::Definition {
                                effect: Some(key.path.clone()),
                                target_tag: key.target_tag.clone(),
                                update_position: key.follow,
                                ..Default::default()
                            });
                            definitions
                                .push((key, Definition::animation_effect(&catalog, parameters)?));
                        }
                        Ok(definitions)
                    });
                    owner.pending = Some(Preparation {
                        revision,
                        cancel,
                        task: Mutex::new(task),
                    });
                }
            }
        }
        for entry in owner.entries.values_mut() {
            if !entry.requested {
                super::runtime_entities::request_for(
                    world,
                    entry.scope.clone(),
                    entry.definition.clone(),
                )?;
                entry.requested = true;
            }
        }
        Ok(())
    })();
    if let Err(error) = &result {
        owner.error = Some(error.clone());
    }
    world.insert_resource(owner);
    result
}

pub(super) fn startup_pending(world: &World) -> bool {
    let Some(player) = world
        .get_resource::<super::player::PlayerState>()
        .and_then(|state| state.player.as_ref())
    else {
        return false;
    };
    let Some(owner) = world.get_resource::<Effects>() else {
        return true;
    };
    owner.pending.is_some()
        || owner.revision != Some(player.animation_effect_revision())
        || owner.entries.values().any(|entry| {
            !entry.requested
                || !matches!(
                    super::runtime_entities::ready(world, &entry.definition),
                    Ok(true)
                )
        })
}
pub(super) fn startup_error(world: &World) -> Option<String> {
    let owner = world.get_resource::<Effects>()?;
    owner.error.clone().or_else(|| {
        owner
            .entries
            .values()
            .find_map(|entry| super::runtime_entities::ready(world, &entry.definition).err())
    })
}

pub(super) fn update(world: &mut World) -> Result<(), String> {
    prepare(world)?;
    let Some(mut owner) = world.remove_resource::<Effects>() else { return Ok(()); };
    let result = (|| -> Result<(), String> {
        while let Some(delivery) = owner.deliveries.front_mut() {
            let key = Parameters::from(&delivery.receipt.effect);
            let Some(entry) = owner.entries.get(&key) else {
                if owner.pending.is_some() {
                    return Ok(());
                }
                return Err(format!(
                    "accepted animation FX {} has no prepared recipe",
                    key.path
                ));
            };
            if !entry.requested || !super::runtime_entities::ready(world, &entry.definition)? {
                return Ok(());
            }
            let player = world
                .resource::<super::player::PlayerState>()
                .player
                .as_ref()
                .ok_or("animation FX source Player absent")?;
            let source = delivery
                .source
                .ok_or("accepted animation FX crossing had no actual managed source actor")?;
            world.resource::<LiveEntities>().get(source).ok_or(
                "accepted animation FX source retired before its callback acknowledgement",
            )?;
            let (joint, offset, orientation) =
                player.animation_effect_binding(source, &delivery.receipt.effect.joint)?;
            let binding = super::physics_scene::throwable::BindingPose {
                joint,
                offset,
                orientation,
                parent_placement: None,
            };
            super::runtime_entities::spawn_animation_effect(
                world,
                entry.definition.clone(),
                source,
                Some(source),
                Some(source),
                binding,
            )?;
            owner.deliveries.pop_front();
        }
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

pub(super) fn blocks_retirement(world: &World, id: EntityId) -> bool {
    world.get_resource::<Effects>().is_some_and(|owner| {
        owner
            .deliveries
            .iter()
            .any(|delivery| delivery.source == Some(id))
    })
}
pub(super) fn finish_accepted(world: &mut World) -> Result<bool, String> {
    update(world)?;
    Ok(world
        .get_resource::<Effects>()
        .is_none_or(|owner| owner.deliveries.is_empty()))
}
///Close new metadata admission before shared FX acquisition closes. Existing
///crossings and their already-required recipe work still drain normally.
pub(super) fn begin_retirement(world: &mut World) {
    let Some(mut owner) = world.remove_resource::<Effects>() else { return; };
    capture(world, &mut owner);
    owner.closing = true;
    world.insert_resource(owner);
}
pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    let Some(mut owner) = world.remove_resource::<Effects>() else {
        return Ok(true);
    };
    owner.closed = true;
    let result = (|| {
        if !owner.deliveries.is_empty() {
            return Err("animation FX shutdown precedes accepted A7 deliveries".into());
        }
        if let Some(pending) = &mut owner.pending {
            pending.cancel.store(true, Ordering::Release);
            if block_on(poll_once(
                pending
                    .task
                    .get_mut()
                    .map_err(|_| "animation FX recipe worker poisoned")?,
            ))
            .is_none()
            {
                return Ok(false);
            }
            owner.pending = None;
        }
        for entry in owner.entries.values_mut() {
            if entry.requested {
                super::runtime_entities::release_for(world, &entry.scope, &entry.definition)?;
                entry.requested = false;
            }
        }
        Ok(true)
    })();
    if !matches!(result, Ok(true)) {
        world.insert_resource(owner);
    }
    result
}
