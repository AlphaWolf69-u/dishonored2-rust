//! Managed identity of the already constructed gameplay player.
//!
//! 1408ad9c0 names client zero `player1`, passes the explicit client slot to
//! 1408aeb10, then 1408aecc0 registers that same object through 140657ae0.
//! Class registration 1413c6010 binds arkPlayer -> idEntity; factory 1408f5790
//! calls 140784450 and the ordinary idEntity constructor 140874c50 (MAIN tag).
//! This adapter owns only that identity, never a second Player or health/body.
use bevy::prelude::*;
use std::sync::Arc;

use crate::gameplay::{
    component_state::types::ComponentTypes,
    entity_lifetime::{EntityId, LiveEntities, Stage},
};

#[derive(Resource)]
struct PlayerLifetime {
    entity: Option<EntityId>,
    // The class index and MAIN name belong to the canonical catalog metadata.
    // Keep that actual owner alive until this map's identity has retired.
    types: Arc<ComponentTypes>,
    class: usize,
    retired: bool,
}

/// Return a generation only while its actual Player and managed entry exist.
pub(super) fn id(world: &World) -> Option<EntityId> {
    world.get_resource::<super::player::PlayerState>()?.player.as_ref()?;
    let owner = world.get_resource::<PlayerLifetime>()?;
    let entity = owner.entity?;
    let live = world.get_resource::<LiveEntities>()?;
    let classes = owner.types.classes();
    (classes.metadata(owner.class).and_then(|class| classes.name_text(class.name)) == Some(b"arkPlayer")
        && live.stage(entity) == Some(Stage::Spawned)
        && live.get(entity).is_some_and(|(name, class)| name == "player1" && class == owner.class))
        .then_some(entity)
}

/// Called after the setup command batch publishes the successfully constructed
/// PlayerState. Diagnostic free cameras have no player and allocate no header.
pub(super) fn admit(world: &mut World) -> Result<(), String> {
    let actual = world.get_resource::<super::player::PlayerState>()
        .is_some_and(|state| state.player.is_some());
    if !actual {
        if world.get_resource::<PlayerLifetime>().is_some_and(|owner| owner.entity.is_some()) {
            return Err("managed player outlived its gameplay owner".into());
        }
        return Ok(());
    }
    if let Some(owner) = world.get_resource::<PlayerLifetime>() {
        if owner.retired { return Err("cannot readmit a retired player in the same world".into()); }
        let entity = id(world).ok_or("published player identity is stale")?;
        bind_power_owner(world, entity);
        return Ok(());
    }
    let catalog = world.get_resource::<super::SceneData>()
        .and_then(|scene| scene.catalog.as_ref())
        .ok_or("player admission has no installed class metadata source")?;
    let types = ComponentTypes::acquire(catalog)?;
    let classes = types.classes();
    let class = classes.hierarchy().registration_order().iter().copied().find(|&class| {
        classes.metadata(class).and_then(|class| classes.name_text(class.name)) == Some(b"arkPlayer")
    }).ok_or("installed arkPlayer class registration is absent")?;
    // Both installed character body.def declarations inherit arkPlayer from
    // player/base/body.def and leave the idEntity constructor's tag unchanged.
    let tag = types.intern_name(Some(b"MAIN"))?;
    let mut live = world.get_resource_mut::<LiveEntities>()
        .ok_or("player admission has no canonical managed entity owner")?;
    let entity = live.allocate_managed(Arc::from("player1"), class, Some(0), 0, tag)?;
    // Player::new has already performed the supported component construction
    // and spawn preparation. Publish its identity stages only after success.
    let publication = live.initialized(entity)
        .and_then(|()| live.spawned(entity))
        .and_then(|()| live.publish_spawn_tag(entity, "MAIN"));
    if let Err(error) = publication {
        // No external consumer has received this generation yet.
        if live.retire_for_world_shutdown(entity).is_none() {
            return Err(format!("player identity publication failed: {error}; rollback could not retire {entity:?}"));
        }
        return Err(error);
    }
    drop(live);
    world.insert_resource(PlayerLifetime { entity: Some(entity), types, class, retired: false });
    bind_power_owner(world, entity);
    Ok(())
}

fn bind_power_owner(world: &mut World, entity: EntityId) {
    if let Some(player) = world.get_resource_mut::<super::player::PlayerState>()
        .as_deref_mut().and_then(|state| state.player.as_mut()) {
        player.set_managed_entity(Some(entity));
    }
    if let Some(mut powers) = world.get_resource_mut::<super::power_world::PowerWorld>() {
        powers.bind_player(entity);
    }
}

/// Whole-world shutdown: stop the real player-bound powers while clock, event,
/// audio and managed-generation owners remain. The caller subsequently drains
/// typed components/audio and drops PlayerState through LevelLifetime.
pub(super) fn retire(world: &mut World) -> Result<(), String> {
    super::power_world::retire(world)?;
    let Some(owner) = world.get_resource::<PlayerLifetime>() else { return Ok(()); };
    if owner.retired { return Ok(()); }
    let entity = owner.entity.ok_or("player retirement lost its managed generation")?;
    let live = world.get_resource::<LiveEntities>()
        .ok_or("player retirement lost the canonical managed entity owner")?;
    if live.get(entity).is_none() { return Err("player retirement generation is stale".into()); }
    // 140199df0/graph-owner retirement precedes removal of the identity used
    // by queued receiver callbacks; use the existing manager, not a copy.
    super::entity_graphs::retire(world, entity)?;
    if world.resource_mut::<LiveEntities>().retire_for_world_shutdown(entity).is_none() {
        return Err(format!("player {entity:?} still owns attachments or managed membership"));
    }
    if let Some(player) = world.get_resource_mut::<super::player::PlayerState>()
        .as_deref_mut().and_then(|state| state.player.as_mut()) {
        player.set_managed_entity(None);
    }
    let mut owner = world.resource_mut::<PlayerLifetime>();
    owner.entity = None;
    owner.retired = true;
    // Retain the metadata Arc until ordinary level resource teardown, so no
    // class/name pointer can outlive its registry.
    Ok(())
}
