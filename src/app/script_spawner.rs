//!79dc40's actual graph/node continuation over the canonical entity factory.
use crate::{
    assets::runtime_entity::Definition,
    gameplay::{
        effects::spawner::{EntitySpawnRequest, EntitySpawner},
        entity_lifetime::{EntityId, LiveEntities},
        event_graph::EntityReference,
        reference_frame::ReferenceFrame,
    },
};
use bevy::{
    prelude::*,
    tasks::{block_on, futures_lite::future::poll_once, IoTaskPool, Task},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct Pending {
    request: EntitySpawnRequest,
    scope: Arc<str>,
    acquisition: Option<Mutex<Task<Result<Arc<Definition>, String>>>>,
    definition: Option<Arc<Definition>>,
    requested: bool,
    frame: Option<ReferenceFrame>,
    born: Option<EntityId>,
    result: Option<Result<EntityId, String>>,
    replied: bool,
}
#[derive(Resource, Default)]
pub(super) struct ScriptEntitySpawner {
    mailbox: Arc<EntitySpawner>,
    pending: VecDeque<Pending>,
}

pub(super) fn attach(
    world: &mut World,
    effects: &mut crate::rendering::bevy_backend::camera_scene::FxSimulation,
) {
    let owner = world.get_resource_or_insert_with(ScriptEntitySpawner::default);
    effects.set_spawner(owner.mailbox.clone());
}
fn frame(request: &EntitySpawnRequest) -> Result<ReferenceFrame, String> {
    let frame = if let Some(frame) = request.target_frame {
        frame
    } else {
        if request.target.is_some() {
            return Err("spawner target has no owned activation pose".into());
        }
        if request.rotation != [0.; 3] {
            return Err("spawner m_rotation without a target is not traced".into());
        }
        ReferenceFrame {
            position: request.location,
            ..Default::default()
        }
    };
    //79dc40 tests zero after applying the target override, including -0.
    if frame.position == [0.; 3] {
        return Err("spawner has no valid location".into());
    }
    Ok(frame)
}
fn admit(world: &World, request: EntitySpawnRequest) -> Pending {
    let scope = Arc::from(format!(
        "script-spawn:{}:{}:{}",
        request.receipt.instance, request.receipt.node, request.receipt.attempt
    ));
    let mut pending = Pending {
        request,
        scope,
        acquisition: None,
        definition: None,
        requested: false,
        frame: None,
        born: None,
        result: None,
        replied: false,
    };
    let result = (|| -> Result<(), String> {
        if pending.request.declaration.is_empty() {
            return Err("spawner has no entity declaration".into());
        }
        pending.frame = Some(frame(&pending.request)?);
        let catalog = world
            .get_resource::<super::SceneData>()
            .and_then(|scene| scene.catalog.clone())
            .ok_or("spawner catalog absent")?;
        let declaration = pending.request.declaration.clone();
        //One accepted request owns this work. The canonical decoder/factory
        //retain resources; no second definition/image/model cache is created.
        pending.acquisition =
            Some(Mutex::new(IoTaskPool::get().spawn(async move {
                Definition::acquire(&catalog, &declaration, None)
            })));
        Ok(())
    })();
    if let Err(error) = result {
        pending.result = Some(Err(error));
    }
    pending
}
pub(super) fn update(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<ScriptEntitySpawner>() else {
        return Ok(());
    };
    let result = (|| -> Result<(), String> {
        //Closing rejects NEW submissions, not already accepted requests.
        for request in owner.mailbox.take_requests()? {
            owner.pending.push_back(admit(world, request));
        }
        while let Some(pending) = owner.pending.front_mut() {
            if pending.result.is_none() {
                if let Some(acquisition) = pending.acquisition.as_mut() {
                    let Some(result) = block_on(poll_once(
                        acquisition
                            .get_mut()
                            .map_err(|_| "script entity acquisition poisoned")?,
                    )) else {
                        break;
                    };
                    pending.acquisition = None;
                    match result {
                        Ok(definition) => pending.definition = Some(definition),
                        Err(error) => pending.result = Some(Err(error)),
                    }
                }
                if pending.result.is_none() {
                    let progress = (|| -> Result<bool, String> {
                        let definition = pending
                            .definition
                            .as_ref()
                            .ok_or("accepted spawner definition absent")?;
                        if !pending.requested {
                            //request_for rolls its newly admitted scopes back on error.
                            super::runtime_entities::request_for(
                                world,
                                pending.scope.clone(),
                                definition.clone(),
                            )?;
                            pending.requested = true;
                        }
                        if !super::runtime_entities::ready(world, definition)? {
                            return Ok(false);
                        }
                        if pending.born.is_none() {
                            let source =
                                match &pending.request.target {
                                    Some(EntityReference::Managed(id)) => Some(*id),
                                    None => None,
                                    _ => return Err(
                                        "spawner target was not captured as a managed generation"
                                            .into(),
                                    ),
                                };
                            if let Some(source) = source {
                                world.resource::<LiveEntities>().get(source).ok_or(
                                    "accepted spawner source retired before its constructor",
                                )?;
                            }
                            pending.born = Some(super::runtime_entities::spawn_script(
                                world,
                                definition.clone(),
                                pending.frame.ok_or("spawner activation frame absent")?,
                                source,
                                pending.request.target_spawn_tag.clone(),
                            )?);
                        }
                        let id = pending.born.unwrap();
                        if !pending.request.spawn_tag.is_empty() {
                            let mut live = world.resource_mut::<LiveEntities>();
                            let tag = definition
                                .types
                                .intern_name(Some(pending.request.spawn_tag.as_bytes()))?;
                            live.set_published_spawn_tag(id, tag, &pending.request.spawn_tag)?;
                        }
                        pending.result = Some(Ok(id));
                        Ok(true)
                    })();
                    match progress {
                        Ok(false) => break,
                        Ok(true) => {}
                        //Before birth native79dc40 emits Failed. Once born,
                        //retry only the remaining host publication, never spawn twice.
                        Err(error) if pending.born.is_none() => pending.result = Some(Err(error)),
                        Err(error) => return Err(error),
                    }
                }
            }
            if !pending.replied {
                let result = pending
                    .result
                    .as_ref()
                    .ok_or("spawner completion absent")?
                    .clone();
                if let Err(error) = &result {
                    warn!(declaration=%pending.request.declaration,"Script entity spawn failed: {error}");
                }
                owner.mailbox.complete(pending.request.receipt, result)?;
                pending.replied = true;
            }
            if pending.requested {
                super::runtime_entities::release_for(
                    world,
                    &pending.scope,
                    pending
                        .definition
                        .as_ref()
                        .ok_or("spawner lease lost its definition")?,
                )?;
                pending.requested = false;
            }
            owner.pending.pop_front();
        }
        Ok(())
    })();
    world.insert_resource(owner);
    result?;
    if let Some(mut effects) =
        world.get_resource_mut::<crate::rendering::bevy_backend::camera_scene::FxSimulation>()
    {
        effects.poll_script_spawns();
    }
    Ok(())
}
pub(super) fn begin_retirement(world: &mut World) -> Result<(), String> {
    if let Some(mut owner) = world.get_resource_mut::<ScriptEntitySpawner>() {
        owner.mailbox.close()?;
    }
    Ok(())
}
pub(super) fn finish_accepted(world: &mut World) -> Result<bool, String> {
    update(world)?;
    world
        .get_resource::<ScriptEntitySpawner>()
        .map_or(Ok(true), |owner| {
            Ok(owner.pending.is_empty() && owner.mailbox.idle()?)
        })
}
pub(super) fn startup_pending(world: &World) -> bool {
    world
        .get_resource::<ScriptEntitySpawner>()
        .is_some_and(|owner| !owner.pending.is_empty() || !matches!(owner.mailbox.idle(), Ok(true)))
}
pub(super) fn blocks_retirement(world: &World, id: EntityId) -> bool {
    world
        .get_resource::<ScriptEntitySpawner>()
        .is_some_and(|owner| {
            owner.pending.iter().any(|pending| {
                !pending.replied
                    && (pending.request.graph_entity == Some(id)
                        || pending.request.target == Some(EntityReference::Managed(id)))
            }) || !matches!(owner.mailbox.references(id), Ok(false))
        })
}
pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    begin_retirement(world)?;
    if !finish_accepted(world)? {
        return Ok(false);
    }
    world.remove_resource::<ScriptEntitySpawner>();
    Ok(true)
}
