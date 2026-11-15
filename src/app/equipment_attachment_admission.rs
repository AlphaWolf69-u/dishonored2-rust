//! Selected-ammo entity requests through the existing equipment Sources and
//! material worker. Old draw/source leases retire through PlayerArms' fence.
use crate::{
    assets::equipment::{Descriptor as Weapon, attachment::Descriptor},
    gameplay::{
        entity_lifetime::{EntityId, LiveEntities, Stage},
        reference_frame::ReferenceFrame,
    },
};
use bevy::{
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

struct Pending {
    cancelled: Arc<AtomicBool>,
    task: Mutex<Task<Result<Arc<Descriptor>, String>>>,
}
struct Entry {
    weapon: Arc<Weapon>,
    ammunition: String,
    path: String,
    tag: String,
    visible: bool,
    child: Option<EntityId>,
    descriptor: Option<Arc<Descriptor>>,
    pending: Option<Pending>,
    failed: Option<String>,
}
#[derive(Resource, Default)]
struct Attachments {
    live: HashMap<EntityId, Entry>,
    cancelled: Vec<Pending>,
    closing: bool,
}

/// Root setup invokes before LevelLifetime records its resource IDs.
pub(super) fn initialize(world: &mut World) {
    world.init_resource::<Attachments>();
}

/// Called by the native selected-ammunition change, never for every inventory
/// ammo variant. The caller supplies the real declaration and bind tag.
pub(super) fn select(
    world: &mut World,
    weapon_id: EntityId,
    weapon: Arc<Weapon>,
    ammunition: &str,
    attachment: Option<&str>,
    tag: Option<&str>,
    visible: bool,
) -> Result<(), String> {
    let mut owner = world
        .remove_resource::<Attachments>()
        .ok_or("ammunition owner was not initialized during level publication")?;
    let result = (|| {
        if owner.closing {
            return Err("ammunition acquisition is closed".into());
        }
        let path = attachment.filter(|path| !path.is_empty() && !path.eq_ignore_ascii_case("NULL"));
        if let Some(entry) = owner.live.get_mut(&weapon_id) {
            if entry.ammunition == ammunition && path == Some(entry.path.as_str()) {
                entry.visible = visible;
                if let Some(child) = entry.child {
                    if let Some(mut arms) =
                        world.get_resource_mut::<super::player_arms::PlayerArms>()
                    {
                        if arms.attachments.has(child) {
                            arms.attachments.set_visible(child, visible)?;
                        }
                    }
                }
                return Ok(());
            }
        }
        if let Some(mut old) = owner.live.remove(&weapon_id) {
            if let Err(error) = withdraw(world, &mut old, &mut owner.cancelled) {
                owner.live.insert(weapon_id, old);
                return Err(error);
            }
        }
        let Some(path) = path else {
            return Ok(());
        };
        let tag = tag.ok_or("selected ammunition has no authored weapon bind tag")?;
        // Validate while this immutable rig is borrowed, before any worker can
        // reserve/publish child draw entities. There is no fallback socket.
        weapon.rig.tag(tag)?;
        let live = world
            .get_resource::<LiveEntities>()
            .ok_or("ammunition has no managed owner")?;
        if live.stage(weapon_id) != Some(Stage::Spawned) {
            return Err("ammunition weapon generation is stale".into());
        }
        let (catalog, sources) = world
            .get_resource::<super::player_actions::PlayerActions>()
            .ok_or("ammunition has no canonical equipment sources")?
            .equipment_attachment_source();
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancellation = cancelled.clone();
        let requested = path.to_owned();
        let task = IoTaskPool::get().spawn(async move {
            let stopped = || cancellation.load(Ordering::Acquire);
            let mut sources = sources
                .lock()
                .map_err(|_| "equipment source owner is poisoned")?;
            sources.acquire_attachment(&catalog, &requested, Some(&stopped))
        });
        owner.live.insert(
            weapon_id,
            Entry {
                weapon,
                ammunition: ammunition.to_owned(),
                path: path.to_owned(),
                tag: tag.to_owned(),
                visible,
                child: None,
                descriptor: None,
                pending: Some(Pending {
                    cancelled,
                    task: Mutex::new(task),
                }),
                failed: None,
            },
        );
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

fn completed(pending: &mut Pending) -> Result<Option<Result<Arc<Descriptor>, String>>, String> {
    let mut task = pending
        .task
        .lock()
        .map_err(|_| "ammunition acquisition task is poisoned")?;
    Ok(block_on(poll_once(&mut *task)))
}

pub(super) fn advance(world: &mut World) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<Attachments>() else {
        return Ok(());
    };
    let result = (|| {
        let mut index = 0;
        while index < owner.cancelled.len() {
            if completed(&mut owner.cancelled[index])?.is_some() {
                owner.cancelled.swap_remove(index);
            } else {
                index += 1;
            }
        }
        for (&weapon, entry) in &mut owner.live {
            if let Some(error) = &entry.failed {
                return Err(error.clone());
            }
            if let Some(pending) = entry.pending.as_mut() {
                if let Some(result) = completed(pending)? {
                    entry.pending = None;
                    match result {
                        Ok(descriptor) => entry.descriptor = Some(descriptor),
                        Err(error) => {
                            entry.failed = Some(error.clone());
                            return Err(error);
                        }
                    }
                }
            }
            let Some(descriptor) = entry.descriptor.as_ref() else {
                continue;
            };
            let child = match entry.child {
                Some(child) => child,
                None => {
                    let child =
                        super::equipment_identity::admit_ammunition(world, descriptor, weapon)?;
                    entry.child = Some(child);
                    child
                }
            };
            if let Some(error) =
                super::runtime_materials::equipment_admission::attachment_failure(world, child)
            {
                return Err(error);
            }
            super::runtime_materials::equipment_admission::request_attachment(
                world,
                child,
                descriptor.clone(),
            )?;
        }
        super::equipment_identity::poll_retirements(world)?;
        Ok(())
    })();
    world.insert_resource(owner);
    result
}

/// Shared material worker calls before applying its assets/command batch.
/// Full managed generations and immutable source identity reject stale work.
pub(super) fn validate(
    world: &World,
    child: EntityId,
    prepared: &super::player_arms::attachment::PreparedModel,
) -> Result<(), String> {
    let owner = world
        .get_resource::<Attachments>()
        .ok_or("ammunition owner was retired")?;
    let (&weapon, entry) = owner
        .live
        .iter()
        .find(|(_, entry)| entry.child == Some(child))
        .ok_or("prepared ammunition request was cancelled")?;
    if owner.closing {
        return Err("prepared ammunition belongs to a closing level".into());
    }
    let descriptor = entry
        .descriptor
        .as_ref()
        .ok_or("prepared ammunition source is absent")?;
    if !Arc::ptr_eq(descriptor, prepared.descriptor()) {
        return Err("prepared ammunition source generation changed".into());
    }
    entry.weapon.rig.tag(&entry.tag)?;
    let live = world
        .get_resource::<LiveEntities>()
        .ok_or("prepared ammunition managed owner is absent")?;
    if live.stage(child) != Some(Stage::Spawned) || live.parent(child)? != Some(weapon) {
        return Err("prepared ammunition identity/binding is stale".into());
    }
    if world
        .get_resource::<super::player_arms::PlayerArms>()
        .is_none_or(|arms| arms.attachments.has(child))
    {
        return Err("prepared ammunition renderer is absent or already published".into());
    }
    Ok(())
}

/// Called in the same exclusive publication after validate and the real image
/// gate. No extra acknowledgement or hidden replacement draw is generated.
pub(super) fn publish(
    world: &mut World,
    child: EntityId,
    prepared: super::player_arms::attachment::PreparedModel,
) -> Result<(), String> {
    validate(world, child, &prepared)?;
    let owner = world.resource::<Attachments>();
    let entry = owner
        .live
        .values()
        .find(|entry| entry.child == Some(child))
        .ok_or("ammunition request disappeared")?;
    let weapon = entry.weapon.clone();
    let tag = entry.tag.clone();
    let visible = entry.visible;
    world
        .resource_mut::<super::player_arms::PlayerArms>()
        .attachments
        .publish(child, weapon, &tag, prepared, visible)?;
    super::equipment_identity::mark_rendered(world, child)
}

pub(super) fn set_visible(
    world: &mut World,
    weapon: EntityId,
    visible: bool,
) -> Result<(), String> {
    let Some(mut owner) = world.get_resource_mut::<Attachments>() else {
        return Ok(());
    };
    let Some(entry) = owner.live.get_mut(&weapon) else {
        return Ok(());
    };
    entry.visible = visible;
    let child = entry.child;
    drop(owner);
    if let Some(child) = child {
        if let Some(mut arms) = world.get_resource_mut::<super::player_arms::PlayerArms>() {
            if arms.attachments.has(child) {
                arms.attachments.set_visible(child, visible)?;
            }
        }
    }
    Ok(())
}

pub(super) fn visible(world: &World, weapon: EntityId) -> bool {
    let Some(child) = world
        .get_resource::<Attachments>()
        .and_then(|owner| owner.live.get(&weapon))
        .and_then(|entry| entry.child)
    else {
        return false;
    };
    world
        .get_resource::<super::player_arms::PlayerArms>()
        .is_some_and(|arms| arms.attachments.visible(child))
}

/// Startup observes the complete selected-child path, including source IO
/// before it reaches the shared material worker. No animation completion is
/// required to enter gameplay.
pub(super) fn startup_pending(world: &World) -> bool {
    let Some(owner) = world.get_resource::<Attachments>() else {
        return false;
    };
    !owner.cancelled.is_empty()
        || owner.live.values().any(|entry| {
            entry.pending.is_some()
                || entry.child.is_none()
                || entry.child.is_some_and(|id| {
                    world
                        .get_resource::<super::player_arms::PlayerArms>()
                        .is_none_or(|arms| !arms.attachments.has(id))
                })
        })
}

pub(super) fn startup_error(world: &World) -> Option<String> {
    let owner = world.get_resource::<Attachments>()?;
    owner.live.values().find_map(|entry| {
        entry.failed.clone().or_else(|| {
            entry.child.and_then(|id| {
                super::runtime_materials::equipment_admission::attachment_failure(world, id)
            })
        })
    })
}

/// The caller passes the current player animation owner's actual tag frame.
/// This is also used before the Fire callback; a previous render frame or a
/// camera/weapon-origin fallback is never used as the loaded bolt's position.
pub(super) fn current_frame(
    world: &World,
    weapon: EntityId,
    tag_frame: Option<(Vec3, Mat3)>,
) -> Option<ReferenceFrame> {
    if !visible(world, weapon) {
        return None;
    }
    let (position, rotation) = tag_frame?;
    Some(ReferenceFrame {
        position: position.to_array(),
        basis: rotation.to_cols_array_2d(),
    })
}

fn withdraw(
    world: &mut World,
    entry: &mut Entry,
    cancelled: &mut Vec<Pending>,
) -> Result<(), String> {
    if let Some(pending) = entry.pending.take() {
        pending.cancelled.store(true, Ordering::Release);
        cancelled.push(pending);
    }
    if let Some(child) = entry.child {
        super::runtime_materials::equipment_admission::withdraw_attachment(world, child);
        let mut arms = world
            .remove_resource::<super::player_arms::PlayerArms>()
            .ok_or("ammunition retirement lost its renderer")?;
        let mut commands = super::setup_commands::SetupCommands::prepared(world);
        let receipt = arms.attachments.remove(child, &mut commands);
        if let Some(receipt) = &receipt {
            arms.retiring_equipment.push(receipt.clone());
        }
        commands.publish(world);
        world.insert_resource(arms);
        match receipt {
            Some(receipt) => {
                super::equipment_identity::await_render_retirement(world, child, receipt)?
            }
            None => {
                // This child never published any render model. Its cancelled
                // prepared batches remain with the shared material worker.
                super::equipment_identity::await_unrendered_retirement(world, child)?;
            }
        }
        entry.child = None;
    }
    entry.descriptor = None;
    Ok(())
}

pub(super) fn remove(world: &mut World, weapon: EntityId) -> Result<(), String> {
    let Some(mut owner) = world.remove_resource::<Attachments>() else {
        return Ok(());
    };
    let result = if let Some(mut entry) = owner.live.remove(&weapon) {
        let result = withdraw(world, &mut entry, &mut owner.cancelled);
        if result.is_err() {
            owner.live.insert(weapon, entry);
        }
        result
    } else {
        Ok(())
    };
    world.insert_resource(owner);
    result
}

pub(super) fn shutdown(world: &mut World) -> Result<bool, String> {
    let Some(mut owner) = world.remove_resource::<Attachments>() else {
        return Ok(true);
    };
    owner.closing = true;
    let result = (|| {
        let weapons = owner.live.keys().copied().collect::<Vec<_>>();
        for weapon in weapons {
            let entry = owner
                .live
                .get_mut(&weapon)
                .ok_or("ammunition shutdown entry disappeared")?;
            withdraw(world, entry, &mut owner.cancelled)?;
        }
        owner.live.clear();
        let mut index = 0;
        while index < owner.cancelled.len() {
            if completed(&mut owner.cancelled[index])?.is_some() {
                owner.cancelled.swap_remove(index);
            } else {
                index += 1;
            }
        }
        Ok(owner.cancelled.is_empty() && super::equipment_identity::poll_retirements(world)?)
    })();
    if !matches!(result, Ok(true)) {
        world.insert_resource(owner);
    }
    result
}
