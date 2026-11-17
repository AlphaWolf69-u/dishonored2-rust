//! Retirement of the concrete component families hosted by PhysicsScene.
//! 7e72c0/7ea870 drain the real component registry before managed identities
//! retire. This adapter does not instantiate unsupported component families.
use super::*;
use crate::gameplay::{
    component_state::{
        lifecycle::{ComponentReleaseWorld, ReleaseStatus},
        updates::ComponentUpdateWorld,
    },
    entity_events::EntityEvents,
    entity_lifetime::LiveEntities,
};

#[derive(Default)]
pub(in crate::app) struct Retirement {
    components: Vec<(ComponentHandle, u32)>,
    snapshot_ready: bool,
    components_drained: bool,
    requested: usize,
    complete: bool,
    retire_next: usize,
    stopped: std::collections::HashSet<(u64, u32)>,
    /// Host: source pool addresses survive the component drain. Retain the
    /// same typed slots until entity destructors remove reciprocal bindings.
    pub(super) retained_actors: Vec<(ComponentHandle, u32)>,
    reclaimed_actors: usize,
    actor_owners: HashMap<(u64, u32), usize>,
}

/// Host projection of the source actor's embedded BasicBody lifetime. Its
///slot is returned with the actual actor, never retained once per projectile.
fn reclaim_reference_actor(
    scene: &mut PhysicsScene,
    registry: &ComponentRegistry,
    handle: ComponentHandle,
    generation: u32,
) -> Result<(), String> {
    registry.ensure_no_update_references(handle)?;
    let actor = scene
        .actors
        .actors
        .get(handle.slot)
        .ok_or("retiring reference actor absent")?;
    if actor.owner.generation != generation
        || actor.owner.entity.is_some()
        || actor.parent.is_some()
        || actor.binding.entity.is_some()
        || !actor.children.is_empty()
        || actor.current_body != actor.default_body
    {
        return Err("retiring reference actor retains dependencies".into());
    }
    let body = actor.default_body;
    let embedded = scene
        .actors
        .bodies
        .get(body)
        .ok_or("reference actor embedded body absent")?;
    if embedded
        .actor
        .is_some_and(|index| index != usize::from(handle.slot))
        || scene.free_rigid_movement.contains(&body)
        || scene
            .actors
            .actors
            .iter()
            .filter(|actor| actor.default_body == body)
            .count()
            != 1
        || scene
            .actors
            .actors
            .iter()
            .any(|actor| actor.default_body != body && actor.current_body == body)
    {
        return Err("reference actor embedded body still has another owner".into());
    }
    scene
        .free_rigid_movement
        .try_reserve(1)
        .map_err(|error| format!("reference actor embedded body retirement: {error}"))?;
    scene.actors.actors.release(handle.slot)?;
    scene.actors.bodies[body].actor = None;
    scene.actors.bodies[body].body_id = 0x00ff_ffff;
    scene.free_rigid_movement.push(body);
    Ok(())
}

/// Complete only an already accepted normal7ea870 batch. New entity
///destructors cannot take its registry cursor or replay its completed Stops.
pub(super) fn finish_gameplay_drain(
    world: &mut bevy::prelude::World,
    scene: &mut PhysicsScene,
    live: &mut LiveEntities,
    events: &mut EntityEvents,
) -> Result<bool, String> {
    let Some(mut state) = scene.gameplay_component_drain.take() else {
        return Ok(true);
    };
    let result = poll_requested_components(world, scene, live, events, &mut state);
    state.components_drained = matches!(result, Ok(true));
    if matches!(result, Ok(true)) {
        state.stopped.clear();
        let retained = &state.retained_actors;
        state.actor_owners.retain(|(packed, generation), _| {
            retained
                .iter()
                .any(|(handle, old)| handle.packed() == *packed && old == generation)
        });
    }
    if !matches!(result, Ok(true)) || !state.retained_actors.is_empty() {
        scene.gameplay_component_drain = Some(state);
    }
    result
}
fn transfer_gameplay_actors(
    scene: &mut PhysicsScene,
    target: &mut Retirement,
    entity: Option<usize>,
) -> Result<(), String> {
    let Some(source) = scene.gameplay_component_drain.as_ref() else {
        return Ok(());
    };
    let count = source
        .retained_actors
        .iter()
        .filter(|(handle, generation)| {
            entity.is_none_or(|entity| {
                source.actor_owners.get(&(handle.packed(), *generation)) == Some(&entity)
            })
        })
        .count();
    target
        .retained_actors
        .try_reserve(count)
        .map_err(|error| format!("component drain actor transfer: {error}"))?;
    target
        .actor_owners
        .try_reserve(count)
        .map_err(|error| format!("component drain actor identities: {error}"))?;
    // Validate before moving the sole receipt owner out of Scene.
    if source.retained_actors.iter().any(|(handle, generation)| {
        !source
            .actor_owners
            .contains_key(&(handle.packed(), *generation))
    }) {
        return Err("retained actor has no checked source entity".into());
    }
    let mut source = scene
        .gameplay_component_drain
        .take()
        .ok_or("component drain receipt owner disappeared")?;
    let mut index = 0;
    while index < source.retained_actors.len() {
        let (handle, generation) = source.retained_actors[index];
        let identity = (handle.packed(), generation);
        if entity.is_some_and(|entity| source.actor_owners.get(&identity) != Some(&entity)) {
            index += 1;
            continue;
        }
        let owner = source
            .actor_owners
            .remove(&identity)
            .expect("validated source actor receipt");
        target.retained_actors.push((handle, generation));
        target.actor_owners.insert(identity, owner);
        source.retained_actors.swap_remove(index);
    }
    if !source.retained_actors.is_empty() {
        scene.gameplay_component_drain = Some(source);
    }
    Ok(())
}
pub(in crate::app) fn prepare_entity_removal(
    world: &mut bevy::prelude::World,
    scene: &mut PhysicsScene,
    live: &mut LiveEntities,
    events: &mut EntityEvents,
    state: &mut Retirement,
) -> Result<bool, String> {
    if !scene
        .simulation
        .as_ref()
        .is_some_and(|driver| driver.is_idle())
    {
        return Ok(false);
    }
    if !finish_gameplay_drain(world, scene, live, events)? {
        return Ok(false);
    }
    let _ = state; // Actor receipts remain with Scene until their own destructor.
    Ok(true)
}
fn common_drain_work(registry: &ComponentRegistry) -> bool {
    registry.pending_stop.count() != 0
        || registry.pending_release.count() != 0
        || registry.pending_update_remove.count() != 0
        || registry.pending_update_add.count() != 0
        || registry.release_events_pending
        || !registry.release_idle()
}
/// Normal component Stops (including Breakable's stopped owner body) use
///the same registry/Host as destruction, without beginning map close.
pub(in crate::app) fn advance(world: &mut bevy::prelude::World) -> Result<(), String> {
    // Host: idle frames inspect the source queue counts without six World
    //resource removals/reinsertions. Pending acknowledgements still own state.
    let Some(scene) = world.get_resource::<PhysicsScene>() else {
        return Ok(());
    };
    if scene.retirement.is_some()
        || !scene
            .simulation
            .as_ref()
            .is_some_and(|driver| driver.is_idle())
        || world
            .get_resource::<LiveEntities>()
            .is_some_and(|live| live.active_removal_pending())
    {
        return Ok(());
    }
    let registry = scene
        .registry
        .as_ref()
        .ok_or("component drain registry borrowed")?;
    if registry.scoped_release.is_some() || registry.scoped_release_running {
        return Ok(());
    }
    if !common_drain_work(registry)
        && scene
            .gameplay_component_drain
            .as_ref()
            .is_none_or(|state| state.components_drained)
    {
        return Ok(());
    }
    let Some(mut scene) = world.remove_resource::<PhysicsScene>() else {
        return Ok(());
    };
    let Some(mut live) = world.remove_resource::<LiveEntities>() else {
        world.insert_resource(scene);
        return Err("component drain managed owner absent".into());
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("component drain events owner absent".into());
    };
    let result = (|| {
        // Another actual accepted source batch owns these callbacks.
        if !scene
            .simulation
            .as_ref()
            .is_some_and(|driver| driver.is_idle())
        {
            return Ok(());
        }
        if scene.retirement.is_some()
            || live.active_removal_pending()
            || scene.registry.as_ref().is_some_and(|registry| {
                registry.scoped_release.is_some() || registry.scoped_release_running
            })
        {
            return Ok(());
        }
        if scene.gameplay_component_drain.is_none() {
            let registry = scene
                .registry
                .as_ref()
                .ok_or("component drain registry borrowed")?;
            if !common_drain_work(registry) {
                return Ok(());
            }
            let mut state = Retirement::default();
            state
                .stopped
                .try_reserve(registry.pending_stop.count() as usize)
                .map_err(|error| format!("normal Stop receipt batch: {error}"))?;
            scene.gameplay_component_drain = Some(state);
        }
        finish_gameplay_drain(world, &mut scene, &mut live, &mut events)?;
        Ok(())
    })();
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    result
}

pub(in crate::app) fn retirement_progress(
    world: &bevy::prelude::World,
) -> Option<(usize, usize, usize, bool)> {
    world
        .get_resource::<PhysicsScene>()?
        .retirement
        .as_ref()
        .map(|state| {
            (
                state.requested,
                state.stopped.len(),
                state.retire_next,
                state.complete,
            )
        })
}

/// Removal from World is temporary; every error and Pending result restores
/// these sole owners. No resource destruction follows an incomplete drain.
pub(in crate::app) fn poll(world: &mut bevy::prelude::World) -> Result<bool, String> {
    let Some(mut scene) = world.remove_resource::<PhysicsScene>() else {
        return Ok(true);
    };
    let Some(mut live) = world.remove_resource::<LiveEntities>() else {
        world.insert_resource(scene);
        return Err("physics retirement has no managed entity owner".into());
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("physics retirement has no entity event owner".into());
    };
    let result = poll_owned(Some(&mut *world), &mut scene, &mut live, &mut events);
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    if matches!(result, Ok(false)) {
        super::super::audio::poll_object_retirements(world)?;
    }
    result
}

pub(super) fn poll_owned(
    mut world: Option<&mut bevy::prelude::World>,
    scene: &mut PhysicsScene,
    live: &mut LiveEntities,
    events: &mut EntityEvents,
) -> Result<bool, String> {
    if scene
        .retirement
        .as_ref()
        .is_some_and(|state| state.complete)
    {
        return Ok(true);
    }
    if !scene
        .simulation
        .as_ref()
        .is_some_and(|driver| driver.is_idle())
    {
        return Ok(false);
    }
    if let Some(world) = world.as_deref_mut() {
        if !finish_gameplay_drain(world, scene, live, events)? {
            return Ok(false);
        }
    } else if scene.gameplay_component_drain.is_some() {
        return Err("unpublished scene has accepted gameplay component drain".into());
    }
    if scene.retirement.is_none() {
        let mut components = Vec::new();
        let count = scene
            .entities
            .iter()
            .map(|entity| entity.components.entries.values().len())
            .sum();
        components
            .try_reserve_exact(count)
            .map_err(|error| format!("physics retirement snapshot: {error}"))?;
        for entity in &scene.entities {
            if entity.retired {
                continue;
            }
            if live.get(entity.shared).is_none() {
                return Err("physics retirement entity is stale".into());
            }
            for &packed in entity.components.entries.values().iter().rev() {
                let handle = ComponentHandle::from_packed(packed)?;
                components.push((handle, scene.owner(handle)?.generation));
            }
        }
        let mut stopped = std::collections::HashSet::new();
        stopped
            .try_reserve(components.len())
            .map_err(|error| format!("physics Stop completion snapshot: {error}"))?;
        scene.retirement = Some(Retirement {
            components,
            stopped,
            ..Default::default()
        });
    }
    let mut retirement = scene
        .retirement
        .take()
        .ok_or("world retirement cursor absent")?;
    let transferred = transfer_gameplay_actors(scene, &mut retirement, None);
    scene.retirement = Some(retirement);
    transferred?;
    let mut registry = scene
        .registry
        .take()
        .ok_or("physics lifecycle registry absent")?;
    let result = (|| {
        let mut host = Host {
            world,
            scene: &mut *scene,
            events: &mut *events,
            live: &mut *live,
            scoped: None,
        };
        loop {
            let state = host
                .scene
                .retirement
                .as_ref()
                .ok_or("physics retirement progress absent")?;
            let Some(&(handle, generation)) = state.components.get(state.requested) else {
                break;
            };
            if host.owner(handle)?.generation != generation {
                return Err("physics retirement component is stale".into());
            }
            registry.request_release(handle, &mut host)?;
            host.scene.retirement.as_mut().unwrap().requested += 1;
        }
        if registry.poll_stop_and_release(&mut host)? == ReleaseStatus::Pending {
            return Ok(false);
        }
        // Stop callbacks may enqueue update membership changes. The slots must
        // remain allocated until those exact references have been drained.
        registry.drain_update_changes(&mut host)?;
        if host.events.normal_pending(){host.pump_events(&mut registry)?;}
        // Deinit still runs reflected Remove through the canonical deadline
        // owner. Yield to LevelLifetime's real pump before graph/ID metadata
        // disappears; do not copy its queue or advance its frozen clock.
        if let Some(world)=host.world.as_deref(){
            if super::super::entity_functions::due_pending(world)?{return Ok(false);}
        }
        if host
            .scene
            .entities
            .iter()
            .any(|entity| !entity.components.entries.values().is_empty())
            || !host.scene.declarations.is_empty()
        {
            return Err("physics component drain retained linked owners".into());
        }
        {
            let query_world = host
                .scene
                .world
                .read()
                .map_err(|_| "level physics query owner is poisoned")?;
            // Body zero is the canonical static anchor, not a component body.
            if !query_world.storage.publication_idle()
                || query_world.storage.bodies.active_count() != 1
            {
                return Err("physics world still owns unfinished body retirement".into());
            }
        }
        while host.scene.retirement.as_ref().unwrap().retire_next < host.scene.entities.len() {
            let index = host.scene.retirement.as_ref().unwrap().retire_next;
            let entity = &host.scene.entities[index];
            if entity.retired {
                host.scene.retirement.as_mut().unwrap().retire_next += 1;
                continue;
            }
            if let Some(world) = host.world.as_deref_mut() {
                super::super::entity_graphs::retire(world, entity.shared)?;
            }
            host.scene.destroy_document_attachments(host.live, index)?;
            let entity = &host.scene.entities[index];
            if !entity.external {
                if let Some(world) = host.world.as_deref_mut() {
                    if let Some(mut clock) = world
                        .get_resource_mut::<crate::gameplay::game_clock_runtime::ClockRuntime>()
                    {
                        super::super::time_sync::retire_entity_time_with_owners(
                            host.scene, host.live, &mut clock, entity.shared,
                        )?;
                    }
                }
            }
            if !entity.external && host.live.retire_for_world_shutdown(entity.shared).is_none() {
                return Err("physics managed entity remains attached during retirement".into());
            }
            host.scene.retirement.as_mut().unwrap().retire_next += 1;
        }
        loop {
            let state = host
                .scene
                .retirement
                .as_ref()
                .ok_or("actor retirement cursor absent")?;
            let Some(&(handle, generation)) = state.retained_actors.get(state.reclaimed_actors)
            else {
                break;
            };
            registry.ensure_no_update_references(handle)?;
            let actor = host
                .scene
                .actors
                .actors
                .get(handle.slot)
                .ok_or("retained actor slot absent")?;
            if actor.owner.generation != generation
                || actor.owner.entity.is_some()
                || actor.parent.is_some()
                || actor.binding.entity.is_some()
                || !actor.children.is_empty()
                || actor.current_body != actor.default_body
            {
                return Err("retained actor still owns attachment dependencies".into());
            }
            reclaim_reference_actor(host.scene, &registry, handle, generation)?;
            host.scene.retirement.as_mut().unwrap().reclaimed_actors += 1;
        }
        host.events.clear_for_world_shutdown();
        if host
            .scene
            .faction_manager
            .as_ref()
            .is_some_and(|manager| manager.manager().has_members())
        {
            return Err("Faction manager still owns live component memberships".into());
        }
        host.scene.faction_manager = None;
        host.scene.shared_indices.clear();
        host.scene.finish_document_hierarchy_retirement()?;
        host.scene.retirement.as_mut().unwrap().complete = true;
        Ok(true)
    })();
    scene.registry = Some(registry);
    result
}

/// 7eac80/7eb060: the ordinary entity destructor drains only this actual
/// component set. The caller retains this cursor across asynchronous Stop.
pub(super) fn poll_entity_components(
    world: &mut bevy::prelude::World,
    scene: &mut PhysicsScene,
    live: &mut LiveEntities,
    events: &mut EntityEvents,
    entity: usize,
    state: &mut Retirement,
) -> Result<bool, String> {
    if state.complete {
        return Ok(true);
    }
    if !scene
        .simulation
        .as_ref()
        .is_some_and(|driver| driver.is_idle())
    {
        return Ok(false);
    }
    if !finish_gameplay_drain(world, scene, live, events)? {
        return Ok(false);
    }
    transfer_gameplay_actors(scene, state, Some(entity))?;
    if !state.snapshot_ready {
        let owner = scene
            .entities
            .get(entity)
            .ok_or("retiring entity header absent")?;
        let mut components = Vec::new();
        components
            .try_reserve_exact(owner.components.entries.values().len())
            .map_err(|error| format!("entity component retirement snapshot: {error}"))?;
        for &packed in owner.components.entries.values() {
            let handle = ComponentHandle::from_packed(packed)?;
            components.push((handle, scene.owner(handle)?.generation));
        }
        state
            .stopped
            .try_reserve(components.len())
            .map_err(|error| format!("entity Stop completion snapshot: {error}"))?;
        state.components = components;
        state.snapshot_ready = true;
    }
    let mut registry = scene
        .registry
        .take()
        .ok_or("entity component registry is borrowed")?;
    // Registry owns its resumable queue-isolation cursor; Host owns the same
    // checked Stop receipts and reference actors as whole-world teardown.
    let targets = std::mem::take(&mut state.components);
    let result = (|| {
        let mut host = Host {
            world: Some(&mut *world),
            scene: &mut *scene,
            events: &mut *events,
            live: &mut *live,
            scoped: Some(&mut *state),
        };
        if !host.retirement()?.components_drained {
            if registry.poll_release_entity_scoped(entity, &targets, &mut host)?
                == ReleaseStatus::Pending
            {
                return Ok(false);
            }
            registry.drain_update_changes(&mut host)?;
            host.retirement_mut()?.components_drained = true;
        }
        if !host.scene.entities[entity]
            .components
            .entries
            .values()
            .is_empty()
        {
            return Err("retiring entity retains linked components".into());
        }
        host.scene.destroy_document_attachments(host.live, entity)?;
        loop {
            let cursor = host.retirement()?;
            let Some(&(handle, generation)) = cursor.retained_actors.get(cursor.reclaimed_actors)
            else {
                break;
            };
            registry.ensure_no_update_references(handle)?;
            let actor = host
                .scene
                .actors
                .actors
                .get(handle.slot)
                .ok_or("retained entity actor absent")?;
            if actor.owner.generation != generation
                || actor.owner.entity.is_some()
                || actor.parent.is_some()
                || actor.binding.entity.is_some()
                || !actor.children.is_empty()
                || actor.current_body != actor.default_body
            {
                return Err("retained entity actor still owns attachment dependencies".into());
            }
            reclaim_reference_actor(host.scene, &registry, handle, generation)?;
            host.retirement_mut()?.reclaimed_actors += 1;
        }
        host.retirement_mut()?.complete = true;
        Ok(true)
    })();
    scene.registry = Some(registry);
    state.components = targets;
    result
}

///8ab380/7e72c0 collect all snapshot requests before the shared7ea870
///drain. The batch retains this Host cursor, including immediate destruction.
pub(in crate::app) fn request_component_release(
    world: &mut bevy::prelude::World,
    scene: &mut PhysicsScene,
    live: &mut LiveEntities,
    events: &mut EntityEvents,
    handle: ComponentHandle,
    generation: u32,
    state: &mut Retirement,
) -> Result<(), String> {
    if scene.owner(handle)?.generation != generation {
        return Err("queued entity component generation is stale".into());
    }
    state
        .stopped
        .try_reserve(1)
        .map_err(|error| format!("batch Stop receipts: {error}"))?;
    let mut registry = scene
        .registry
        .take()
        .ok_or("entity release registry borrowed")?;
    let result = registry.request_release(
        handle,
        &mut Host {
            world: Some(world),
            scene: &mut *scene,
            live,
            events,
            scoped: Some(state),
        },
    );
    scene.registry = Some(registry);
    result
}
pub(in crate::app) fn poll_requested_components(
    world: &mut bevy::prelude::World,
    scene: &mut PhysicsScene,
    live: &mut LiveEntities,
    events: &mut EntityEvents,
    state: &mut Retirement,
) -> Result<bool, String> {
    let mut registry = scene
        .registry
        .take()
        .ok_or("entity release registry borrowed")?;
    let result = (|| {
        let mut host = Host {
            world: Some(world),
            scene: &mut *scene,
            live,
            events,
            scoped: Some(state),
        };
        if registry.poll_stop_and_release(&mut host)? == ReleaseStatus::Pending {
            return Ok(false);
        }
        registry.drain_update_changes(&mut host)?;
        Ok(true)
    })();
    scene.registry = Some(registry);
    result
}
/// After the batch's shared drain, each actual entity destructor removes its
///reciprocal attachments before its retained reference actor can be recycled.
pub(in crate::app) fn finish_requested_entity(
    scene: &mut PhysicsScene,
    live: &mut LiveEntities,
    entity: usize,
    state: &mut Retirement,
) -> Result<(), String> {
    transfer_gameplay_actors(scene, state, Some(entity))?;
    if !scene
        .entities
        .get(entity)
        .ok_or("released entity header absent")?
        .components
        .entries
        .values()
        .is_empty()
    {
        return Err("released entity retains linked components".into());
    }
    scene.destroy_document_attachments(live, entity)?;
    let mut index = 0;
    while index < state.retained_actors.len() {
        let (handle, generation) = state.retained_actors[index];
        let identity = (handle.packed(), generation);
        if state.actor_owners.get(&identity) != Some(&entity) {
            index += 1;
            continue;
        }
        scene
            .registry
            .as_ref()
            .ok_or("entity release registry borrowed")?
            .ensure_no_update_references(handle)?;
        let actor = scene
            .actors
            .actors
            .get(handle.slot)
            .ok_or("released entity reference actor absent")?;
        if actor.owner.generation != generation
            || actor.owner.entity.is_some()
            || actor.parent.is_some()
            || actor.binding.entity.is_some()
            || !actor.children.is_empty()
            || actor.current_body != actor.default_body
        {
            return Err("released entity reference actor retains attachment dependencies".into());
        }
        // End the registry borrow before mutating the actual scene pools.
        let registry = scene
            .registry
            .take()
            .ok_or("entity release registry borrowed")?;
        let reclaimed = reclaim_reference_actor(scene, &registry, handle, generation);
        scene.registry = Some(registry);
        reclaimed?;
        state.retained_actors.swap_remove(index);
        state.actor_owners.remove(&identity);
    }
    Ok(())
}

struct Host<'a> {
    world: Option<&'a mut bevy::prelude::World>,
    scene: &'a mut PhysicsScene,
    events: &'a mut EntityEvents,
    /// Same identity owner supplied to published and preparation rollback.
    live: &'a mut LiveEntities,
    scoped: Option<&'a mut Retirement>,
}

impl Host<'_> {
    fn audio_objects_retired(&mut self, handle: ComponentHandle) -> Result<bool, String> {
        if handle.class_id != self.scene.classes[4] { return Ok(true); }
        let Some(world) = self.world.as_deref_mut() else { return Ok(true); };
        let component = self.scene.audios.get_mut(handle.slot)
            .ok_or("audio retiring component absent")?;
        let generation = component.owner.generation;
        super::super::audio::drain_component_posts(world, handle, generation, component)?;
        super::super::audio::retire_component_joints(world, handle.packed(), generation, component)?;
        super::super::audio::retire_component_object(world,
            crate::gameplay::sound::Emitter::Component { handle: handle.packed(), generation })
    }

    fn retire_contacts(
        &mut self,
        handle: ComponentHandle,
        include_world: bool,
    ) -> Result<(), String> {
        let mut bodies = Vec::new();
        if self.scene.is_rigid_class(handle.class_id) {
            bodies.extend(self.scene.rigid_component(handle)?.body());
        } else if handle.class_id == self.scene.classes[0] {
            let movement = self
                .scene
                .triggers
                .get(handle.slot)
                .ok_or("retiring Trigger payload absent")?
                .movement_body;
            let body = self
                .scene
                .actors
                .bodies
                .get(movement)
                .ok_or("retiring Trigger movement body absent")?
                .body_id;
            if body & 0x00ff_ffff != 0x00ff_ffff {
                bodies.push(body);
            }
        } else if include_world && handle.class_id == self.scene.classes[14] {
            bodies.extend_from_slice(
                &self
                    .scene
                    .level_bodies
                    .get(handle.slot)
                    .ok_or("retiring PhysicsWorld payload absent")?
                    .bodies,
            );
        }
        if !bodies.is_empty() {
            self.scene.retire_body_contacts(
                self.world.as_deref_mut(),
                self.live,
                self.events,
                &bodies,
            )?;
        }
        Ok(())
    }
    fn retirement(&self) -> Result<&Retirement, String> {
        self.scoped
            .as_deref()
            .or(self.scene.retirement.as_ref())
            .ok_or_else(|| "component retirement cursor absent".into())
    }
    fn retirement_mut(&mut self) -> Result<&mut Retirement, String> {
        match self.scoped.as_deref_mut() {
            Some(state) => Ok(state),
            None => self
                .scene
                .retirement
                .as_mut()
                .ok_or_else(|| "component retirement cursor absent".into()),
        }
    }
}

impl ComponentLifecycleWorld for Host<'_> {
    fn priority(&self, handle: ComponentHandle) -> Result<i32, String> {
        self.scene.priority(handle)
    }
    fn owner(&self, handle: ComponentHandle) -> Result<&ComponentOwner, String> {
        self.scene.owner(handle)
    }
    fn owner_mut(&mut self, handle: ComponentHandle) -> Result<&mut ComponentOwner, String> {
        self.scene.owner_mut(handle)
    }
    fn initialize(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        if self.scene.is_flash_class(handle.class_id) {
            return self.scene.initialize_flash_component(self.live,self.events,handle);
        }
        if self.scene.is_smart_object_class(handle.class_id) {
            return self.scene.initialize_smart_object(self.live,self.events,handle);
        }
        if handle.class_id==self.scene.classes[48]||handle.class_id==self.scene.classes[49] {
            return self.scene.initialize_cue_component(self.live,handle);
        }
        if self.scene.is_ai_startup_class(handle.class_id){return self.scene.initialize_ai_startup(self.world.as_deref_mut().ok_or("AI manager Initialize requires published world")?,self.live,self.events,registry,handle);}
        if self.scene.is_crew_class(handle.class_id){return self.scene.initialize_crew(handle);}
        if self.scene.is_crew_collector_class(handle.class_id){return self.scene.initialize_crew_collector(handle);}
        if self.scene.is_crew_module_class(handle.class_id){return self.scene.initialize_crew_module(handle);}
        if handle.class_id==self.scene.classes[21]{return self.scene.initialize_statistics(handle);}
        if handle.class_id==self.scene.classes[22]{return self.scene.initialize_timers(self.world.as_deref_mut(),self.live,handle);}
        if handle.class_id==self.scene.classes[20]{return self.scene.initialize_ai_danger(self.world.as_deref_mut().ok_or("AI danger Initialize requires published world")?,self.live,handle);}
        if handle.class_id==self.scene.classes[19]{return self.scene.initialize_grenade(handle,self.live);}
        if handle.class_id==self.scene.classes[18]{return self.scene.initialize_damage_zone(handle,self.live);}
        if handle.class_id==self.scene.classes[7]{return self.scene.initialize_loot(handle);}
        if handle.class_id==self.scene.classes[17]{
            return self.scene.initialize_sound_volume(
                self.world.as_deref_mut().ok_or("sound volume Initialize requires its published world")?,
                self.live,handle,
            );
        }
        self.scene.initialize(handle, registry)
    }
    fn start(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        if handle.class_id == self.scene.classes[4] {
            if let Some(world) = self.world.as_deref_mut() {
                super::super::audio::resume_component_object(world, crate::gameplay::sound::Emitter::Component {
                    handle: handle.packed(), generation: self.scene.audios.get(handle.slot)
                        .ok_or("audio Start component absent")?.owner.generation,
                })?;
            }
            self.scene.start(handle, registry)?;
            return self.scene.start_audio_events(self.live, self.events, handle);
        }
        if self.scene.is_smart_object_class(handle.class_id) {
            self.scene.start(handle,registry)?;
            return self.scene.start_smart_object(self.world.as_deref(),self.live,handle);
        }
        if self.scene.is_flash_class(handle.class_id) {
            self.scene.start(handle,registry)?;
            return self.scene.start_flash_component(handle);
        }
        if handle.class_id==self.scene.classes[48]||handle.class_id==self.scene.classes[49] {
            self.scene.start(handle,registry)?;
            return self.scene.start_cue_component(self.world.as_deref(),self.live,self.events,handle);
        }
        if self.scene.is_ai_startup_class(handle.class_id){self.scene.start(handle,registry)?;return self.scene.start_ai_startup(self.world.as_deref_mut().ok_or("AI manager Start requires published world")?,self.events,registry,handle);}
        if self.scene.is_crew_class(handle.class_id){self.scene.start(handle,registry)?;return self.scene.start_crew(self.events,handle);}
        if self.scene.is_crew_collector_class(handle.class_id){self.scene.start(handle,registry)?;return self.scene.start_crew_collector(self.events,registry,handle);}
        if self.scene.is_crew_module_class(handle.class_id){self.scene.start(handle,registry)?;return self.scene.start_crew_module(self.events,registry,handle);}
        if handle.class_id==self.scene.classes[21]{
            self.scene.start(handle,registry)?;return self.scene.start_statistics(self.events,handle);
        }
        if handle.class_id==self.scene.classes[22]{
            self.scene.start(handle,registry)?;return self.scene.start_timers(self.live,handle);
        }
        if handle.class_id==self.scene.classes[20]{
            self.scene.start(handle,registry)?;
            return self.scene.start_ai_danger(handle,self.events);
        }
        if handle.class_id==self.scene.classes[19]{
            self.scene.start(handle,registry)?;
            return self.scene.start_grenade(self.world.as_deref_mut(),self.live,self.events,handle);
        }
        if handle.class_id==self.scene.classes[5]{
            self.scene.start(handle,registry)?;
            if let Some(world)=self.world.as_deref_mut(){self.scene.start_usable_backend(world,handle)?;}
            return Ok(());
        }
        if handle.class_id==self.scene.classes[18]{
            self.scene.start(handle,registry)?;
            return self.scene.start_damage_zone(self.world.as_deref_mut().ok_or("DamageZone Start requires published world")?,self.live,handle);
        }
        if handle.class_id==self.scene.classes[7]{
            self.scene.start(handle,registry)?;
            return self.scene.start_loot(self.live,self.events,registry,handle);
        }
        if handle.class_id==self.scene.classes[17]{
            self.scene.start(handle,registry)?;
            self.scene.sound_volumes.get_mut(handle.slot).ok_or("sound volume Start payload absent")?.started = true;
            return Ok(());
        }
        self.scene.start(handle, registry)
    }
    fn stop_queued(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        self.scene.stop_queued(handle, registry)
    }
    fn initialization_failed(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        self.scene.initialization_failed(handle, registry)
    }
}

impl ComponentUpdateWorld for Host<'_> {
    fn update_remove(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        if self.scene.is_crew_module_class(handle.class_id){self.scene.update_remove_crew_module(handle,registry,self.live,self.events)?;}
        registry.base_update_remove(handle, self)?;
        self.scene.crew_update_subscription(handle,self.events,false)
    }
    fn update_add(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        registry.base_update_add(handle, self)?;
        self.scene.crew_update_subscription(handle,self.events,true)
    }
    fn set_update_scheduled(
        &mut self,
        handle: ComponentHandle,
        scheduled: bool,
    ) -> Result<(), String> {
        self.scene.set_component_update_scheduled(handle, scheduled)
    }
}

impl ComponentReleaseWorld for Host<'_> {
    fn pump_events(&mut self, registry: &mut ComponentRegistry) -> Result<(), String> {
        // Retirement drains the same numeric registry and actual callback
        // owner as normal gameplay, before retiring those component leases.
        let scene=&mut *self.scene;
        let live=&mut *self.live;
        let events=&mut *self.events;
        if let Some(world)=self.world.as_deref_mut() {
            let mut ai_worlds = world.remove_resource::<crate::gameplay::ai_world::Worlds>();
            let mut hearts=world.remove_resource::<super::super::heart::Hearts>();
            let mut statistics=world.remove_resource::<super::super::player_statistics::PlayerStatistics>();
            let result = if world.contains_resource::<super::super::player::PlayerState>() {
                world.resource_scope(|world,mut player:bevy::prelude::Mut<super::super::player::PlayerState>| {
                    scene.pump_entity_messages_with_registry(live,events,registry,
                        world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>(),player.player.as_mut(), ai_worlds.as_mut(),hearts.as_mut(),statistics.as_mut())
                })
            } else {
                scene.pump_entity_messages_with_registry(live,events,registry,
                    world.get_resource::<crate::gameplay::game_clock_runtime::ClockRuntime>(),None, ai_worlds.as_mut(),hearts.as_mut(),statistics.as_mut())
            };
            if let Some(owner) = ai_worlds { world.insert_resource(owner); }
            if let Some(owner)=hearts {world.insert_resource(owner);}
            if let Some(owner)=statistics {world.insert_resource(owner);}
            return result;
        }
        scene.pump_entity_messages_with_registry(live,events,registry,None,None,None,None,None)
    }

    fn stop(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        self.retire_contacts(handle, false)?;
        if handle.class_id==self.scene.classes[6] {
            if let Some(world)=self.world.as_deref_mut(){self.scene.heart_pickable_stopped(world,handle)?;}
            //76eaf0 withdraws the checked references at Stop, before owners
            //can retire or reuse their component generations.
            self.scene.pickables.get_mut(handle.slot).ok_or("Pickable Stop payload absent")?.references=None;
        }
        let published = self.world.is_some();
        let scene = &mut self.scene;
        if scene.is_rigid_class(handle.class_id) {
            if handle.class_id == scene.classes[12] {
                scene.stop_throwable_component(handle, self.events)?;
            }
            if !scene.rigid_component(handle)?.can_release() {
                scene.stop_placed_rigid(handle, registry, self.events)?;
            }
        } else if handle.class_id == scene.classes[0] {
            let declaration = scene
                .owner(handle)?
                .declaration
                .ok_or("trigger declaration absent")?;
            let shared = scene.world.clone();
            let mut query_owner = shared
                .write()
                .map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            let keyframed = world
                .geometry
                .placements
                .get(declaration)
                .and_then(Option::as_ref)
                .ok_or("trigger placement absent")?
                .binding
                .collision
                .keyframed;
            scene_publication_events!(scene, registry, events);
            crate::gameplay::trigger_body::startup::stop_trigger(
                &mut world.storage,
                &mut world.geometry,
                &mut events,
                handle,
                keyframed,
                |components, reference| {
                    let body = components.trigger(reference)?.movement_body;
                    let actor = components
                        .actors
                        .bodies
                        .get(body)
                        .ok_or("trigger movement interface absent")?
                        .actor;
                    match actor {
                        Some(actor) => components.actors.detach_body(actor, body),
                        None if !published => Ok(()), // Failed Start never completed attachment.
                        None => Err("trigger reference actor absent".into()),
                    }
                },
            )?;
            drop(query_owner);
        } else if handle.class_id == scene.classes[1] {
            scene.notify_water_contacts(handle, true)?;
            // 809880 can publish an exit during Stop itself, after the
            // frame's water-publication pass. Deliver it before this drain
            // destroys model/entity owners, including on world shutdown.
            if let Some(world) = self.world.as_deref_mut() {
                scene.drain_water_notifications(world, self.live, self.events)?;
            } else if !scene.water_notifications.is_empty() {
                return Err("water exit publication requires its model owner".into());
            }
        } else if handle.class_id == scene.classes[4] {
            let component = scene.audios.get_mut(handle.slot).ok_or("audio component absent")?;
            if let Some(world) = self.world.as_deref_mut() {
                super::super::audio::drain_component_posts(world, handle, component.owner.generation, component)?;
                super::super::audio::retire_component_joints(world, handle.packed(), component.owner.generation, component)?;
            } else if !component.posts.is_empty() {
                return Err("audio Stop requires the accepted animation post backend".into());
            }
            scene.stop_audio_events(self.events, handle)?;
        } else if handle.class_id == scene.classes[5] {
            let actor = scene
                .usables
                .get(handle.slot)
                .ok_or("usable component absent")?
                .actor;
            // Preparation has not published interactions or submitted game actions.
            let Some(world) = self.world.as_deref_mut() else {
                return registry.stop_update_ownership(handle, &mut **scene);
            };
            if !world.contains_resource::<super::super::interactions::Interactions>()
                || !world.contains_resource::<super::super::NavigationWorld>()
                || !world.contains_resource::<super::super::usable_animation::UsableAnimations>()
                || !world.contains_resource::<super::super::usable_effects::UsableEffects>()
            {
                return Err("usable Stop is waiting for its interaction backends".into());
            }
            world.resource_scope(|world, mut interactions: bevy::prelude::Mut<super::super::interactions::Interactions>| {
                world.resource_scope(|world, mut navigation: bevy::prelude::Mut<super::super::NavigationWorld>| {
                    world.resource_scope(|world, mut animations: bevy::prelude::Mut<super::super::usable_animation::UsableAnimations>| {
                        let mut effects = world.resource_mut::<super::super::usable_effects::UsableEffects>();
                        interactions.stop_actor(actor, &mut navigation.0, &mut animations, &mut effects);
                    });
                });
            });
        } else if handle.class_id == scene.classes[9] {
            scene
                .factions
                .get_mut(handle.slot)
                .ok_or("Faction component absent")?
                .stop(
                    scene
                        .faction_manager
                        .as_mut()
                        .ok_or("Faction manager absent")?
                        .manager_mut(),
                );
        } else if handle.class_id == scene.classes[11] {
            scene.stop_breakable(handle, self.events)?;
        } else if handle.class_id == scene.classes[13] {
            scene.stop_projectile(handle,self.events)?;
        } else if handle.class_id == scene.classes[15] {
            scene.stop_animation(handle)?;
        } else if handle.class_id == scene.classes[17] {
            scene.sound_volumes.get_mut(handle.slot).ok_or("sound volume Stop payload absent")?.started = false;
        } else if handle.class_id == scene.classes[19] {
            scene.stop_grenade(self.world.as_deref_mut(),self.live,self.events,handle)?;
        } else if handle.class_id == scene.classes[21] {
            scene.stop_statistics(self.events,handle)?;
        } else if handle.class_id == scene.classes[22] {
            scene.stop_timers(handle)?;
        } else if handle.class_id == scene.classes[20] {
            scene.stop_ai_danger(handle,self.events)?;
        } else if scene.is_smart_object_class(handle.class_id) {
            scene.stop_smart_object(self.world.as_deref(),self.live,handle)?;
        } else if scene.is_flash_class(handle.class_id) {
            scene.stop_flash_component(self.world.as_deref_mut(),self.events,handle)?;
        } else if handle.class_id==scene.classes[48]||handle.class_id==scene.classes[49] {
            scene.stop_cue_component(self.world.as_deref(),self.live,self.events,handle)?;
        } else if scene.is_ai_startup_class(handle.class_id) {
            scene.stop_ai_startup(self.events,handle)?;
        } else if scene.is_crew_class(handle.class_id) {
            scene.stop_crew(self.events,handle)?;
        } else if scene.is_crew_collector_class(handle.class_id) {
            scene.stop_crew_collector(self.events,handle)?;
        } else if scene.is_crew_module_class(handle.class_id) {
            scene.stop_crew_module(self.events,handle)?;
        } else if handle.class_id == scene.classes[18] {
            scene.stop_damage_zone(self.world.as_deref_mut(),handle)?;
        } else if scene.is_health_class(handle.class_id) {
            // Health Stop718240 clears the entity presence flag before
            // base7e7750; deinitialization must not leave a stopped receiver
            // advertised on its still-live entity in the meantime.
            let index = scene.owner(handle)?.entity.ok_or("health entity absent")?;
            scene.entities[index].entity_flags_d7 &= !2;
        }
        registry.stop_update_ownership(handle, self)
    }

    fn poll_stop(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
        _first: bool,
    ) -> Result<ReleaseStatus, String> {
        let identity = (handle.packed(), self.owner(handle)?.generation);
        if !self.retirement()?.stopped.contains(&identity) {
            self.retirement_mut()?
                .stopped
                .try_reserve(1)
                .map_err(|error| format!("component Stop completion receipt: {error}"))?;
            self.stop(handle, registry)?;
            // Capacity was reserved before any callback. Failed callbacks retry
            // against their real body cursor; completed Stop is never replayed.
            self.retirement_mut()?.stopped.insert(identity);
        }
        if handle.class_id == self.scene.classes[4] && self.world.is_some() {
            let generation = self.owner(handle)?.generation;
            {
                let component = self.scene.audios.get_mut(handle.slot).ok_or("audio component absent")?;
                super::super::audio::drain_component_posts(
                    self.world.as_deref_mut().ok_or("published audio owner absent")?,
                    handle, generation, component,
                )?;
                super::super::audio::retire_component_joints(
                    self.world.as_deref_mut().ok_or("published audio owner absent")?,
                    handle.packed(), generation, component,
                )?;
            }
            if !super::super::audio::retire_component_object(
                self.world
                    .as_deref_mut()
                    .ok_or("published audio owner absent")?,
                crate::gameplay::sound::Emitter::Component {
                    handle: handle.packed(),
                    generation,
                },
            )? {
                return Ok(ReleaseStatus::Pending);
            }
            let component=self.scene.audios.get_mut(handle.slot).ok_or("audio Stop component absent")?;
            component.allocated=false;
            for joint in &mut component.joints { joint.allocated=false; }
        }
        if self.scene.is_flash_class(handle.class_id)&&!self.scene.flash_stop_complete(self.world.as_deref_mut(),handle)? {
            return Ok(ReleaseStatus::Pending);
        }
        if handle.class_id==self.scene.classes[19]&&!self.scene.grenade_stop_complete(self.world.as_deref(),handle)?{
            return Ok(ReleaseStatus::Pending);
        }
        if handle.class_id==self.scene.classes[48]&&!self.scene.cue_component_stop_complete(handle)? {
            return Ok(ReleaseStatus::Pending);
        }
        if handle.class_id==self.scene.classes[18]&&!self.scene.damage_zone_stop_complete(self.world.as_deref(),handle)?{
            return Ok(ReleaseStatus::Pending);
        }
        Ok(ReleaseStatus::Complete)
    }

    fn poll_deinitialize(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<ReleaseStatus, String> {
        //708620 returns actual objects even without a preceding709020 Stop
        //(initialized hidden cue pools). Await the real backend return.
        if !self.audio_objects_retired(handle)? {
            return Ok(ReleaseStatus::Pending);
        }
        self.deinitialize(handle, registry)?;
        Ok(ReleaseStatus::Complete)
    }

    fn deinitialize(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        self.retire_contacts(handle, true)?;
        if self.scene.is_flash_class(handle.class_id) {
            self.scene.deinitialize_flash_component(self.events,handle)?;
        }
        if self.scene.is_smart_object_class(handle.class_id) {
            self.scene.deinitialize_smart_object(self.events,handle)?;
        }
        if handle.class_id==self.scene.classes[48]||handle.class_id==self.scene.classes[49] {
            self.scene.deinitialize_cue_component(handle)?;
        }
        if self.scene.is_ai_startup_class(handle.class_id){self.scene.deinitialize_ai_startup(self.world.as_deref_mut().ok_or("AI manager Deinitialize requires published world")?,self.live,self.events,handle)?;}
        if self.scene.is_crew_class(handle.class_id){self.scene.deinitialize_crew(handle)?;}
        if self.scene.is_crew_collector_class(handle.class_id){self.scene.deinitialize_crew_collector(handle)?;}
        if self.scene.is_crew_module_class(handle.class_id){self.scene.deinitialize_crew_module(handle)?;}
        if handle.class_id==self.scene.classes[21]{self.scene.deinitialize_statistics(handle)?;}
        if handle.class_id==self.scene.classes[22]{self.scene.deinitialize_timers(self.world.as_deref_mut(),self.live,handle)?;}
        if handle.class_id==self.scene.classes[20]{self.scene.deinitialize_ai_danger(self.world.as_deref_mut().ok_or("AI danger Deinitialize requires published world")?,handle)?;}
        if handle.class_id==self.scene.classes[19]{self.scene.deinitialize_grenade(self.world.as_deref_mut(),handle)?;}
        if handle.class_id==self.scene.classes[18]{self.scene.deinitialize_damage_zone(handle)?;}
        if handle.class_id == self.scene.classes[17] {
            let component = self.scene.sound_volumes.get_mut(handle.slot).ok_or("sound volume Deinitialize payload absent")?;
            component.state.deinitialize();
            component.definition = None;
        }
        if handle.class_id == self.scene.classes[6] {
            if let Some(world) = self.world.as_deref_mut() {
                super::super::carrying_resources::release_pickable(
                    world, handle, self.scene.owner(handle)?.generation,
                )?;
            }
        }
        if handle.class_id == self.scene.classes[7] {
            self.scene.deinitialize_loot(self.live, self.events, handle)?;
        }
        if handle.class_id == self.scene.classes[13] {
            self.scene.deinitialize_projectile(handle)?;
        }
        if handle.class_id == self.scene.classes[15] {
            self.scene.retire_animation(handle)?;
        }
        if handle.class_id == self.scene.classes[14] {
            let shared = self.scene.world.clone();
            let mut query = shared
                .write()
                .map_err(|_| "physics world retirement lock poisoned")?;
            let scene = &mut self.scene;
            let query = &mut *query;
            scene_publication_events!(scene, registry, publication);
            scene
                .level_bodies
                .get_mut(handle.slot)
                .ok_or("PhysicsWorld retiring payload absent")?
                .retire(&mut query.storage, &mut query.geometry, &mut publication)?;
        }
        if handle.class_id == self.scene.classes[11] {
            let index = self
                .owner(handle)?
                .entity
                .ok_or("Breakable retiring entity absent")?;
            let id = self.scene.entities[index].shared;
            for child in self.scene.begin_breakable_retirement(id)? {
                if self.live.get(child).is_some() {
                    self.live.request_removal(child)?;
                }
                self.scene.acknowledge_breakable_chunk_removal(id, child)?;
            }
        }
        if self.scene.is_rigid_class(handle.class_id) {
            if !self.scene.rigid_component(handle)?.can_release() {
                return self
                    .scene
                    .deinitialize_placed_rigid(handle, registry, self.events);
            }
            return self.shutdown(handle, registry);
        }
        if handle.class_id == self.scene.classes[9] {
            self.scene
                .factions
                .get_mut(handle.slot)
                .ok_or("Faction component absent")?
                .deinitialize()?;
        }
        if handle.class_id == self.scene.classes[0] {
            let scene = &mut self.scene;
            let shared = scene.world.clone();
            let mut query_owner = shared
                .write()
                .map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            scene_publication_events!(scene, registry, events);
            crate::gameplay::trigger_body::startup::deinitialize_trigger(
                &mut world.storage,
                &mut world.geometry,
                &mut events,
                handle,
            )?;
            let placement = scene
                .triggers
                .get(handle.slot)
                .ok_or("trigger component absent")?
                .owner
                .declaration
                .ok_or("trigger declaration absent")?;
            world.geometry.retire_placement(placement)?;
        }
        if handle.class_id == self.scene.classes[4] && self.world.is_some() {
            let generation = self.owner(handle)?.generation;
            {
                let component = self.scene.audios.get_mut(handle.slot).ok_or("audio component absent")?;
                super::super::audio::drain_component_posts(
                    self.world.as_deref_mut().ok_or("published audio owner absent")?,
                    handle, generation, component,
                )?;
                super::super::audio::retire_component_joints(
                    self.world.as_deref_mut().ok_or("published audio owner absent")?,
                    handle.packed(), generation, component,
                )?;
            }
            if !super::super::audio::retire_component_object(
                self.world
                    .as_deref_mut()
                    .ok_or("published audio owner absent")?,
                crate::gameplay::sound::Emitter::Component {
                    handle: handle.packed(),
                    generation,
                },
            )? {
                return Err("audio object retirement lost its acknowledged Stop".into());
            }
            let component = self
                .scene
                .audios
                .get_mut(handle.slot)
                .ok_or("audio component absent")?;
            super::super::audio::release_component_animation(
                self.world.as_deref_mut().ok_or("published audio owner absent")?, component,
            )?;
            component.allocated = false;
            component.started = false;
        }
        if handle.class_id == self.scene.classes[4] {
            let component=self.scene.audios.get(handle.slot).ok_or("audio component absent")?;
            if !component.posts.is_empty() || !component.animation_scopes.is_empty() {
                return Err("audio Deinitialize retains unacknowledged animation resources".into());
            }
        }
        self.shutdown(handle, registry)
    }

    fn shutdown(
        &mut self,
        handle: ComponentHandle,
        _: &mut ComponentRegistry,
    ) -> Result<(), String> {
        let index = self
            .owner(handle)?
            .entity
            .ok_or("retiring component entity absent")?;
        if self.scene.is_ai_startup_class(handle.class_id){return self.scene.release_ai_startup_binding(index,handle);}
        if self.scene.is_crew_class(handle.class_id){return self.scene.release_crew_binding(index,handle);}
        if self.scene.is_crew_collector_class(handle.class_id){return self.scene.release_crew_collector_binding(index,handle);}
        if self.scene.is_crew_module_class(handle.class_id){return self.scene.release_crew_module(index,handle);}
        let scene = &mut self.scene;
        let entity = scene
            .entities
            .get_mut(index)
            .ok_or("retiring physics entity absent")?;
        if handle.class_id == scene.classes[0] {
            entity.components.release(
                handle,
                scene
                    .triggers
                    .get_mut(handle.slot)
                    .ok_or("trigger absent")?,
            )
        } else if handle.class_id == scene.classes[1] {
            entity.components.release(
                handle,
                scene.waters.get_mut(handle.slot).ok_or("water absent")?,
            )
        } else if handle.class_id == scene.classes[2] {
            entity.components.release(
                handle,
                scene.winds.get_mut(handle.slot).ok_or("wind absent")?,
            )
        } else if handle.class_id == scene.classes[3] {
            let actor = scene
                .actors
                .actors
                .get_mut(handle.slot)
                .ok_or("reference actor absent")?;
            let receipt = (handle.packed(), actor.owner.generation);
            let state = match self.scoped.as_deref_mut() {
                Some(state) => state,
                None => scene
                    .retirement
                    .as_mut()
                    .ok_or("reference actor has no retirement cursor")?,
            };
            state
                .actor_owners
                .try_reserve(1)
                .map_err(|error| format!("retiring actor owner receipts: {error}"))?;
            state.actor_owners.insert(receipt, index);
            if actor.current_body != actor.default_body {
                return Err("reference actor still owns a component body".into());
            }
            entity.components.release(handle, actor)?;
            if entity.components.reference_actor == Some(handle) {
                entity.components.reference_actor = None;
            }
            Ok(())
        } else if handle.class_id == scene.classes[4] {
            entity.components.release(
                handle,
                scene.audios.get_mut(handle.slot).ok_or("audio absent")?,
            )
        } else if handle.class_id == scene.classes[5] {
            entity.components.release(
                handle,
                scene.usables.get_mut(handle.slot).ok_or("usable absent")?,
            )
        } else if handle.class_id == scene.classes[6] {
            let component = scene
                .pickables
                .get_mut(handle.slot)
                .ok_or("Pickable absent")?;
            component.references = None;
            component.player = None;
            component.item = None;
            component.loot_owner = None;
            scene.readable_components.remove(&entity.name);
            entity.components.release(handle, component)
        } else if handle.class_id == scene.classes[7] {
            entity.components.release(
                handle,
                scene.loots.get_mut(handle.slot).ok_or("Loot absent")?,
            )
        } else if handle.class_id == scene.classes[8] || handle.class_id == scene.classes[12] {
            let component = rigid_pool_mut!(scene, handle.class_id)?
                .get_mut(handle.slot)
                .ok_or("rigid component slot is absent")?;
            if !component.can_release() {
                return Err("RigidBody resources remain at shutdown".into());
            }
            entity.components.release(handle, component)
        } else if handle.class_id == scene.classes[9] {
            entity.components.release(
                handle,
                scene
                    .factions
                    .get_mut(handle.slot)
                    .ok_or("Faction absent")?,
            )
        } else if handle.class_id == scene.classes[10] {
            entity.components.release(
                handle,
                scene.health.get_mut(handle.slot).ok_or("health absent")?,
            )
        } else if handle.class_id == scene.classes[11] {
            let component = scene
                .breakables
                .get_mut(handle.slot)
                .ok_or("Breakable shutdown payload absent")?;
            if !component.runtime.stopped() {
                return Err("Breakable destructor retains source journal/chunks".into());
            }
            entity.components.release(handle, component)
        } else if handle.class_id == scene.classes[13] {
            entity.components.release(
                handle,
                scene
                    .projectiles
                    .get_mut(handle.slot)
                    .ok_or("Projectile shutdown payload absent")?,
            )
        } else if handle.class_id == scene.classes[14] {
            let component = scene
                .level_bodies
                .get_mut(handle.slot)
                .ok_or("PhysicsWorld shutdown payload absent")?;
            if component.source.is_some() || !component.bodies.is_empty() {
                return Err("PhysicsWorld still retains body/source ownership".into());
            }
            entity.components.release(handle, component)
        } else if handle.class_id == scene.classes[15] {
            entity.components.release(
                handle,
                scene
                    .animations
                    .get_mut(handle.slot)
                    .ok_or("AnimBase shutdown payload absent")?,
            )
        } else if handle.class_id == scene.classes[16] {
            entity.components.release(
                handle,
                scene.health_doors.get_mut(handle.slot).ok_or("door health absent")?,
            )
        } else if handle.class_id == scene.classes[17] {
            entity.components.release(
                handle,
                scene.sound_volumes.get_mut(handle.slot).ok_or("sound volume absent")?,
            )
        } else if handle.class_id == scene.classes[19] {
            entity.components.release(handle,scene.grenades.get_mut(handle.slot).ok_or("Grenade absent")?)
        } else if handle.class_id == scene.classes[21] {
            entity.components.release(handle,scene.statistics.get_mut(handle.slot).ok_or("AIStatistics absent")?)
        } else if handle.class_id == scene.classes[22] {
            entity.components.release(handle,scene.timers.get_mut(handle.slot).ok_or("Timers absent")?)
        } else if handle.class_id == scene.classes[20] {
            entity.components.release(handle,scene.ai_danger_zones.get_mut(handle.slot).ok_or("AI danger absent")?)
        } else if handle.class_id==scene.classes[48] {
            entity.components.release(handle,scene.observables.get_mut(handle.slot).ok_or("Observable absent")?)
        } else if handle.class_id==scene.classes[49] {
            entity.components.release(handle,scene.attention_sources.get_mut(handle.slot).ok_or("AttentionSource absent")?)
        } else if handle.class_id==scene.classes[50] {
            entity.components.release(handle,scene.investigate_spots.get_mut(handle.slot).ok_or("InvestigateSpot absent")?)
        } else if handle.class_id==scene.classes[51] {
            entity.components.release(handle,scene.dynamic_search_spots.get_mut(handle.slot).ok_or("SearchSpotDynamic absent")?)
        } else if handle.class_id==scene.classes[52] {
            entity.components.release(handle,scene.flash_surfaces.get_mut(handle.slot).ok_or("FlashSurface absent")?)
        } else if handle.class_id==scene.classes[53] {
            entity.components.release(handle,scene.interactive_maps.get_mut(handle.slot).ok_or("InteractiveMap absent")?)
        } else if handle.class_id == scene.classes[18] {
            entity.components.release(handle,scene.damage_zones.get_mut(handle.slot).ok_or("DamageZone absent")?)
        } else {
            Err("physics shutdown class is unsupported".into())
        }
    }

    fn poll_released(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<ReleaseStatus, String> {
        //A canceled, uninitialized registration can bypass Deinitialize.
        //Retain its actual immediate-release generation until object return.
        if !self.audio_objects_retired(handle)? { return Ok(ReleaseStatus::Pending); }
        self.released(handle, registry)?;
        Ok(ReleaseStatus::Complete)
    }

    fn released(
        &mut self,
        handle: ComponentHandle,
        registry: &mut ComponentRegistry,
    ) -> Result<(), String> {
        registry.ensure_no_update_references(handle)?;
        if handle.class_id==self.scene.classes[18]&&!self.scene.damage_zone_stop_complete(self.world.as_deref(),handle)?{
            return Err("DamageZone pool release precedes actual query/FX acknowledgement".into());
        }
        if handle.class_id==self.scene.classes[19]&&!self.scene.grenade_stop_complete(self.world.as_deref(),handle)?{
            return Err("Grenade pool release precedes actual FX acknowledgement".into());
        }
        if self.owner(handle)?.entity.is_some() {
            return Err("component remains linked at pool release".into());
        }
        let audio = if handle.class_id == self.scene.classes[4] && self.world.is_some() {
            let emitter = crate::gameplay::sound::Emitter::Component {
                handle: handle.packed(),
                generation: self.owner(handle)?.generation,
            };
            if !super::super::audio::retire_component_object(
                self.world
                    .as_deref_mut()
                    .ok_or("published audio owner absent")?,
                emitter,
            )? {
                return Err("audio pool release requires its acknowledged sound object".into());
            }
            Some(emitter)
        } else {
            None
        };
        let scene = &mut self.scene;
        if scene.is_ai_startup_class(handle.class_id) {
            scene.reclaim_ai_startup(handle)?;
        } else if scene.is_crew_class(handle.class_id) {
            scene.reclaim_crew(handle)?;
        } else if scene.is_crew_collector_class(handle.class_id) {
            scene.reclaim_crew_collector(handle)?;
        } else if scene.is_crew_module_class(handle.class_id) {
            scene.reclaim_crew_module(handle)?;
        } else if handle.class_id == scene.classes[0] {
            scene.triggers.release(handle.slot)?;
        } else if handle.class_id == scene.classes[1] {
            scene.waters.release(handle.slot)?;
        } else if handle.class_id == scene.classes[2] {
            scene.winds.release(handle.slot)?;
        } else if handle.class_id == scene.classes[3] {
            let actor = scene
                .actors
                .actors
                .get(handle.slot)
                .ok_or("retiring actor absent")?;
            if actor.parent.is_some()
                || actor.binding.entity.is_some()
                || !actor.children.is_empty()
            {
                let record = (handle, actor.owner.generation);
                let state = match self.scoped.as_deref_mut() {
                    Some(state) => state,
                    None => scene
                        .retirement
                        .as_mut()
                        .ok_or("actor release has no retirement owner")?,
                };
                if !state.retained_actors.contains(&record) {
                    state
                        .retained_actors
                        .try_reserve(1)
                        .map_err(|error| format!("retiring attachment actor handles: {error}"))?;
                    state.retained_actors.push(record);
                }
            } else {
                let generation = actor.owner.generation;
                reclaim_reference_actor(scene, registry, handle, generation)?;
            }
        } else if handle.class_id == scene.classes[4] {
            scene.audios.release(handle.slot)?;
        } else if handle.class_id == scene.classes[5] {
            scene.usables.release(handle.slot)?;
        } else if handle.class_id == scene.classes[6] {
            scene.pickables.release(handle.slot)?;
        } else if handle.class_id == scene.classes[7] {
            scene.loots.release(handle.slot)?;
        } else if handle.class_id == scene.classes[8] || handle.class_id == scene.classes[12] {
            let component = scene.rigid_component(handle)?;
            if !component.can_release() {
                return Err("RigidBody resources still live at release".into());
            }
            let movement = component.movement_body()?;
            if scene
                .actors
                .bodies
                .get(movement)
                .ok_or("rigid movement slot absent")?
                .actor
                .is_some()
                || scene
                    .actors
                    .actors
                    .iter()
                    .any(|actor| actor.current_body == movement || actor.default_body == movement)
            {
                return Err("rigid movement interface still referenced".into());
            }
            scene
                .free_rigid_movement
                .try_reserve(1)
                .map_err(|error| format!("rigid free slot: {error}"))?;
            scene.actors.bodies[movement].body_id = 0x00ff_ffff;
            scene.free_rigid_movement.push(movement);
            scene
                .rigid_pool_mut(handle.class_id)?
                .release(handle.slot)?;
        } else if handle.class_id == scene.classes[9] {
            scene.factions.release(handle.slot)?;
        } else if handle.class_id == scene.classes[10] {
            scene.health.release(handle.slot)?;
        } else if handle.class_id == scene.classes[11] {
            scene.breakables.release(handle.slot)?;
        } else if handle.class_id == scene.classes[13] {
            scene.release_projectile_interface(handle)?;
            scene.projectiles.release(handle.slot)?;
        } else if handle.class_id == scene.classes[14] {
            scene.level_bodies.release(handle.slot)?;
        } else if handle.class_id == scene.classes[15] {
            scene.animations.release(handle.slot)?;
        } else if handle.class_id == scene.classes[16] {
            scene.health_doors.release(handle.slot)?;
        } else if handle.class_id == scene.classes[17] {
            scene.sound_volumes.release(handle.slot)?;
        } else if handle.class_id == scene.classes[19] {
            scene.grenades.release(handle.slot)?;
        } else if handle.class_id == scene.classes[21] {
            scene.statistics.release(handle.slot)?;
        } else if handle.class_id == scene.classes[22] {
            scene.timers.release(handle.slot)?;
        } else if handle.class_id == scene.classes[20] {
            scene.ai_danger_zones.release(handle.slot)?;
        } else if handle.class_id==scene.classes[48] {
            scene.observables.release(handle.slot)?;
        } else if handle.class_id==scene.classes[49] {
            scene.attention_sources.release(handle.slot)?;
        } else if handle.class_id==scene.classes[50] {
            scene.investigate_spots.release(handle.slot)?;
        } else if handle.class_id==scene.classes[51] {
            scene.dynamic_search_spots.release(handle.slot)?;
        } else if handle.class_id==scene.classes[52] {
            scene.flash_surfaces.release(handle.slot)?;
        } else if handle.class_id==scene.classes[53] {
            scene.interactive_maps.release(handle.slot)?;
        } else if handle.class_id == scene.classes[18] {
            scene.damage_zones.release(handle.slot)?;
        } else {
            return Err("physics pool release class is unsupported".into());
        }
        scene.declarations.remove(&handle.packed());
        if let Some(emitter) = audio {
            super::super::audio::forget_retired_component_object(
                self.world
                    .as_deref_mut()
                    .ok_or("published audio owner absent")?,
                emitter,
            )?;
        }
        Ok(())
    }
}
