//! Failed host preparation uses the same 7e72c0/7ea870 component owner.
//! The source completes Initialize synchronously; our fallible host boundary
//! must undo entered 808ea0/809d80 work before its geometry leases can drop.
use super::*;
use crate::gameplay::{entity_events::EntityEvents, entity_lifetime::LiveEntities};

/// PreparedLevel is the owner until publication, including cancellation after
/// successful PhysicsScene startup. No render/audio/game callback has run yet.
pub(in crate::app) fn discard(prepared: &mut crate::app::loading_screen::PreparedLevel) -> Result<(), String> {
    let Some(mut scene) = prepared.take_resource::<PhysicsScene>() else { return Ok(()); };
    let Some(mut live) = prepared.take_resource::<LiveEntities>() else {
        prepared.insert_resource(scene);
        return Err("unpublished physics cancellation has no managed entity owner".into());
    };
    let Some(mut events) = prepared.take_resource::<EntityEvents>() else {
        prepared.insert_resource(live);
        prepared.insert_resource(scene);
        return Err("unpublished physics cancellation has no entity event owner".into());
    };
    let result = rollback(&mut scene, &mut live, &mut events);
    // Even on failure the existing PreparedLevel remains the sole owner until
    // its Drop reports the incomplete phase and performs normal Rust cleanup.
    prepared.insert_resource(events);
    prepared.insert_resource(live);
    prepared.insert_resource(scene);
    result
}

pub(super) fn rollback(scene: &mut PhysicsScene, live: &mut LiveEntities,
    entity_events: &mut EntityEvents) -> Result<(), String> {
    let mut registry = scene.registry.take().ok_or("preparation rollback registry absent")?;
    let result: Result<(), String> = (|| {
        // Start and creation callbacks borrow the real component pool. Split
        // their cursors around that borrow, never move a component out of it.
        let mut handles: Vec<_> = scene.entities.iter().flat_map(|entity| entity.components.entries.values().iter().copied())
            .map(ComponentHandle::from_packed).collect::<Result<_, _>>()?;
        // A suspended creation/Start owns the world's publication cursor.
        // Unwind that component before touching previously completed bodies.
        let creation = scene.world.read().map_err(|_| "level physics query owner is poisoned")?
            .storage.disabled_component_creation_owner().map(|(handle, _)| handle);
        if let Some(entered) = creation.or(registry.interrupted_startup_component()?) {
            if let Some(index) = handles.iter().position(|handle| *handle == entered) { handles.swap(0, index); }
            else { return Err("entered physics startup component is not retained by its scene".into()); }
        }
        for handle in handles {
            if scene.is_rigid_class(handle.class_id) {
                rollback_rigid(scene, &mut registry, entity_events, handle)
                    .map_err(|error| format!("RigidBody {handle:?} preparation abort: {error}"))?;
            }
        }
        Ok(())
    })();
    scene.registry = Some(registry);
    result?;
    // No Engine emitter or interaction action exists until publication. The
    // canonical host therefore has no external consumer to acknowledge here.
    if !lifetime::poll_owned(None, scene, live, entity_events)? {
        return Err("unpublished component drain unexpectedly awaits a consumer".into());
    }
    Ok(())
}

fn rollback_rigid(scene: &mut PhysicsScene, registry: &mut ComponentRegistry,
    entity_events: &mut EntityEvents, handle: ComponentHandle) -> Result<(), String> {
    let shared = scene.world.clone();
    let mut guard = shared.write().map_err(|_| "level physics query owner is poisoned")?;
    let world = &mut *guard;
    loop {
        let request = rigid_pool_mut!(scene, handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent")?.unpublished_start_request();
        let Some((frame, activation, step)) = request else { break; };
        let body = scene.rigid_component(handle)?.body().ok_or("rigid body absent")?;
        if step < 4 {
            let behavior = if step == 2 {
                Some(scene.rigid_component(handle)?
                    .start_behavior.ok_or("entered rigid Start behavior absent")?)
            } else { None };
            scene_publication_events!(scene, registry, events);
            match step {
                0 => { world.storage.set_body_transform(&mut world.geometry, &mut events, body, frame, 0)?; }
                1 => world.storage.add_dynamic_body_resumable(&mut world.geometry, &mut events, body, activation)?,
                2 => {
                    // Start can fail while its static/keyframe transition owns
                    // a resumable cursor. Finish that same authored transition
                    // before asking the world to remove the body.
                    match behavior.ok_or("rigid behavior absent")? {
                        0 => world.storage.static_single_body(&mut world.geometry, &mut events, body)?,
                        1 => world.storage.keyframe_single_body(&mut world.geometry, &mut events, body)?,
                        // 727a00 returns on equal dynamic types. Preserve that
                        // same motion while unwinding an entered sleeping Start.
                        2 => {},
                        _ => return Err("rigid behavior is invalid".into()),
                    }
                },
                3 => world.storage.unpublish_bodies(&mut world.geometry, &mut events, &[body], 0)?,
                _ => unreachable!(),
            }
        } else {
            let component = rigid_pool_mut!(scene, handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent")?;
            component.unregister_start_listeners(entity_events, false, || Err("unadmitted wind registration".into()))?;
            component.unregister_activation_listener(&mut scene.body_activation_events)?;
            let movement = component.movement_body()?;
            if let Some(actor) = scene.actors.bodies.get(movement).ok_or("rigid movement absent")?.actor {
                scene.actors.detach_body(actor, movement)?;
            }
            component.clear_body_update_pause();
        }
        rigid_pool_mut!(scene, handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent")?.advance_unpublished_start(step)?;
    }
    // Attachment precedes the Start cursor in 809d80; a failure there
    // still has to detach the actual interface before body destruction.
    {
        let component = rigid_pool_mut!(scene, handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent")?;
        component.unregister_start_listeners(entity_events, false, || Err("unadmitted wind registration".into()))?;
        component.unregister_activation_listener(&mut scene.body_activation_events)?;
        let movement = component.movement_body()?;
        if let Some(actor) = scene.actors.bodies.get(movement).ok_or("rigid movement absent")?.actor {
            scene.actors.detach_body(actor, movement)?;
        }
        component.clear_body_update_pause();
    }
    loop {
        let step = rigid_pool_mut!(scene, handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent")?.unpublished_abort_step()?;
        match step {
            0 => {}, // Prepared immutable source leases have no world registration.
            1 => {
                let generation = scene.owner(handle)?.generation;
                scene_publication_events!(scene, registry, events);
                world.storage.abort_disabled_component_body(&mut world.geometry, &mut events, handle, generation)?;
            },
            2 => rigid_pool_mut!(scene, handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent")?
                .unregister_initialize_listeners(entity_events, false)?,
            3 => {
                let body = scene.rigid_component(handle)?.body().ok_or("rigid body absent")?;
                scene_publication_events!(scene, registry, events);
                world.storage.destroy_bodies(&mut world.geometry, &mut events, &[body], 0)?;
            },
            4 => {
                let shape = scene.rigid_component(handle)?.retained_shape().ok_or("rigid shape absent")?;
                world.geometry.retire_standalone_shape(shape)?;
            },
            5 => break,
            _ => return Err("invalid rigid creation rollback step".into()),
        }
        rigid_pool_mut!(scene, handle.class_id)?.get_mut(handle.slot).ok_or("rigid component slot is absent")?.advance_unpublished_abort(step)?;
    }
    Ok(())
}
