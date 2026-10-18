//! Scene queue.
use super::{
    env, render_model, surface_gpu, texture, tome, BTreeMap, DiffuseMipState, HashMap,
    MaterialHandle, PickupState, RenderInstance, RenderModelSurface, TomeRenderInstanceMap,
    TomeWorld,
};
use bevy::prelude::*;

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
    pub(super) native_material_images: HashMap<MaterialHandle, Vec<usize>>,
    pub(super) diffuse_image_handles: Vec<Option<Handle<Image>>>,
    /// Initial native whole-view alpha casters are independent of color visibility.
    pub(super) native_shadow_diffuse_slots: Vec<usize>,
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

/// FUN_140330c10 copies each prepared model's +0x1840/+0x1850 bounds
/// before invoking FUN_140a70710. This list is independent of forward-camera
/// visibility and of the per-surface shadow-program admission tests.
/// Initial boxes use the owned resource bounds and FUN_140235c80's transform.
/// The skinned-model bounds virtual (+0x80) is FUN_14025da70: resource+0xe0,
/// not an AABB reconstructed from the currently drawn/rest-pose vertices.
#[derive(Resource, Default)]
pub(super) struct NativeShadowModelBoundsSources(pub(super) Vec<NativeShadowModelBoundsSource>);

pub(super) struct NativeShadowModelBoundsSource {
    pub(super) placement: usize,
    pub(super) instances: std::ops::Range<usize>,
    pub(super) bounds: [[f32; 3]; 2],
}

impl NativeShadowModelBoundsSources {
    pub(super) fn active_bounds(
        &self,
        scene: &SceneRenderQueue,
        pickups: &PickupState,
    ) -> Result<Vec<[[f32; 3]; 2]>, String> {
        let mut bounds = Vec::with_capacity(self.0.len());
        for source in &self.0 {
            if source.instances.is_empty() || source.instances.end > scene.instances.len() {
                return Err("native shadow model bounds reference an invalid owner range".into());
            }
            if !source
                .bounds
                .iter()
                .flatten()
                .all(|value| value.is_finite())
                || (0..3).any(|axis| source.bounds[0][axis] > source.bounds[1][axis])
            {
                return Err("native shadow model owner has invalid prepared bounds".into());
            }
            let mut active = false;
            for instance in &scene.instances[source.instances.clone()] {
                let placement = match instance.source.as_ref().map(|value| &value.surface) {
                    Some(RenderModelSurface::Bmodel { placement, .. })
                    | Some(RenderModelSurface::Bmd6 { placement, .. }) => *placement,
                    _ => return Err("native shadow model bounds have no model owner".into()),
                };
                if placement != source.placement {
                    return Err("native shadow model bounds disagree with their model owner".into());
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
