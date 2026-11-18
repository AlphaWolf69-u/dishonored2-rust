//! Host of arkKisculeNodeEntitySpawnerControl (14079dc40): constructs the
//! requested declaration through the runtime entity owner and returns the
//! created entity to the requesting graph node.
use crate::gameplay::effects::spawner::{EntitySpawnRequest, EntitySpawner};
use crate::gameplay::event_graph::EntityReference;
use crate::gameplay::reference_frame::ReferenceFrame;
use bevy::prelude::*;
use std::sync::Arc;

#[derive(Resource, Default)]
pub(super) struct ScriptEntitySpawner {
    mailbox: Arc<EntitySpawner>,
    /// Requests whose declaration prototype is still being prepared.
    pending: Vec<(EntitySpawnRequest, Arc<crate::assets::runtime_entity::Definition>)>,
}

/// Attach at scene publication, with the gameflow mailbox.
pub(super) fn attach(world: &mut World, effects: &mut crate::rendering::bevy_backend::camera_scene::FxSimulation) {
    let owner = world.get_resource_or_insert_with(ScriptEntitySpawner::default);
    effects.set_spawner(owner.mailbox.clone());
}

/// 14079dc40: a missing declaration or a zero location without a target logs
/// and takes Failed; the target's origin and axis replace m_location and
/// m_rotation; the entity spawns with m_spawnTag.
pub(super) fn update(world: &mut World) {
    let Some(mut owner) = world.remove_resource::<ScriptEntitySpawner>() else {
        return;
    };
    let mailbox = owner.mailbox.clone();
    let requests = match mailbox.take_requests() {
        Ok(requests) => requests,
        Err(error) => {
            warn!("Script entity spawner: {error}");
            Vec::new()
        }
    };
    for request in requests {
        match admit(world, &request) {
            Ok(definition) => owner.pending.push((request, definition)),
            Err(error) => complete(&mailbox, request, Err(error)),
        }
    }
    let mut waiting = Vec::new();
    for (request, definition) in std::mem::take(&mut owner.pending) {
        match super::runtime_entities::ready(world, &definition) {
            Ok(false) => waiting.push((request, definition)),
            Ok(true) => {
                let result = spawn(world, &request, definition);
                complete(&mailbox, request, result);
            }
            Err(error) => complete(&mailbox, request, Err(error)),
        }
    }
    owner.pending = waiting;
    world.insert_resource(owner);
}

fn complete(mailbox: &EntitySpawner, request: EntitySpawnRequest, result: Result<String, String>) {
    if let Err(error) = &result {
        warn!(declaration = %request.declaration, "Script entity spawn failed: {error}");
    }
    if let Err(error) = mailbox.complete(request.owner, request.node, result) {
        warn!("Script entity spawner: {error}");
    }
}

fn admit(
    world: &mut World,
    request: &EntitySpawnRequest,
) -> Result<Arc<crate::assets::runtime_entity::Definition>, String> {
    if request.declaration.is_empty() {
        return Err("spawner has no entity declaration".into());
    }
    let catalog = world
        .get_resource::<super::SceneData>()
        .and_then(|scene| scene.catalog.clone())
        .ok_or("spawner catalog absent")?;
    let definition = crate::assets::runtime_entity::Definition::acquire(&catalog, &request.declaration, None)?;
    super::runtime_entities::request_for(world, Arc::from("script entity spawner"), definition.clone())?;
    Ok(definition)
}

fn spawn(
    world: &mut World,
    request: &EntitySpawnRequest,
    definition: Arc<crate::assets::runtime_entity::Definition>,
) -> Result<String, String> {
    let frame = match &request.target {
        Some(EntityReference::Named(name)) => {
            let live = world
                .get_resource::<crate::gameplay::entity_lifetime::LiveEntities>()
                .ok_or("spawner entity owner absent")?;
            let id = live.find(name).ok_or_else(|| format!("spawner target {name} is absent"))?;
            world
                .get_resource::<super::physics_scene::PhysicsScene>()
                .ok_or("spawner scene absent")?
                .entity_frame(live, id)?
                .ok_or_else(|| format!("spawner target {name} has no frame"))?
        }
        Some(other) => return Err(format!("spawner target {other:?} is not a placed entity")),
        None => {
            if request.location == [0.0; 3] {
                return Err("spawner has neither a target nor a location".into());
            }
            if request.rotation != [0.0; 3] {
                return Err("spawner m_rotation without a target is not traced".into());
            }
            ReferenceFrame { position: request.location, ..ReferenceFrame::default() }
        }
    };
    let id = super::runtime_entities::spawn(
        world,
        definition,
        super::runtime_entities::SpawnOptions { frame, scale: None, hidden_disabled: false },
    )?;
    let mut live = world
        .get_resource_mut::<crate::gameplay::entity_lifetime::LiveEntities>()
        .ok_or("spawner entity owner absent")?;
    if !request.spawn_tag.is_empty() {
        live.publish_spawn_tag(id, &request.spawn_tag)?;
    }
    let name = live.get(id).map(|(name, _)| name.to_owned()).ok_or("spawned entity is stale")?;
    Ok(name)
}
