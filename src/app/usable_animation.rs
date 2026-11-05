//! Per-instance AnimBasic playback for authored usable components.
//! Rig/mesh acquisition runs on the IO worker; immutable assets are shared
//! only while placements actually retain them.
use std::{collections::HashMap, sync::{Arc, Weak}};
use bevy::{prelude::*, tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once}};
use crate::{assets::animation_rig::BasicRig, gameplay::player::animation::{AnimBasicCommand, AnimBasicNode}};

struct Asset {
    rig: BasicRig,
    model: Arc<crate::bmd6::ModelInfo>,
}
struct Placement {
    actor: usize,
    model: String,
    source_index: Option<usize>,
    asset: Option<Arc<Asset>>,
    node: Option<AnimBasicNode>,
    publication: super::skin_publication::SkinPublication,
    requested: bool,
    failed: bool,
    command: Option<AnimBasicCommand>,
    initialized: bool,
    physics_owned: bool,
    animation_pose: Option<Vec<super::physics_skinning::Affine>>,
}

#[derive(Resource, Default)]
pub(super) struct UsableAnimations {
    placements: HashMap<usize, Placement>,
    lookup: HashMap<String, Weak<Asset>>,
    loading: HashMap<String, std::sync::Mutex<Task<Result<Arc<Asset>, String>>>>,
    completed: Vec<(String, Result<Arc<Asset>, String>)>,
    playing: Vec<Option<bool>>,
    initialized: bool,
}

impl UsableAnimations {
    /// Physics publishes until an overlap has an actual animation pose; then
    /// this owner publishes their combined pose exactly once per frame.
    pub(super) fn combines_physics(&self, placement: usize) -> bool {
        self.placements.get(&placement).is_some_and(|slot|
            slot.physics_owned && slot.asset.is_some() && slot.animation_pose.is_some())
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn advance(
    mut owner: ResMut<UsableAnimations>,
    interactions: Option<ResMut<super::interactions::Interactions>>,
    clock: Res<crate::gameplay::game_clock_runtime::ClockRuntime>,
    placements: Res<super::RenderSourcePlacements>,
    mut sources: ResMut<super::shadow_updates::ModelShadowSources>,
    mut queue: ResMut<super::SceneRenderQueue>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut buffers: ResMut<Assets<super::ShaderBuffer>>,
    machinery: Option<Res<super::machinery::AmbientMachinery>>,
) {
    let Some(mut interactions) = interactions else { return };
    if !owner.initialized {
        let physics = machinery.as_ref().map(|m| m.deformed_placements()).unwrap_or_default();
        let shadow_sources: HashMap<_, _> = sources.0.iter().enumerate().map(|(index, source)| (source.placement, index)).collect();
        for (index, placement) in placements.0.iter().enumerate() {
            let Some(&actor) = interactions.actor_by_entity.get(&placement.id) else { continue };
            if !placement.resource_path.ends_with(".md6") || !interactions.actors[actor].definition.steps.iter()
                .any(|state| state.animation.is_some() || state.transition_animation.is_some()) { continue; }
            owner.placements.insert(index, Placement { actor, model: placement.resource_path.clone(), source_index: shadow_sources.get(&index).copied(), asset: None,
                node: None, publication: Default::default(), requested: false, failed: false, command: None,
                initialized: false, physics_owned: physics.contains(&index), animation_pose: None });
        }
        owner.initialized = true;
    }
    // Interactive state changes post commands; they remain pending on the
    // placement until its worker acquisition finishes.
    for change in &mut interactions.pending {
        if let Some(command) = change.animation.take() {
            for slot in owner.placements.values_mut().filter(|slot| slot.actor == change.actor) {
                slot.command = Some(command);
            }
        }
    }
    let mut load = Vec::new();
    for (&index, slot) in &mut owner.placements {
        let visible = slot.source_index.and_then(|source| sources.0.get(source))
            .is_some_and(|source| queue.instances.get(source.instances.clone()).is_some_and(|instances| instances.iter().any(|i| i.visible)));
        if slot.asset.is_none() && !slot.requested && (visible || slot.command.is_some()) {
            slot.requested = true;
            load.push((index, slot.model.clone()));
        }
    }
    for (index, model) in load {
        let key = if interactions.catalog.has_overlays() { format!("{model}#{index}") } else { model.clone() };
        if let Some(asset) = owner.lookup.get(&key).and_then(Weak::upgrade) {
            let slot = owner.placements.get_mut(&index).unwrap();
            slot.node = Some(asset.rig.template.clone());
            slot.asset = Some(asset);
        } else if !owner.loading.contains_key(&key) {
            let catalog = interactions.catalog.clone();
            let path = model.clone();
            owner.loading.insert(key, std::sync::Mutex::new(IoTaskPool::get().spawn(async move {
                let rig = BasicRig::load(&catalog, &path)?;
                let model = crate::bmd6::ModelSources::new(&catalog).acquire(&catalog, &path)?;
                Ok(Arc::new(Asset { rig, model }))
            })));
        }
    }
    let owner = &mut *owner;
    owner.completed.clear();
    let completed = &mut owner.completed;
    owner.loading.retain(|key, task| {
        let task = task.get_mut().unwrap_or_else(|error| error.into_inner());
        if let Some(result) = block_on(poll_once(task)) {
            completed.push((key.clone(), result));
            false
        } else { true }
    });
    for (key, result) in owner.completed.drain(..) {
        match result {
            Ok(asset) => {
                owner.lookup.insert(key.clone(), Arc::downgrade(&asset));
                for (&index, slot) in &mut owner.placements {
                    let matches = if interactions.catalog.has_overlays() { key == format!("{}#{index}", slot.model) } else { key == slot.model };
                    if slot.requested && slot.asset.is_none() && matches {
                        slot.node = Some(asset.rig.template.clone());
                        slot.asset = Some(asset.clone());
                    }
                }
            }
            Err(error) => {
                bevy::log::warn!("Usable animation acquisition failed for {key}: {error}");
                for (&index, slot) in &mut owner.placements {
                    let matches = if interactions.catalog.has_overlays() { key == format!("{}#{index}", slot.model) } else { key == slot.model };
                    if slot.requested && matches { slot.failed = true; }
                }
            }
        }
    }
    if !clock.advanced() { return; }
    let Ok(clock) = clock.clock() else { return };
    // idEntity constructor 140874c50 gives time-group index 0; the lookup
    // table at 1422952d8 maps it to gameplay clock group 3.
    let dt = clock.delta(3).expect("five clock groups");
    owner.playing.resize(interactions.actors.len(), None);
    // Actors without an animation placement still complete their authored
    // minimum-duration transitions. A pending acquisition, however, must not
    // be mistaken for a finished animation.
    owner.playing.fill(Some(false));
    for slot in owner.placements.values() {
        if slot.asset.is_none() && !slot.failed {
            owner.playing[slot.actor] = None;
        }
    }
    let playing = &mut owner.playing;
    for (&index, slot) in &mut owner.placements {
        let (Some(asset), Some(node)) = (&slot.asset, &mut slot.node) else { continue };
        if !slot.initialized {
            if slot.command.is_none() {
                if let Some(state) = &interactions.actors[slot.actor].state { slot.command = state.initial().animation; }
            }
            slot.initialized = true;
        }
        let changed = slot.command.is_some() || !node.is_settled();
        if changed {
            node.update(dt, slot.command.take(), &asset.rig.animations);
        }
        if let Some(playing) = &mut playing[slot.actor] {
            *playing |= node.current_clip().is_some_and(|(_, _, playing)| playing);
        }
        if changed {
            let joints = match node.evaluate(&asset.rig.animations, &asset.rig.skeleton.rest) {
                Ok(Some(pose)) => pose.joints,
                Ok(None) => continue,
                Err(error) => { bevy::log::warn!("Usable animation evaluation: {error}"); continue; }
            };
            let matrices = match crate::edge_skeleton::pose_matrices(&joints, &asset.rig.skeleton.hierarchy, &asset.rig.root) {
                Ok(value) => value, Err(error) => { bevy::log::warn!("Usable skeleton pose: {error}"); continue; }
            };
            if slot.physics_owned {
                slot.animation_pose = Some(matrices);
            } else {
                let Some(inverse_bind) = asset.rig.skeleton.inverse_bind.get(..matrices.len()) else {
                    bevy::log::warn!("Usable skeleton has fewer inverse bind matrices than joints");
                    continue;
                };
                let palette = match crate::edge_skeleton::skin_palette(&matrices, inverse_bind) {
                    Ok(value) => value, Err(error) => { bevy::log::warn!("Usable skin palette: {error}"); continue; }
                };
                super::skin_publication::publish_skin(index, &asset.model, palette, &mut slot.publication,
                    &mut meshes, &mut buffers, &mut queue, &mut sources, &placements);
            }
        }
        if slot.physics_owned {
            let (Some(animation), Some(machinery)) = (&slot.animation_pose, machinery.as_deref()) else { continue };
            let Some(result) = machinery.animated_palette(index, animation) else { continue };
            match result {
                Ok(palette) => super::skin_publication::publish_skin(index, &asset.model, palette, &mut slot.publication,
                    &mut meshes, &mut buffers, &mut queue, &mut sources, &placements),
                Err(error) => {
                    slot.animation_pose = None;
                    bevy::log::warn!("Usable physics/animation pose: {error}");
                },
            }
        }
    }
    for (actor, is_playing) in playing.iter().copied().enumerate().filter_map(|(index, playing)| playing.map(|value| (index, value))) {
        let change = {
            let Some(state) = interactions.actors[actor].state.as_mut() else { continue };
            let change = state.tick(dt, is_playing);
            let animation = change.animation;
            let stop_effect = change.stop_effect;
            let path = change.start_effect.map(str::to_owned);
            super::interactions::PendingChange { actor, animation, stop_effect,
                start_effect: path.as_deref().map(|path| super::interactions::effect_spawn(state, path)) }
        };
        if change.animation.is_some() || change.stop_effect || change.start_effect.is_some() {
            interactions.pending.push(change);
        }
    }
    // A stationary pose lives in the published palette; the source rig and
    // clips need not remain resident solely because the actor once moved.
    for slot in owner.placements.values_mut() {
        let visible = slot.source_index.and_then(|source| sources.0.get(source))
            .is_some_and(|source| queue.instances.get(source.instances.clone()).is_some_and(|instances| instances.iter().any(|i| i.visible)));
        let playing = slot.node.as_ref().and_then(AnimBasicNode::current_clip).is_some_and(|(_, _, playing)| playing);
        if !visible && !playing && slot.command.is_none() && slot.asset.is_some() {
            slot.asset = None;
            slot.node = None;
            slot.animation_pose = None;
            slot.requested = false;
            // A later acquisition creates a fresh node. Reapply the current
            // authored idle state instead of leaving that node at its rest pose.
            slot.initialized = false;
        }
    }
}
