//! Retirement of the five concrete component families hosted by PhysicsScene.
//! 7e72c0/7ea870 drain the real component registry before managed identities
//! retire. This adapter does not instantiate unsupported component families.
use super::*;
use crate::gameplay::{
    component_state::{lifecycle::{ComponentReleaseWorld, ReleaseStatus}, updates::ComponentUpdateWorld},
    entity_events::EntityEvents,
    entity_lifetime::LiveEntities,
};

#[derive(Default)]
pub(super) struct Retirement {
    components: Vec<(ComponentHandle, u32)>,
    requested: usize,
    complete: bool,
    retire_next: usize,
    stopped: std::collections::HashSet<(u64, u32)>,
}

pub(in crate::app) fn retirement_progress(world: &bevy::prelude::World) -> Option<(usize, usize, usize, bool)> {
    world.get_resource::<PhysicsScene>()?.retirement.as_ref()
        .map(|state| (state.requested, state.stopped.len(), state.retire_next, state.complete))
}

/// Removal from World is temporary; every error and Pending result restores
/// these sole owners. No resource destruction follows an incomplete drain.
pub(in crate::app) fn poll(world: &mut bevy::prelude::World) -> Result<bool, String> {
    let Some(mut scene) = world.remove_resource::<PhysicsScene>() else { return Ok(true); };
    let Some(mut live) = world.remove_resource::<LiveEntities>() else {
        world.insert_resource(scene);
        return Err("physics retirement has no managed entity owner".into());
    };
    let Some(mut events) = world.remove_resource::<EntityEvents>() else {
        world.insert_resource(live);
        world.insert_resource(scene);
        return Err("physics retirement has no entity event owner".into());
    };
    let result = poll_owned(world, &mut scene, &mut live, &mut events);
    world.insert_resource(events);
    world.insert_resource(live);
    world.insert_resource(scene);
    if matches!(result, Ok(false)) { super::super::audio::poll_object_retirements(world)?; }
    result
}

fn poll_owned(world: &mut bevy::prelude::World, scene: &mut PhysicsScene,
    live: &mut LiveEntities, events: &mut EntityEvents) -> Result<bool, String> {
    if scene.retirement.as_ref().is_some_and(|state| state.complete) { return Ok(true); }
    if scene.retirement.is_none() {
        let mut components = Vec::new();
        let count = scene.entities.iter().map(|entity| entity.components.entries.values().len()).sum();
        components.try_reserve_exact(count).map_err(|error| format!("physics retirement snapshot: {error}"))?;
        for entity in &scene.entities {
            if live.get(entity.shared).is_none() { return Err("physics retirement entity is stale".into()); }
            for &packed in entity.components.entries.values().iter().rev() {
                let handle = ComponentHandle::from_packed(packed)?;
                components.push((handle, scene.owner(handle)?.generation));
            }
        }
        let mut stopped = std::collections::HashSet::new();
        stopped.try_reserve(components.len()).map_err(|error| format!("physics Stop completion snapshot: {error}"))?;
        scene.retirement = Some(Retirement { components, stopped, ..Default::default() });
    }
    let mut registry = scene.registry.take().ok_or("physics lifecycle registry absent")?;
    let result = (|| {
        let mut host = Host { world: &mut *world, scene: &mut *scene, events: &mut *events };
        loop {
            let state = host.scene.retirement.as_ref().ok_or("physics retirement progress absent")?;
            let Some(&(handle, generation)) = state.components.get(state.requested) else { break; };
            if host.owner(handle)?.generation != generation { return Err("physics retirement component is stale".into()); }
            registry.request_release(handle, &mut host)?;
            host.scene.retirement.as_mut().unwrap().requested += 1;
        }
        if registry.poll_stop_and_release(&mut host)? == ReleaseStatus::Pending { return Ok(false); }
        // Stop callbacks may enqueue update membership changes. The slots must
        // remain allocated until those exact references have been drained.
        registry.drain_update_changes(&mut host)?;
        if host.scene.entities.iter().any(|entity| !entity.components.entries.values().is_empty())
            || !host.scene.declarations.is_empty() {
            return Err("physics component drain retained linked owners".into());
        }
        {
            let query_world = host.scene.world.read().map_err(|_| "level physics query owner is poisoned")?;
            // Body zero is the canonical static anchor, not a component body.
            if !query_world.storage.publication_idle() || query_world.storage.bodies.active_count() != 1 {
                return Err("physics world still owns unfinished body retirement".into());
            }
        }
        while host.scene.retirement.as_ref().unwrap().retire_next < host.scene.entities.len() {
            let index = host.scene.retirement.as_ref().unwrap().retire_next;
            let entity = &host.scene.entities[index];
            if live.retire_for_world_shutdown(entity.shared).is_none() {
                return Err("physics managed entity remains attached during retirement".into());
            }
            host.scene.retirement.as_mut().unwrap().retire_next += 1;
        }
        host.events.clear_for_world_shutdown();
        host.scene.shared_indices.clear();
        host.scene.retirement.as_mut().unwrap().complete = true;
        Ok(true)
    })();
    scene.registry = Some(registry);
    result
}

struct Host<'a> {
    world: &'a mut bevy::prelude::World,
    scene: &'a mut PhysicsScene,
    events: &'a mut EntityEvents,
}

impl ComponentLifecycleWorld for Host<'_> {
    fn priority(&self, handle: ComponentHandle) -> Result<i32, String> { self.scene.priority(handle) }
    fn owner(&self, handle: ComponentHandle) -> Result<&ComponentOwner, String> { self.scene.owner(handle) }
    fn owner_mut(&mut self, handle: ComponentHandle) -> Result<&mut ComponentOwner, String> { self.scene.owner_mut(handle) }
    fn initialize(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> { self.scene.initialize(handle, registry) }
    fn start(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> { self.scene.start(handle, registry) }
    fn stop_queued(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> { self.scene.stop_queued(handle, registry) }
    fn initialization_failed(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> { self.scene.initialization_failed(handle, registry) }
}

impl ComponentUpdateWorld for Host<'_> {
    fn update_remove(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> { registry.base_update_remove(handle, self) }
    fn update_add(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> { registry.base_update_add(handle, self) }
    fn set_update_scheduled(&mut self, handle: ComponentHandle, scheduled: bool) -> Result<(), String> {
        macro_rules! membership {
            ($pool:expr) => { if scheduled { $pool.schedule(handle.slot) } else { $pool.unschedule(handle.slot) } };
        }
        let scene = &mut self.scene;
        if handle.class_id == scene.classes[0] { membership!(scene.triggers) }
        else if handle.class_id == scene.classes[1] { membership!(scene.waters) }
        else if handle.class_id == scene.classes[2] { membership!(scene.winds) }
        else if handle.class_id == scene.classes[3] { membership!(scene.actors.actors) }
        else if handle.class_id == scene.classes[4] { membership!(scene.audios) }
        else if handle.class_id == scene.classes[5] { membership!(scene.usables) }
        else if handle.class_id == scene.classes[6] { membership!(scene.pickables) }
        else if handle.class_id == scene.classes[7] { membership!(scene.loots) }
        else { Err("physics update roster class is unsupported".into()) }
    }
}

impl ComponentReleaseWorld for Host<'_> {
    fn pump_events(&mut self, _: &mut ComponentRegistry) -> Result<(), String> {
        // No callback in the current five factories subscribes numeric entity
        // messages. Preserve queued errors instead of fabricating dispatch for
        // an unimplemented listener owner. This runs without a world lock.
        let errors = self.events.pump(|listener, _, _| Err(format!("physics retirement has no callback for {listener:?}")));
        if errors.is_empty() { Ok(()) } else { Err(errors.join("; ")) }
    }

    fn stop(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        let scene = &mut self.scene;
        if handle.class_id == scene.classes[0] {
            let declaration = scene.owner(handle)?.declaration.ok_or("trigger declaration absent")?;
            let shared = scene.world.clone();
            let mut query_owner = shared.write().map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            let keyframed = world.geometry.placements.get(declaration).ok_or("trigger placement absent")?.binding.collision.keyframed;
            let mut events = ComponentPublicationEvents {
                components: TriggerComponentStore { trigger_class: scene.classes[0], water_class: scene.classes[1],
                    triggers: &mut scene.triggers, waters: &mut scene.waters, actors: &mut scene.actors, deferred: &mut scene.deferred },
                signals: &mut scene.signals, dispatch: startup::dispatch_observer,
            };
            crate::gameplay::trigger_body::startup::stop_trigger(&mut world.storage, &mut world.geometry,
                &mut events, handle, keyframed, |components, reference| {
                    let body = components.trigger(reference)?.movement_body;
                    let actor = components.actors.bodies.get(body).and_then(|body| body.actor).ok_or("trigger reference actor absent")?;
                    components.actors.detach_body(actor, body)
                })?;
            drop(query_owner);
        } else if handle.class_id == scene.classes[4] {
            scene.audios.get_mut(handle.slot).ok_or("audio component absent")?.started = false;
        } else if handle.class_id == scene.classes[5] {
            let actor = scene.usables.get(handle.slot).ok_or("usable component absent")?.actor;
            if !self.world.contains_resource::<super::super::interactions::Interactions>()
                || !self.world.contains_resource::<super::super::NavigationWorld>()
                || !self.world.contains_resource::<super::super::usable_animation::UsableAnimations>()
                || !self.world.contains_resource::<super::super::usable_effects::UsableEffects>() {
                return Err("usable Stop is waiting for its interaction backends".into());
            }
            self.world.resource_scope(|world, mut interactions: bevy::prelude::Mut<super::super::interactions::Interactions>| {
                world.resource_scope(|world, mut navigation: bevy::prelude::Mut<super::super::NavigationWorld>| {
                    world.resource_scope(|world, mut animations: bevy::prelude::Mut<super::super::usable_animation::UsableAnimations>| {
                        let mut effects = world.resource_mut::<super::super::usable_effects::UsableEffects>();
                        interactions.stop_actor(actor, &mut navigation.0, &mut animations, &mut effects);
                    });
                });
            });
        }
        registry.stop_update_ownership(handle, self)
    }

    fn poll_stop(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry,
        _first: bool) -> Result<ReleaseStatus, String> {
        let identity = (handle.packed(), self.owner(handle)?.generation);
        if !self.scene.retirement.as_ref().ok_or("physics retirement progress absent")?.stopped.contains(&identity) {
            self.stop(handle, registry)?;
            // Capacity was reserved before any callback. Failed callbacks retry
            // against their real body cursor; completed Stop is never replayed.
            self.scene.retirement.as_mut().unwrap().stopped.insert(identity);
        }
        if handle.class_id == self.scene.classes[4] {
            let generation = self.owner(handle)?.generation;
            if !super::super::audio::retire_component_object(self.world,
                crate::gameplay::sound::Emitter::Component { handle: handle.packed(), generation })? {
                return Ok(ReleaseStatus::Pending);
            }
        }
        Ok(ReleaseStatus::Complete)
    }

    fn deinitialize(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        if handle.class_id == self.scene.classes[0] {
            let scene = &mut self.scene;
            let shared = scene.world.clone();
            let mut query_owner = shared.write().map_err(|_| "level physics query owner is poisoned")?;
            let world = &mut *query_owner;
            let mut events = ComponentPublicationEvents {
                components: TriggerComponentStore { trigger_class: scene.classes[0], water_class: scene.classes[1],
                    triggers: &mut scene.triggers, waters: &mut scene.waters, actors: &mut scene.actors, deferred: &mut scene.deferred },
                signals: &mut scene.signals, dispatch: startup::dispatch_observer,
            };
            crate::gameplay::trigger_body::startup::deinitialize_trigger(&mut world.storage, &mut world.geometry, &mut events, handle)?;
        }
        if handle.class_id == self.scene.classes[4] {
            let generation = self.owner(handle)?.generation;
            if !super::super::audio::retire_component_object(self.world,
                crate::gameplay::sound::Emitter::Component { handle: handle.packed(), generation })? {
                return Err("audio object retirement lost its acknowledged Stop".into());
            }
            let component = self.scene.audios.get_mut(handle.slot).ok_or("audio component absent")?;
            component.allocated = false;
            component.started = false;
        }
        self.shutdown(handle, registry)
    }

    fn shutdown(&mut self, handle: ComponentHandle, _: &mut ComponentRegistry) -> Result<(), String> {
        let index = self.owner(handle)?.entity.ok_or("retiring component entity absent")?;
        let scene = &mut self.scene;
        let entity = scene.entities.get_mut(index).ok_or("retiring physics entity absent")?;
        if handle.class_id == scene.classes[0] { entity.components.release(handle, scene.triggers.get_mut(handle.slot).ok_or("trigger absent")?) }
        else if handle.class_id == scene.classes[1] { entity.components.release(handle, scene.waters.get_mut(handle.slot).ok_or("water absent")?) }
        else if handle.class_id == scene.classes[2] { entity.components.release(handle, scene.winds.get_mut(handle.slot).ok_or("wind absent")?) }
        else if handle.class_id == scene.classes[3] {
            let actor = scene.actors.actors.get_mut(handle.slot).ok_or("reference actor absent")?;
            if actor.parent.is_some() || actor.binding.entity.is_some() || !actor.children.is_empty() || actor.current_body != actor.default_body {
                return Err("reference actor remains attached".into());
            }
            entity.components.release(handle, actor)?;
            if entity.components.reference_actor == Some(handle) { entity.components.reference_actor = None; }
            Ok(())
        } else if handle.class_id == scene.classes[4] { entity.components.release(handle, scene.audios.get_mut(handle.slot).ok_or("audio absent")?) }
        else if handle.class_id == scene.classes[5] { entity.components.release(handle, scene.usables.get_mut(handle.slot).ok_or("usable absent")?) }
        else if handle.class_id == scene.classes[6] {
            let component = scene.pickables.get_mut(handle.slot).ok_or("Pickable absent")?;
            component.references = None;
            component.player = None;
            component.item = None;
            component.loot_owner = None;
            scene.readable_components.remove(&entity.name);
            entity.components.release(handle, component)
        }
        else if handle.class_id == scene.classes[7] { entity.components.release(handle, scene.loots.get_mut(handle.slot).ok_or("Loot absent")?) }
        else { Err("physics shutdown class is unsupported".into()) }
    }

    fn released(&mut self, handle: ComponentHandle, registry: &mut ComponentRegistry) -> Result<(), String> {
        registry.ensure_no_update_references(handle)?;
        if self.owner(handle)?.entity.is_some() { return Err("component remains linked at pool release".into()); }
        let audio = if handle.class_id == self.scene.classes[4] {
            let emitter = crate::gameplay::sound::Emitter::Component { handle: handle.packed(), generation: self.owner(handle)?.generation };
            if !super::super::audio::retire_component_object(self.world, emitter)? {
                return Err("audio pool release requires its acknowledged sound object".into());
            }
            Some(emitter)
        } else { None };
        let scene = &mut self.scene;
        if handle.class_id == scene.classes[0] { scene.triggers.release(handle.slot)?; }
        else if handle.class_id == scene.classes[1] { scene.waters.release(handle.slot)?; }
        else if handle.class_id == scene.classes[2] { scene.winds.release(handle.slot)?; }
        else if handle.class_id == scene.classes[3] { scene.actors.actors.release(handle.slot)?; }
        else if handle.class_id == scene.classes[4] { scene.audios.release(handle.slot)?; }
        else if handle.class_id == scene.classes[5] { scene.usables.release(handle.slot)?; }
        else if handle.class_id == scene.classes[6] { scene.pickables.release(handle.slot)?; }
        else if handle.class_id == scene.classes[7] { scene.loots.release(handle.slot)?; }
        else { return Err("physics pool release class is unsupported".into()); }
        scene.declarations.remove(&handle.packed());
        if let Some(emitter) = audio { super::super::audio::forget_retired_component_object(self.world, emitter)?; }
        Ok(())
    }
}
