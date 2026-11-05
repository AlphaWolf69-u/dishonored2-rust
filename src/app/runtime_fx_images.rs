//! Runtime particle-image leases using the scene's existing streaming slots.
use bevy::prelude::*;
use std::{collections::HashMap, sync::{Arc, Weak, atomic::Ordering}};
use crate::rendering::{bevy_backend::particle_images::SceneParticleImages, particles::image_resources::ParticleImage};

struct Slot {
    index: usize,
    image: Weak<ParticleImage>,
    owned: bool,
    registered: bool,
    added_consumer: bool,
    wanted: bool,
    lease: Option<Arc<super::resident_images::ResidentImage>>,
}

#[derive(Resource, Default)]
pub(super) struct RuntimeFxImages {
    slots: HashMap<Arc<str>, Slot>,
    free: Vec<usize>,
    seen_revision: Option<u64>,
    seen_world_revision: Option<u64>,
    world_entries: std::collections::HashSet<String>,
    pub(super) requested: Vec<usize>,
    pub(super) failure: Option<(u64, String)>,
}

impl RuntimeFxImages {
    /// Scene streaming slot only after its particle image identity has been
    /// registered with the shared consumer owner.
    pub(super) fn slot(&self, path: &str) -> Option<usize> {
        self.slots.get(path).filter(|slot| slot.registered).map(|slot| slot.index)
    }
}

/// Register newly acquired images without replacing the scene's existing
/// slot identities. Texture reads continue through TextureReadWorker.
pub(super) fn synchronize(
    owner: Option<Res<super::usable_effects::UsableEffects>>,
    world: Option<ResMut<super::usable_effects::DynamicWorldMaterials>>,
    state: Option<ResMut<RuntimeFxImages>>,
    queue: Option<ResMut<super::SceneRenderQueue>>,
    worker: Option<ResMut<super::texture_streaming::TextureReadWorker>>,
    consumers: Option<ResMut<SceneParticleImages>>,
    decals: Option<Res<super::usable_effects::DynamicDecalMaterials>>,
    prewarm: Res<crate::rendering::bevy_backend::world_effects::DynamicMaterialPrewarm>,
    registry: Option<Res<super::resident_images::ResidentImages>>,
    assets: Res<Assets<Image>>,
) {
    let (Some(owner), Some(mut world), Some(mut state), Some(mut queue), Some(mut worker), Some(mut consumers)) =
        (owner, world, state, queue, worker, consumers) else { return };
    if state.failure.as_ref().is_some_and(|(generation, _)| *generation != world.generation) {
        state.failure = None;
    }
    if owner.ready.is_empty() && world.images.is_empty() && world.extra_images.is_empty() && state.slots.is_empty() && state.world_entries.is_empty() {
        world.images_ready = world.image_revision;
        if state.seen_revision != Some(owner.revision) || state.seen_world_revision != Some(world.image_revision) { state.seen_revision = Some(owner.revision); }
        if !state.requested.is_empty() { state.requested.clear(); }
        return;
    }
    if state.seen_revision != Some(owner.revision) || state.seen_world_revision != Some(world.image_revision) {
        for slot in state.slots.values_mut() { slot.wanted = false; }
        for source in owner.ready.values().flat_map(|sources| sources.particle_materials())
            .flat_map(|material| material.images.values())
            .chain(world.images.values().flat_map(|images| images.iter().map(|(_, image)| image)))
            .chain(world.extra_images.iter())
            .filter(|image| image.scene_owned)
        {
            let path = source.path.clone();
            if let Some(slot) = state.slots.get_mut(&path) {
                slot.wanted = true;
                // An immutable registry normally reuses the same identity; a
                // fresh acquisition after retirement must not retain a dead weak pointer.
                if slot.image.strong_count() == 0 { slot.image = Arc::downgrade(source); }
                continue;
            }
            if let Some((index, _)) = consumers.slots.iter().find(|(_, image)| image.path == path) {
                state.slots.insert(path, Slot { index: *index, image: Arc::downgrade(source), owned: false, registered: true, added_consumer: false, wanted: true, lease: None });
                continue;
            }
            let existing = queue.diffuse_image_paths.iter().position(|name| name == path.as_ref());
            let index = if let Some(index) = existing { index } else if let Some(index) = state.free.pop() {
                queue.diffuse_image_paths[index] = path.to_string();
                index
            } else {
                let index = queue.diffuse_image_paths.len();
                queue.diffuse_image_paths.push(path.to_string());
                queue.diffuse_image_demands.push(Default::default());
                queue.diffuse_image_materials.push(Vec::new());
                queue.diffuse_image_handles.push(None);
                queue.diffuse_image_pending.push(None);
                queue.diffuse_mip_states.push(Default::default());
                queue.diffuse_image_attempted.push(false);
                queue.diffuse_image_last_visible.push(0);
                worker.inflight.push(false);
                index
            };
            let priority = source.sampling_priority.load(Ordering::Relaxed);
            queue.diffuse_image_demands[index].raise_priority(priority);
            queue.diffuse_mip_states[index].sampling_priority = priority;
            state.slots.insert(path, Slot { index, image: Arc::downgrade(source), owned: existing.is_none(), registered: false, added_consumer: false, wanted: true, lease: None });
            if crate::rendering::bevy_backend::world_effects::trace_lifecycle() {
                eprintln!("FX_LIFECYCLE scene_image_request={} slot={index} reused_scene={}", source.path, existing.is_some());
            }
        }
        state.seen_revision = Some(owner.revision);
        state.seen_world_revision = Some(world.image_revision);
    }
    state.requested.clear();
    let mut remove = Vec::new();
    let mut failed = None;
    let state = &mut *state;
    let (slots, requested) = (&mut state.slots, &mut state.requested);
    for (path, slot) in slots {
        if !slot.wanted && !slot.added_consumer && !slot.owned {
            remove.push(path.clone());
            continue;
        }
        let strong = slot.image.strong_count();
        // One registered scene consumer is our lease; all other strong
        // references belong to active definitions, instances or GPU material.
        if strong <= usize::from(slot.added_consumer) {
            if !worker.inflight.get(slot.index).copied().unwrap_or(false)
                && queue.diffuse_image_pending[slot.index].is_none() {
                remove.push(path.clone());
            }
            continue;
        }
        if slot.wanted { requested.push(slot.index); }
        if slot.wanted && queue.diffuse_image_handles[slot.index].is_none() {
            if let Some(error) = worker.initial_failures.get(&slot.index) {
                failed.get_or_insert_with(|| format!("runtime FX image {path}: {error}"));
            }
        }
        let Some(level) = queue.diffuse_mip_states[slot.index].resident else { continue };
        let Some(handle) = queue.diffuse_image_handles[slot.index].as_ref().cloned() else { continue };
        if let Some(registry) = registry.as_deref() {
            if slot.lease.as_ref().is_none_or(|lease| lease.handle.id() != handle.id()) {
                if let Some(mip_count) = assets.get(&handle).map(|image| image.texture_descriptor.mip_level_count) {
                    slot.lease = Some(registry.lease_scene_view(path, &handle, level, mip_count));
                }
            }
        }
        if slot.registered { continue; }
        let Some(image) = slot.image.upgrade() else { continue };
        Arc::make_mut(&mut consumers.slots).push((slot.index, image));
        Arc::make_mut(&mut consumers.handles).insert(path.clone(), handle);
        slot.registered = true;
        slot.added_consumer = true;
    }
    // Source revisions are published after this system. Never attribute an
    // incoming request's failure to the previous generation.
    if world.generation != 0 && world.generation == owner.revision {
        if let Some(error) = failed {
            state.failure = Some((world.generation, error));
            world.failed.store(world.generation, Ordering::Release);
            if let Some(decals) = decals.as_deref() { decals.failed.store(world.generation, Ordering::Release); }
            prewarm.failed.store(world.generation, Ordering::Release);
        }
    }
    // A world material can be demanded only once every scene-owned image has
    // a committed handle. Preserve startup bindings when the same material is
    // already part of the level; dynamic entries own only their own keys.
    if world.images_ready != world.image_revision && world.images.values().flatten().map(|(_, image)| image)
        .chain(world.extra_images.iter()).all(|image| !image.scene_owned || consumers.handles.contains_key(&image.path)) {
        let obsolete: Vec<_> = state.world_entries.iter()
            .filter(|name| !world.images.contains_key(*name)).cloned().collect();
        for name in obsolete {
            Arc::make_mut(&mut consumers.world_materials).remove(&name);
            state.world_entries.remove(&name);
        }
        for (name, images) in &world.images {
            if state.world_entries.contains(name) || !consumers.world_materials.contains_key(name) {
                let binding: Vec<_> = images.iter().map(|(_, image)| image.clone()).collect();
                let changed = consumers.world_materials.get(name).is_none_or(|old|
                    old.len() != binding.len() || old.iter().zip(&binding).any(|(a, b)| !Arc::ptr_eq(a, b)));
                if changed { Arc::make_mut(&mut consumers.world_materials).insert(name.clone(), binding); }
                state.world_entries.insert(name.clone());
            }
        }
        world.images_ready = world.image_revision;
    }
    for path in remove {
        let Some(slot) = state.slots.remove(&path) else { continue };
        if slot.added_consumer {
            Arc::make_mut(&mut consumers.slots).retain(|(index, image)| *index != slot.index || image.path != path);
            Arc::make_mut(&mut consumers.handles).remove(&path);
        }
        if slot.owned {
            if crate::rendering::bevy_backend::world_effects::trace_lifecycle() {
                eprintln!("FX_LIFECYCLE scene_image_retire={path} slot={}", slot.index);
            }
            queue.diffuse_image_paths[slot.index].clear();
            queue.diffuse_image_handles[slot.index] = None;
            queue.diffuse_mip_states[slot.index] = Default::default();
            queue.diffuse_image_attempted[slot.index] = false;
            worker.initial_failures.remove(&slot.index);
            queue.diffuse_image_last_visible[slot.index] = 0;
            queue.diffuse_image_demands[slot.index] = Default::default();
            state.free.push(slot.index);
        }
        drop(slot.lease);
        if let Some(registry) = registry.as_deref() { registry.retire_unused_scene_lease(&path); }
        prewarm.retired_signal.fetch_add(1, Ordering::Release);
    }
}
