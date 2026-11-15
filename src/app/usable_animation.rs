//! Per-instance AnimBasic playback for authored usable components.
//! Rig/mesh acquisition runs on the IO worker; immutable assets are shared
//! only while placements actually retain them.
use std::{collections::HashMap, sync::{Arc, Weak, atomic::Ordering}};
use bevy::{prelude::*, tasks::{IoTaskPool, Task, block_on, futures_lite::future::poll_once}};
use crate::{assets::animation_rig::BasicRig, gameplay::player::animation::{AnimBasicCommand, AnimBasicNode}};

struct Asset {
    rig: BasicRig,
    model: crate::bmd6::SkinModel,
}
impl Asset {
    fn load(catalog: &crate::archive::Catalog, path: &str, cancelled: impl Fn() -> bool) -> Result<Arc<Self>, String> {
        if cancelled() { return Err("usable animation acquisition cancelled".into()); }
        let declaration = crate::bmd6::declaration_path(path).ok_or("usable model is not a compiled MD6")?;
        let rig_name = declaration.strip_prefix("generated/decls/md6def/")
            .and_then(|name| name.strip_suffix(".decl"))
            .ok_or("usable MD6 declaration path is invalid")?;
        let rig = BasicRig::load_cancellable(catalog, rig_name, Some(&cancelled))?;
        if cancelled() { return Err("usable animation acquisition cancelled".into()); }
        let model = crate::bmd6::ModelSources::acquire_skin_for_rig(catalog, path, &rig, Some(&cancelled))?;
        Ok(Arc::new(Self { rig, model }))
    }
    fn instantiate(&self) -> AnimBasicNode { self.rig.template.clone() }
}

fn animated_actor(interactions: &super::interactions::Interactions,
    placement: &crate::entities::PlacedModel) -> Option<usize> {
    let &actor = interactions.actor_by_entity.get(&placement.id)?;
    (crate::bmd6::declaration_path(&placement.resource_path).is_some()
        && interactions.actors[actor].definition.steps.iter()
            .any(|state| state.animation.is_some() || state.transition_animation.is_some()))
        .then_some(actor)
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
    local_pose: Option<crate::gameplay::player::animation::Pose>,
    /// Dependency-driven startup sample, consumed by the ordinary publisher.
    prepared_pose: Option<Vec<super::physics_skinning::Affine>>,
}

impl Placement {
    fn new(actor: usize, model: String, physics_owned: bool) -> Self {
        Self { actor, model, source_index: None, asset: None, node: None,
            publication: Default::default(), requested: false, failed: false, command: None,
            initialized: false, physics_owned, animation_pose: None, local_pose: None, prepared_pose: None }
    }
    fn stop(&mut self) {
        self.asset = None;
        self.node = None;
        self.animation_pose = None;
        self.local_pose = None;
        self.prepared_pose = None;
        self.command = None;
        self.requested = false;
        self.failed = false;
        self.initialized = false;
    }
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
    /// 77b3b0 initializes the actual state's idle or held transition frame.
    /// An attachment needs that pose before body admission, even off screen.
    /// The same asset, node and sample become the live animation owner.
    pub(super) fn prepare_dependencies(catalog: &crate::archive::Catalog,
        interactions: Option<&super::interactions::Interactions>, models: &[crate::entities::PlacedModel],
        requests: &[(usize, String)], machinery: &super::machinery::AmbientMachinery,
        cancelled: impl Fn() -> bool) -> Result<Self, String> {
        let mut owner = Self::default();
        if requests.is_empty() { return Ok(owner); }
        let Some(interactions) = interactions else { return Ok(owner); };
        let physics = machinery.physics_placements();
        for &(index, _) in requests {
            if owner.placements.contains_key(&index) { continue; }
            if cancelled() { return Err("bound animation preparation cancelled".into()); }
            let placed = models.get(index).ok_or("bound animation parent placement is absent")?;
            let Some(actor) = animated_actor(interactions, placed) else { continue; };
            let key = if catalog.has_overlays() { format!("{}#{index}", placed.resource_path) } else { placed.resource_path.clone() };
            let asset = match owner.lookup.get(&key).and_then(Weak::upgrade) {
                Some(asset) => asset,
                None => {
                    let asset = Asset::load(catalog, &placed.resource_path, &cancelled)?;
                    owner.lookup.insert(key, Arc::downgrade(&asset));
                    asset
                },
            };
            let mut node = asset.instantiate();
            let command = interactions.actors[actor].state.as_ref().and_then(|state| state.initial().animation);
            node.update(0.0, command, &asset.rig.animations);
            let pose = node.evaluate(&asset.rig.animations, &asset.rig.skeleton.rest)?;
            // No active leaf retains the model's initialized skeleton pose,
            // as the normal publisher does; a failed animation is an error.
            let joints = pose.as_ref().map_or(asset.rig.skeleton.rest.as_slice(), |pose| pose.joints.as_slice());
            let matrices = crate::edge_skeleton::pose_matrices(joints, &asset.rig.skeleton.hierarchy, &asset.rig.root)?;
            let mut slot = Placement::new(actor, placed.resource_path.clone(), physics.contains(&index));
            slot.node = Some(node);
            slot.asset = Some(asset);
            slot.requested = true;
            slot.initialized = true;
            slot.prepared_pose = Some(matrices);
            owner.placements.insert(index, slot);
        }
        Ok(owner)
    }

    pub(super) fn prepared_rig(&self, placement: usize) -> Option<&Arc<crate::assets::animation_rig::Rig>> {
        Some(&self.placements.get(&placement)?.asset.as_ref()?.rig.rig)
    }

    pub(super) fn initial_joint_matrix(&self, placement: usize, joint: i16,
        skeleton: &Arc<crate::edge_skeleton::EdgeSkeleton>)
        -> Result<Option<super::physics_skinning::Affine>, String> {
        let Some(slot) = self.placements.get(&placement) else { return Ok(None); };
        let asset = slot.asset.as_ref().ok_or("bound animation asset was retired during preparation")?;
        if !Arc::ptr_eq(&asset.rig.skeleton, skeleton) { return Err("attachment and usable skeleton owners differ".into()); }
        Ok(usize::try_from(joint).ok().and_then(|joint| slot.prepared_pose.as_ref()?.get(joint)).copied())
    }
    /// 77b610/77bda0: stop the usable layer, retaining the last published
    /// palette with its existing placement rather than resetting its pose.
    pub(super) fn stop_actor(&mut self, actor: usize) {
        for slot in self.placements.values_mut().filter(|slot| slot.actor == actor) {
            slot.stop();
        }
        self.lookup.retain(|_, asset| asset.strong_count() != 0);
    }

    pub(super) fn stop_all(&mut self) {
        for slot in self.placements.values_mut() { slot.stop(); }
        self.lookup.clear();
    }
    fn request_asset(&mut self, key: String, path: String, interactions: &super::interactions::Interactions) {
        self.loading.entry(key).or_insert_with(|| {
            let catalog = interactions.catalog.clone();
            let closed = interactions.acquisitions_closed.clone();
            std::sync::Mutex::new(IoTaskPool::get().spawn(async move {
                Asset::load(&catalog, &path, || closed.load(Ordering::Acquire))
            }))
        });
    }

    pub(super) fn retire_acquisitions(&mut self) -> bool {
        self.loading.retain(|_, task| {
            let task = task.get_mut().unwrap_or_else(|error| error.into_inner());
            block_on(poll_once(task)).is_none()
        });
        self.loading.is_empty()
    }

    /// Physics publishes until an overlap has an actual animation pose; then
    /// this owner publishes their combined pose exactly once per frame.
    pub(super) fn local_pose(&self, placement: usize) -> Option<(&Arc<crate::edge_skeleton::EdgeSkeleton>, &crate::gameplay::player::animation::Pose)> {
        let slot = self.placements.get(&placement)?;
        Some((&slot.asset.as_ref()?.rig.skeleton, slot.local_pose.as_ref()?))
    }

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
    mut joint_frames: Option<ResMut<super::joint_frames::JointFrames>>,
) {
    let Some(mut interactions) = interactions else { return };
    if !owner.initialized {
        let physics = machinery.as_ref().map(|m| m.physics_placements()).unwrap_or_default();
        let shadow_sources: HashMap<_, _> = sources.0.iter().enumerate().map(|(index, source)| (source.placement, index)).collect();
        for (index, placement) in placements.0.iter().enumerate() {
            let Some(actor) = animated_actor(&interactions, placement) else { continue };
            let slot = owner.placements.entry(index).or_insert_with(|| Placement::new(actor, placement.resource_path.clone(), physics.contains(&index)));
            slot.source_index = shadow_sources.get(&index).copied();
        }
        owner.initialized = true;
    }
    let interactions = &mut *interactions;
    // Interactive state changes post commands; they remain pending on the
    // placement until its worker acquisition finishes.
    for change in &mut interactions.pending {
        if let Some(command) = change.animation.take() {
            if !interactions.actors[change.actor].started { continue; }
            for slot in owner.placements.values_mut().filter(|slot| slot.actor == change.actor) {
                slot.command = Some(command);
            }
        }
    }
    let mut load = Vec::new();
    for (&index, slot) in &mut owner.placements {
        if !interactions.actors[slot.actor].started { continue; }
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
            slot.node = Some(asset.instantiate());
            slot.asset = Some(asset);
        } else {
            owner.request_asset(key, model, &interactions);
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
                    if interactions.actors[slot.actor].started && slot.requested && slot.asset.is_none() && matches {
                        slot.node = Some(asset.instantiate());
                        slot.asset = Some(asset.clone());
                    }
                }
                if Arc::strong_count(&asset) == 1 { owner.lookup.remove(&key); }
            }
            Err(error) => {
                bevy::log::warn!("Usable animation acquisition failed for {key}: {error}");
                for (&index, slot) in &mut owner.placements {
                    let matches = if interactions.catalog.has_overlays() { key == format!("{}#{index}", slot.model) } else { key == slot.model };
                    if interactions.actors[slot.actor].started && slot.requested && matches { slot.failed = true; }
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
    for (index, actor) in interactions.actors.iter().enumerate() {
        if !actor.started { owner.playing[index] = None; }
    }
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
        let scene_active = machinery.as_deref().is_some_and(|m| m.scene_layer_active(index));
        let prepared = slot.prepared_pose.take();
        let changed = slot.command.is_some() || !node.is_settled() || (scene_active && slot.local_pose.is_none());
        if changed {
            node.update(dt, slot.command.take(), &asset.rig.animations);
        }
        if let Some(playing) = &mut playing[slot.actor] {
            *playing |= node.current_clip().is_some_and(|(_, _, playing)| playing);
        }
        if changed || prepared.is_some() {
            let matrices = if !changed {
                prepared.expect("unchanged startup pose is retained")
            } else {
                let pose = match node.evaluate(&asset.rig.animations, &asset.rig.skeleton.rest) {
                    Ok(Some(pose)) => pose,
                    Ok(None) => continue,
                    Err(error) => { bevy::log::warn!("Usable animation evaluation: {error}"); continue; }
                };
                if scene_active {
                    slot.local_pose = Some(pose);
                    continue;
                }
                slot.local_pose = None;
                let matrices = match crate::edge_skeleton::pose_matrices(&pose.joints, &asset.rig.skeleton.hierarchy, &asset.rig.root) {
                    Ok(value) => value, Err(error) => { bevy::log::warn!("Usable skeleton pose: {error}"); continue; }
                };
                matrices
            };
            if slot.physics_owned {
                slot.animation_pose = Some(matrices);
            } else {
                if let Some(frames) = joint_frames.as_deref_mut() { frames.publish(index, &matrices); }
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
        if slot.physics_owned && !scene_active {
            let (Some(animation), Some(machinery)) = (&slot.animation_pose, machinery.as_deref()) else { continue };
            let Some(result) = machinery.animated_palette(index, animation, joint_frames.as_deref_mut()) else { continue };
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
    for (&index, slot) in &mut owner.placements {
        let scene_active = machinery.as_deref().is_some_and(|m| m.scene_layer_active(index));
        let visible = slot.source_index.and_then(|source| sources.0.get(source))
            .is_some_and(|source| queue.instances.get(source.instances.clone()).is_some_and(|instances| instances.iter().any(|i| i.visible)));
        let playing = slot.node.as_ref().and_then(AnimBasicNode::current_clip).is_some_and(|(_, _, playing)| playing);
        if !scene_active && !visible && !playing && slot.command.is_none() && slot.asset.is_some() {
            slot.asset = None;
            slot.node = None;
            slot.animation_pose = None;
            slot.local_pose = None;
            slot.requested = false;
            // A later acquisition creates a fresh node. Reapply the current
            // authored idle state instead of leaving that node at its rest pose.
            slot.initialized = false;
        }
    }
}

/// Stop the actual clip and usable-mesh owners just after their archive
/// read, then acquire the same authored resources from those catalogs.
pub(super) fn observe_read_cancellation(base: &std::path::Path, model: &str)
    -> Result<(), Box<dyn std::error::Error>> {
    use crate::archive::Catalog;
    let control = Catalog::open(base)?;
    let expected = Asset::load(&control, model, || false)?;
    let item = expected.rig.template.items.first().ok_or("usable model has no AnimBasic clips")?;
    let source = expected.rig.alias(&item.alias).ok_or("usable clip alias is absent")?;
    let clip = crate::edge_animation::resource_path(source).ok_or("usable clip path is invalid")?
        .replace('\\', "/").to_ascii_lowercase();
    let mesh = expected.rig.model_reference().compiled_mesh().ok_or("usable mesh path is absent")?
        .replace('\\', "/").to_ascii_lowercase();

    let clip_catalog = Catalog::open(base)?;
    let clip_read = || crate::loading::catalog_read_recorded(clip_catalog.identity(), "anim", &clip);
    let clip_stopped = crate::edge_animation::acquire_cancellable(&clip_catalog, &clip, Some(&clip_read))
        .is_err_and(|error| error.contains("acquisition cancelled")) && clip_read();
    let decoded = crate::edge_animation::acquire(&clip_catalog, &clip)?;
    let reference = &expected.rig.animations[item.clip];
    let clip_retried = decoded.frame_count == reference.frame_count
        && decoded.frame_rate.to_bits() == reference.frame_rate.to_bits();

    let mesh_catalog = Catalog::open(base)?;
    let kind = mesh_catalog.kind(&mesh).ok_or("installed compiled mesh absent")?;
    let mesh_read = || crate::loading::catalog_read_recorded(mesh_catalog.identity(), kind, &mesh);
    let mesh_stopped = Asset::load(&mesh_catalog, model, &mesh_read)
        .is_err_and(|error| error.contains("acquisition cancelled")) && mesh_read();
    let restored = Asset::load(&mesh_catalog, model, || false)?;
    let original = expected.model.surfaces();
    let actual = restored.model.surfaces();
    let mesh_retried = original.len() == actual.len() && original.iter().zip(actual).all(|(a, b)| {
        a.positions == b.positions && a.packed_channels == b.packed_channels
            && a.skin_indices == b.skin_indices && a.skin_weights == b.skin_weights
            && a.matrix_offset == b.matrix_offset && a.skin_influences == b.skin_influences
    });
    eprintln!("USABLE_READ_CANCELLATION model={model} clip={clip} mesh={mesh} clip_stopped={clip_stopped} clip_retried={clip_retried} mesh_stopped={mesh_stopped} mesh_retried={mesh_retried}");
    Ok(())
}

/// Submit a real installed usable model without advancing any actor.
pub(super) fn observe_level_acquisition(world: &mut World) -> bool {
    if !world.contains_resource::<UsableAnimations>() { return false; }
    world.resource_scope(|world, mut owner: Mut<UsableAnimations>| {
        let (Some(interactions), Some(placements)) = (world.get_resource::<super::interactions::Interactions>(),
            world.get_resource::<super::RenderSourcePlacements>()) else { return false };
        let Some(placement) = placements.0.iter().find(|placement| animated_actor(interactions, placement).is_some())
            else { return false };
        owner.request_asset(placement.resource_path.clone(), placement.resource_path.clone(), interactions);
        true
    })
}


/// Observe the same acquisition/instance constructors for authored animated
/// root placements. No frames, commands, visibility or gameplay are simulated.
pub(super) fn observe_resources(catalog: &crate::archive::Catalog,
    interactions: &super::interactions::Interactions,
    placements: &[crate::entities::PlacedModel]) {
    let mut assets = HashMap::<String, Result<Arc<Asset>, String>>::new();
    let mut nodes = Vec::new();
    let mut failures = 0;
    for (index, placement) in placements.iter().enumerate() {
        if animated_actor(interactions, placement).is_none() { continue; }
        let path = &placement.resource_path;
        let key = if catalog.has_overlays() { format!("{path}#{index}") } else { path.clone() };
        let asset = assets.entry(key).or_insert_with(|| Asset::load(catalog, path, || false));
        match asset {
            Ok(asset) => {
                let node = asset.instantiate();
                eprintln!("ANIMBASIC_INSTANCE entity={} path={path} items={} signature={}",
                    placement.id, node.items.len(), blake3::hash(format!("{node:?}").as_bytes()));
                nodes.push(node);
            }
            Err(error) => { failures += 1; eprintln!("ANIMBASIC_INSTANCE_FAILURE path={path} error={error}"); }
        }
    }
    for (path, asset) in &assets {
        let Ok(asset) = asset else { continue; };
        let mut signature = blake3::Hasher::new();
        let mut vertices = 0;
        let mut indices = 0;
        let mut render_bytes = 0;
        for surface in asset.model.surfaces() {
            signature.update(&surface.vertex_count.to_le_bytes());
            signature.update(&surface.index_count.to_le_bytes());
            signature.update(&surface.matrix_offset.to_le_bytes());
            signature.update(&[surface.skin_influences]);
            for value in surface.positions.iter().flatten() {
                signature.update(&value.to_le_bytes());
            }
            for stream in &surface.packed_channels { signature.update(stream); }
            for stream in &surface.skin_indices { signature.update(stream); }
            for stream in &surface.skin_weights { signature.update(stream); }
            vertices += surface.positions.len();
            indices += surface.index_count as usize;
            render_bytes += surface.uvs.capacity() * std::mem::size_of::<[half::f16; 2]>()
                + surface.indices.capacity() * std::mem::size_of::<u16>();
        }
        eprintln!("ANIMBASIC_SKIN_SOURCE path={path} vertices={vertices} indices={indices} render_bytes={render_bytes} signature={}", signature.finalize());
    }
    let mut unique = std::collections::HashSet::new();
    let mut aliases = 0;
    let mut logical_payload_bytes = 0;
    for node in nodes.iter().chain(assets.values().filter_map(|asset|
        asset.as_ref().ok().map(|asset| &asset.rig.template))) {
        if unique.insert((node.items.as_ptr() as usize, node.items.len())) {
            aliases += node.items.len();
            logical_payload_bytes += 2 * std::mem::size_of::<usize>()
                + node.items.len() * std::mem::size_of::<crate::gameplay::player::animation::AnimBasicItem>()
                + node.items.iter().map(|item| item.alias.capacity()).sum::<usize>();
        }
    }
    eprintln!("ANIMBASIC_ITEM_STORAGE assets={} instances={} failures={failures} unique_lists={} aliases={aliases} logical_payload_bytes={logical_payload_bytes}",
        assets.len(), nodes.len(), unique.len());
}
