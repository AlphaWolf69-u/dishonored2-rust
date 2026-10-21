//! Scene queue.
use super::{
    env, render_model, surface_gpu, texture, tome, BTreeMap, DiffuseMipState, HashMap,
    MaterialHandle, PickupState, RenderInstance, RenderModelSurface, TomeRenderInstanceMap,
    TomeWorld,
};
use bevy::prelude::*;

/// Forward visibility's ordered instance indices, published before streaming.
/// Shadow-only image requests remain independent of this color-view list.
#[derive(Resource, Default)]
pub(super) struct VisibleSceneInstances {
    instance_buffer: usize,
    count: usize,
    pub(super) indices: Vec<usize>,
}

impl VisibleSceneInstances {
    pub(super) fn begin(&mut self, scene: &SceneRenderQueue) {
        self.instance_buffer = scene.instances.as_ptr() as usize;
        self.count = scene.instances.len();
        self.indices.clear();
    }
}

pub(super) fn for_each_visible_instance(
    scene: &SceneRenderQueue,
    selection: Option<&VisibleSceneInstances>,
    mut visit: impl FnMut(&RenderInstance),
) {
    if let Some(selection) = selection.filter(|selection| {
        selection.instance_buffer == scene.instances.as_ptr() as usize
            && selection.count == scene.instances.len()
    }) {
        for &index in &selection.indices {
            let instance = &scene.instances[index];
            // A pickup may have been removed after forward visibility ran.
            if instance.visible {
                visit(instance);
            }
        }
    } else {
        for instance in &scene.instances {
            if instance.visible {
                visit(instance);
            }
        }
    }
}

/// A producer reads one immutable scene snapshot and publishes its texture
/// requests through deferred commands. World and model preparation can then
/// run together instead of contending over the whole scene resource.
pub(super) struct ShadowTextureRequests<'a> {
    scene: &'a SceneRenderQueue,
    slots: Vec<usize>,
}

impl<'a> ShadowTextureRequests<'a> {
    pub(super) fn new(scene: &'a SceneRenderQueue) -> Self {
        Self {
            scene,
            slots: Vec::new(),
        }
    }

    pub(super) fn request_shadow_diffuse(&mut self, slot: usize) {
        self.slots.push(slot);
    }

    pub(super) fn into_slots(self) -> Vec<usize> {
        self.slots
    }

    pub(super) fn append_slots(&mut self, slots: Vec<usize>) {
        self.slots.extend(slots);
    }

    pub(super) fn submit(self, commands: &mut Commands) {
        if self.slots.is_empty() {
            return;
        }
        commands.queue(move |world: &mut World| {
            let mut scene = world.resource_mut::<SceneRenderQueue>();
            for slot in self.slots {
                scene.request_shadow_diffuse(slot);
            }
        });
    }
}

impl std::ops::Deref for ShadowTextureRequests<'_> {
    type Target = SceneRenderQueue;
    fn deref(&self) -> &Self::Target {
        self.scene
    }
}

#[derive(Resource, Default)]
pub(super) struct SceneRenderQueue {
    pub(super) instances: Vec<RenderInstance>,
    pub(super) active: usize,
    /// Renderer +0x2060 / slice task +0x68 ownership condition. Original
    /// 23a98b calls factory295690 then stores its BWM owner at23a99e;
    /// factory allocates even an empty model. Publish only after actual BWM
    /// construction below, never from Tome/depth availability or mesh count.
    pub(super) static_world_model_present: bool,
    pub(super) tome_render_instances: TomeRenderInstanceMap,
    // FUN_14037ef90's sorted authored-ID output. Its render-entity consumer
    // is still unverified; never equate these IDs with unmatched BWM records.
    pub(super) visible_authored_ids: Vec<u32>,
    // FUN_14037ef90 -> FUN_140a73580/FUN_140a70570 light-query inputs.
    pub(super) visible_cell_indices: Vec<u32>,
    pub(super) visible_cell_mask: Vec<u32>,
    pub(super) light_query_state: Option<tome::RenderQueryState>,
    /// The current merged ordinary query consumed by native receiver/caster
    /// preparation (FUN_140330c10 -> FUN_140a707e0). Preserve its Tome-index
    /// domain and original depth, separately from color-instance mapping and
    /// the light-only diagnostic that deliberately removes the depth grid.
    pub(super) shadow_ordinary_query: Option<tome::RenderQueryObjectOutput>,
    pub(super) visibility_suppression_groups: Vec<RenderSuppressionGroup>,
    pub(super) trace_bwm_lod: bool,
    pub(super) world_lods_by_render_instance: Vec<Option<usize>>,
    pub(super) last_bwm_lod_visibility_report: Option<BwmLodVisibilityReport>,
    pub(super) diffuse_image_demands: Vec<texture::ProjectedAreaDemand>,
    pub(super) diffuse_image_paths: Vec<String>,
    pub(super) diffuse_image_materials: Vec<Vec<(MaterialHandle, surface_gpu::ImageRole)>>,
    pub(super) material_images: HashMap<MaterialHandle, Vec<usize>>,
    /// Preserve both stageSort and stageSortAdjust for the game's pass lists.
    pub(super) material_stage_sorts: HashMap<MaterialHandle, Result<[u32; 2], String>>,
    pub(super) diffuse_image_handles: Vec<Option<Handle<Image>>>,
    /// Initial native whole-view alpha casters are independent of color visibility.
    pub(super) shadow_diffuse_slots: Vec<usize>,
    pub(super) shadow_diffuse_membership: Vec<bool>,
    pub(super) diffuse_image_pending: Vec<Option<Handle<Image>>>,
    pub(super) diffuse_mip_states: Vec<DiffuseMipState>,
    pub(super) diffuse_image_attempted: Vec<bool>,
    pub(super) diffuse_image_last_visible: Vec<u64>,
    pub(super) texture_stream_frame: u64,
    pub(super) texture_viewport_pixels: u32,
    pub(super) diffuse_mip_upgrades: u64,
    pub(super) preferred_mip: Option<u8>,
}

impl SceneRenderQueue {
    /// Preserve first-request order without scanning every previously requested
    /// texture for each caster in every cascade and local-light side.
    pub(super) fn request_shadow_diffuse(&mut self, slot: usize) {
        if self.shadow_diffuse_membership.len() != self.diffuse_image_paths.len() {
            self.shadow_diffuse_membership = vec![false; self.diffuse_image_paths.len()];
            for &old in &self.shadow_diffuse_slots {
                if let Some(member) = self.shadow_diffuse_membership.get_mut(old) {
                    *member = true;
                }
            }
        }
        if let Some(member) = self.shadow_diffuse_membership.get_mut(slot) {
            if !std::mem::replace(member, true) {
                self.shadow_diffuse_slots.push(slot);
            }
        } else if !self.shadow_diffuse_slots.contains(&slot) {
            // Keep the existing error path and request order for malformed IDs.
            self.shadow_diffuse_slots.push(slot);
        }
    }

    pub(super) fn publish_shadow_ordinary_query(&mut self, output: &tome::RenderQueryObjectOutput) {
        self.shadow_ordinary_query = Some(output.clone());
    }

    pub(super) fn publish_light_query_outputs(
        &mut self,
        world: &TomeWorld,
        output: &tome::RenderQueryObjectOutput,
    ) -> Result<(), String> {
        let ids = self
            .tome_render_instances
            .visible_authored_ids(&world.bytes, output.indices())?;
        let mask =
            tome::cell_visibility_mask(world.query_data.cell_count(), output.cell_indices())?;
        self.visible_authored_ids = ids;
        self.visible_cell_indices.clear();
        self.visible_cell_indices
            .extend_from_slice(output.cell_indices());
        self.visible_cell_mask = mask;
        self.light_query_state = output.state.clone();
        // Diagnostic A/B keeps traversal identical and selects the existing
        // no-grid descriptor path only for downstream model/light queries.
        if env::var_os("DH2_PERF_PROFILE").is_some()
            && env::var_os("DH2_PERF_WITHOUT_QUERY_DEPTH").is_some()
        {
            if let Some(state) = self.light_query_state.as_mut() {
                state.depth_grid = None;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_index_handles_removed_pickups_and_replaced_scene_buffers() {
        let mut scene = SceneRenderQueue::default();
        for index in 0..6 {
            scene.instances.push(RenderInstance {
                source: None,
                mesh: Handle::default(),
                material: MaterialHandle::Probe(Handle::default()),
                diffuse_image_slot: None,
                probe_index: index,
                transform: Transform::IDENTITY,
                bounds: bevy::camera::primitives::Aabb::from_min_max(Vec3::ZERO, Vec3::ONE),
                pickup_id: None,
                entity: None,
                visible: index == 1 || index == 4,
            });
        }
        let mut selection = VisibleSceneInstances::default();
        selection.begin(&scene);
        selection.indices.extend([1, 4]);
        let collect = |scene: &SceneRenderQueue, selection: Option<&VisibleSceneInstances>| {
            let mut result = Vec::new();
            for_each_visible_instance(scene, selection, |instance| {
                result.push(instance.probe_index)
            });
            result
        };
        assert_eq!(collect(&scene, Some(&selection)), collect(&scene, None));
        scene.instances[1].visible = false;
        assert_eq!(collect(&scene, Some(&selection)), vec![4]);
        // Change the domain while retaining the old selection: use the actual
        // current visibility until the new scene's collector publishes indices.
        scene.instances.pop();
        scene.instances[2].visible = true;
        assert_eq!(collect(&scene, Some(&selection)), vec![2, 4]);
    }

    #[test]
    fn independent_shadow_producers_publish_ordered_requests_at_the_deferred_boundary() {
        let mut world = World::new();
        let mut scene = SceneRenderQueue::default();
        scene.diffuse_image_paths.resize(4, String::new());
        world.insert_resource(scene);
        let mut deferred = bevy::ecs::world::CommandQueue::default();
        {
            let scene = world.resource::<SceneRenderQueue>();
            let mut world_requests = ShadowTextureRequests::new(scene);
            let mut model_requests = ShadowTextureRequests::new(scene);
            world_requests.request_shadow_diffuse(2);
            world_requests.request_shadow_diffuse(1);
            model_requests.request_shadow_diffuse(1);
            model_requests.request_shadow_diffuse(3);
            let mut commands = Commands::new(&mut deferred, &world);
            world_requests.submit(&mut commands);
            model_requests.submit(&mut commands);
            assert!(scene.shadow_diffuse_slots.is_empty());
        }
        deferred.apply(&mut world);
        assert_eq!(
            world
                .resource::<SceneRenderQueue>()
                .shadow_diffuse_slots,
            [2, 1, 3]
        );
    }

    #[test]
    fn shadow_texture_requests_keep_first_request_order_and_rebuild_membership() {
        let mut queue = SceneRenderQueue::default();
        queue.diffuse_image_paths.resize(4, String::new());
        for slot in [2, 1, 2, 0, 1, 99, 99] {
            queue.request_shadow_diffuse(slot);
        }
        assert_eq!(queue.shadow_diffuse_slots, [2, 1, 0, 99]);
        queue.diffuse_image_paths.resize(6, String::new());
        for slot in [2, 5, 1, 5] {
            queue.request_shadow_diffuse(slot);
        }
        assert_eq!(queue.shadow_diffuse_slots, [2, 1, 0, 99, 5]);
    }
}

/// FUN_140330c10 copies each prepared model's +0x1840/+0x1850 bounds
/// before invoking FUN_140a70710. This list is independent of forward-camera
/// visibility and of the per-surface shadow-program admission tests.
/// Initial boxes use the owned resource bounds and FUN_140235c80's transform.
/// The skinned-model bounds virtual (+0x80) is FUN_14025da70: resource+0xe0,
/// not an AABB reconstructed from the currently drawn/rest-pose vertices.
#[derive(Resource, Default)]
pub(super) struct ShadowModelBoundsSources(pub(super) Vec<ShadowModelBoundsSource>);

pub(super) struct ShadowModelBoundsSource {
    pub(super) placement: usize,
    pub(super) instances: std::ops::Range<usize>,
    pub(super) bounds: [[f32; 3]; 2],
}

impl ShadowModelBoundsSources {
    pub(super) fn active_bounds(
        &self,
        scene: &SceneRenderQueue,
        pickups: &PickupState,
    ) -> Result<Vec<[[f32; 3]; 2]>, String> {
        let mut bounds = Vec::with_capacity(self.0.len());
        for source in &self.0 {
            if source.instances.is_empty() || source.instances.end > scene.instances.len() {
                return Err("shadow model bounds reference an invalid owner range".into());
            }
            if !source
                .bounds
                .iter()
                .flatten()
                .all(|value| value.is_finite())
                || (0..3).any(|axis| source.bounds[0][axis] > source.bounds[1][axis])
            {
                return Err("shadow model owner has invalid prepared bounds".into());
            }
            let mut active = false;
            for instance in &scene.instances[source.instances.clone()] {
                let placement = match instance.source.as_ref().map(|value| &value.surface) {
                    Some(RenderModelSurface::Bmodel { placement, .. })
                    | Some(RenderModelSurface::Bmd6 { placement, .. }) => *placement,
                    _ => return Err("shadow model bounds have no model owner".into()),
                };
                if placement != source.placement {
                    return Err("shadow model bounds disagree with their model owner".into());
                }
                active |= instance
                    .pickup_id
                    .as_deref()
                    .is_none_or(|id| pickups.world.pickup_available(id));
            }
            if active {
                bounds.push(source.bounds);
            }
        }
        Ok(bounds)
    }
}

#[derive(Resource, Default)]
pub(super) struct PlacedModelQueries(pub(super) Vec<PlacedModelQueryGroup>);

pub(super) struct PlacedModelQueryGroup {
    pub(super) indices: std::ops::Range<usize>,
    pub(super) input: render_model::ModelQueryInput,
}

pub(super) struct RenderSuppressionGroup {
    pub(super) parent: Vec<usize>,
    pub(super) children: Vec<usize>,
}

impl RenderSuppressionGroup {
    pub(super) fn apply_all(groups: &[Self], visible: &mut [bool]) {
        for group in groups {
            if group
                .parent
                .iter()
                .any(|&index| visible.get(index) == Some(&true))
            {
                for &index in &group.children {
                    if let Some(value) = visible.get_mut(index) {
                        *value = false;
                    }
                }
            }
        }
    }
}

pub(super) type BwmLodVisibilityCounts = BTreeMap<usize, (usize, usize)>;
pub(super) type BwmLodVisibilityReport = (BwmLodVisibilityCounts, BwmLodVisibilityCounts);

/// Count world render instances by authored BWM LOD tag. Values are
/// `(total instances, visible instances)`; `None` tags are non-world draws.
pub(super) fn bwm_lod_visibility_counts(
    lod_by_render_instance: &[Option<usize>],
    visible: &[bool],
) -> BwmLodVisibilityCounts {
    let mut counts = BTreeMap::<usize, (usize, usize)>::new();
    for (lod, &is_visible) in lod_by_render_instance.iter().zip(visible) {
        if let Some(lod) = lod {
            let entry = counts.entry(*lod).or_default();
            entry.0 += 1;
            entry.1 += usize::from(is_visible);
        }
    }
    counts
}

#[cfg(test)]
mod bwm_lod_visibility_tests {
    use super::bwm_lod_visibility_counts;
    use std::collections::BTreeMap;

    #[test]
    pub(super) fn counts_authored_lod_instances_before_and_after_suppression() {
        let lod_by_instance = [Some(0), Some(0), Some(1), None, Some(1)];
        let before_suppression = [true, true, false, true, true];
        let after_suppression = [true, false, false, true, true];
        assert_eq!(
            bwm_lod_visibility_counts(&lod_by_instance, &before_suppression),
            BTreeMap::from([(0, (2, 2)), (1, (2, 1))])
        );
        assert_eq!(
            bwm_lod_visibility_counts(&lod_by_instance, &after_suppression),
            BTreeMap::from([(0, (2, 1)), (1, (2, 1))])
        );
    }
}
