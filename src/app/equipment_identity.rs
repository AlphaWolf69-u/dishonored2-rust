//! Managed identities for the actual player equipment and selected ammo child.
//! 619380 marks player equipment +d4 bit3 before 8aecc0/657ae0 registration;
//! 44dd00/62b240 spawn the ammo child normally, then set bit4 and bind it.
//! Model leases/fences remain with PlayerArms, not with this identity table.
use crate::gameplay::{
    component_state::types::ComponentTypes,
    entity_lifetime::{EntityId, LiveEntities, Stage},
};
use bevy::prelude::*;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    parent: EntityId,
    identity: String,
    declaration: String,
    ammo: bool,
}
struct Entry {
    key: Key,
    linked: bool,
    runtime_flags: u8,
    frame: Option<crate::gameplay::reference_frame::ReferenceFrame>,
    time_scale: f32,
}
#[derive(Resource)]
struct Identities {
    types: Arc<ComponentTypes>,
    entries: HashMap<EntityId, Entry>,
    pending: Vec<(EntityId, super::player_arms::RetiredModel)>,
}

/// Setup publication calls this before LevelLifetime captures its resources.
pub(super) fn initialize(world: &mut World) -> Result<(), String> {
    if world.contains_resource::<Identities>() {
        return Ok(());
    }
    let catalog = world
        .get_resource::<super::SceneData>()
        .and_then(|scene| scene.catalog.as_ref())
        .ok_or("equipment identity has no class metadata source")?;
    let types = ComponentTypes::acquire(catalog)?;
    world.insert_resource(Identities {
        types,
        entries: HashMap::new(),
        pending: Vec::new(),
    });
    Ok(())
}

/// Only a real admitted player can own equipment (619380). This function is
/// called after the existing equipment component/runtime and model prepare.
pub(super) fn admit_equipment(
    world: &mut World,
    identity: &str,
    declaration: &str,
    class: &str,
) -> Result<EntityId, String> {
    let parent = super::player_lifetime::id(world).ok_or("equipment has no live player owner")?;
    admit(world, identity, declaration, class, parent, false)
}

/// The actual weapon generation is the parent; never bind a bolt to a stand-in
/// player or use model readiness as a fabricated child identity.
pub(super) fn admit_ammunition(
    world: &mut World,
    descriptor: &crate::assets::equipment::attachment::Descriptor,
    weapon: EntityId,
) -> Result<EntityId, String> {
    let identities = world
        .get_resource::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    if !identities
        .entries
        .get(&weapon)
        .is_some_and(|entry| !entry.key.ammo && entry.linked)
    {
        return Err("ammunition parent is not an admitted weapon generation".into());
    }
    admit(
        world,
        &descriptor.entity,
        &descriptor.entity,
        &descriptor.class,
        weapon,
        true,
    )
}

fn admit(
    world: &mut World,
    identity: &str,
    declaration: &str,
    class: &str,
    parent: EntityId,
    ammo: bool,
) -> Result<EntityId, String> {
    let key = Key {
        parent,
        identity: identity.to_owned(),
        declaration: declaration.to_owned(),
        ammo,
    };
    if let Some((&id, _entry)) = world.get_resource::<Identities>().and_then(|owner| {
        owner
            .entries
            .iter()
            .find(|(_, entry)| entry.linked && entry.key == key)
    }) {
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("equipment managed owner is absent")?;
        if live.stage(id) != Some(Stage::Spawned) || live.parent(id)? != Some(parent) {
            return Err("equipment identity is retiring or stale".into());
        }
        return Ok(id);
    }
    if !world.contains_resource::<Identities>() {
        return Err("equipment identity was not initialized during level publication".into());
    }
    let types = world.resource::<Identities>().types.clone();
    let classes = types.classes();
    let class_id = classes
        .hierarchy()
        .registration_order()
        .iter()
        .copied()
        .find(|&index| {
            classes
                .metadata(index)
                .and_then(|metadata| classes.name_text(metadata.name))
                == Some(class.as_bytes())
        })
        .ok_or_else(|| format!("equipment class is not registered: {class}"))?;
    let tag = world
        .get_resource::<LiveEntities>()
        .ok_or("equipment managed owner is absent")?
        .spawn_tag(parent)?;
    let mut live = world.resource_mut::<LiveEntities>();
    // idEntity constructor uses MAIN. Equipment619380 copies the actual
    // owning player's tag before registration; ammo8ae9b0 leaves MAIN intact.
    let main = types.intern_name(Some(b"MAIN"))?;
    let tag = if ammo { main } else { tag };
    if tag != main {
        return Err("equipment player spawn tag has no admitted spelling".into());
    }
    let runtime_flags = if ammo { 0 } else { 8 };
    let id = live.allocate_managed_auto_name(declaration, class_id, runtime_flags, tag)?;
    let publication = live
        .initialized(id)
        .and_then(|()| live.spawned(id))
        .and_then(|()| live.publish_spawn_tag(id, "MAIN"));
    if let Err(error) = publication {
        if live.retire_for_world_shutdown(id).is_none() {
            return Err(format!(
                "equipment publication failed: {error}; rollback retained {id:?}"
            ));
        }
        return Err(error);
    }
    let linked = live.link_child(parent, id);
    if let Err(error) = linked {
        if live.retire_for_world_shutdown(id).is_none() {
            return Err(format!(
                "equipment binding failed: {error}; rollback retained {id:?}"
            ));
        }
        return Err(error);
    }
    drop(live);
    world.resource_mut::<Identities>().entries.insert(
        id,
        Entry {
            key,
            linked: true,
            runtime_flags: if ammo { 0x10 } else { runtime_flags },
            frame: None,
            time_scale: 1.0,
        },
    );
    Ok(id)
}

pub(super) fn runtime_flags(world: &World, id: EntityId) -> Option<u8> {
    world
        .get_resource::<Identities>()?
        .entries
        .get(&id)
        .map(|entry| entry.runtime_flags)
}

pub(super) fn equipment(world: &World, identity: &str) -> Option<EntityId> {
    let owner = world.get_resource::<Identities>()?;
    owner.entries.iter().find_map(|(&id, entry)| {
        (!entry.key.ammo && entry.linked && entry.key.identity == identity).then_some(id)
    })
}

pub(super) fn id(world: &World, identity: &str) -> Option<EntityId> {
    equipment(world, identity)
}

/// The real equipment animation/binding owner supplies its entity frame and
/// clock scale. No arm-slot alias or inferred model root becomes an identity.
pub(super) fn publish_frame(
    world: &mut World,
    id: EntityId,
    frame: crate::gameplay::reference_frame::ReferenceFrame,
    time_scale: f32,
) -> Result<(), String> {
    if !time_scale.is_finite() || time_scale < 0.0 {
        return Err("equipment time scale is invalid".into());
    }
    let mut owner = world
        .get_resource_mut::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    let entry = owner
        .entries
        .get_mut(&id)
        .ok_or("equipment frame targeted a stale generation")?;
    if !entry.linked {
        return Err("equipment frame targeted a detached generation".into());
    }
    entry.frame = Some(frame);
    entry.time_scale = time_scale;
    Ok(())
}

pub(super) fn frame(
    world: &World,
    id: EntityId,
) -> Option<(crate::gameplay::reference_frame::ReferenceFrame, f32)> {
    let owner = world.get_resource::<Identities>()?;
    let entry = owner.entries.get(&id)?;
    entry.linked.then_some((entry.frame?, entry.time_scale))
}

/// Withdraw the actual hierarchy binding when the render owner withdraws its
/// surfaces. Keep the generation until that owner's real GPU fence completes.
pub(super) fn detach(world: &mut World, id: EntityId) -> Result<(), String> {
    let owner = world
        .get_resource::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    let entry = owner
        .entries
        .get(&id)
        .ok_or("equipment generation is not owned")?;
    if !entry.linked {
        return Ok(());
    }
    let parent = entry.key.parent;
    world
        .get_resource_mut::<LiveEntities>()
        .ok_or("equipment managed owner is absent")?
        .unlink_child(parent, id)?;
    let mut owner = world.resource_mut::<Identities>();
    owner
        .entries
        .get_mut(&id)
        .ok_or("equipment owner disappeared")?
        .linked = false;
    Ok(())
}

/// Called only after the existing PlayerArms submitted-work acknowledgement.
/// Source/model leases stay with the retired render payload through that ack.
pub(super) fn retire_acknowledged(world: &mut World, id: EntityId) -> Result<(), String> {
    let owner = world
        .get_resource::<Identities>()
        .ok_or("equipment identity owner is absent")?;
    let Some(entry) = owner.entries.get(&id) else {
        return Ok(());
    };
    if entry.linked {
        return Err("equipment retirement preceded hierarchy detach".into());
    }
    super::entity_graphs::retire(world, id)?;
    if world
        .resource_mut::<LiveEntities>()
        .retire_for_world_shutdown(id)
        .is_none()
    {
        return Err("equipment identity still owns children or managed references".into());
    }
    world.resource_mut::<Identities>().entries.remove(&id);
    Ok(())
}

/// Accept the exact renderer receipt after draw withdrawal; never make a
/// frame-count or acquisition-complete event stand in for this GPU fence.
pub(super) fn await_render_retirement(
    world: &mut World,
    id: EntityId,
    receipt: super::player_arms::RetiredModel,
) -> Result<(), String> {
    let mut owner = world.resource_mut::<Identities>();
    if !owner.entries.contains_key(&id) {
        return Err("render retirement has no owned equipment generation".into());
    }
    if !owner.pending.iter().any(|(pending, _)| *pending == id) {
        owner.pending.push((id, receipt));
    }
    drop(owner);
    // Retain the real receipt before a fallible hierarchy callback. A retry
    // must never mistake already withdrawn surfaces for an unrendered child.
    detach(world, id)
}

pub(super) fn retirement_pending(world: &World, id: EntityId) -> bool {
    world
        .get_resource::<Identities>()
        .is_some_and(|owner| owner.pending.iter().any(|(pending, _)| *pending == id))
}

/// Ordinary per-frame completion polling by the existing admission owner.
/// Retire children first, even when one submission acknowledged both models.
pub(super) fn poll_retirements(world: &mut World) -> Result<bool, String> {
    loop {
        let Some(owner) = world.get_resource::<Identities>() else {
            return Ok(true);
        };
        let next = owner.pending.iter().find_map(|(id, receipt)| {
            (receipt.complete()
                && owner.entries.get(id).is_some_and(|entry| !entry.linked)
                && !owner.entries.values().any(|entry| entry.key.parent == *id))
            .then_some(*id)
        });
        let Some(id) = next else {
            return Ok(owner.pending.is_empty());
        };
        retire_acknowledged(world, id)?;
        let mut owner = world.resource_mut::<Identities>();
        let index = owner
            .pending
            .iter()
            .position(|(pending, _)| *pending == id)
            .ok_or("equipment retirement receipt disappeared")?;
        owner.pending.swap_remove(index);
    }
}

/// After all actual equipment/attachment renderer withdrawals, wait for their
/// receipts before allowing the player generation and level owner to retire.
pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    if !poll_retirements(world)? {
        return Ok(false);
    }
    if world
        .get_resource::<Identities>()
        .is_some_and(|owner| !owner.entries.is_empty())
    {
        return Err("equipment shutdown still owns active entity generations".into());
    }
    world.remove_resource::<Identities>();
    Ok(true)
}
