//! Visibility instances.
use super::{texture, tome, world_mesh, HashMap, HashSet, MaterialHandle, RenderInstance};
use bevy::prelude::*;

/// Flatten the authored Tome object -> BWM visibility record -> client
/// render-instance mapping once, preserving the order and multiplicity of
/// both original association tables.
#[derive(Debug, Default)]
pub(super) struct TomeRenderInstanceMap {
    pub(super) ranges: Vec<std::ops::Range<usize>>,
    pub(super) indices: Vec<usize>,
    pub(super) empty_bwm_groups: Vec<bool>,
    pub(super) spatial_query_instances: Vec<SpatialQueryRenderInstance>,
    pub(super) zero_metric_instances: HashSet<usize>,
}

#[derive(Debug)]
pub(super) struct SpatialQueryRenderInstance {
    pub(super) bounds: [[f32; 3]; 2],
    pub(super) render_indices: Vec<usize>,
}

impl TomeRenderInstanceMap {
    pub(super) fn from_world_mapping(
        world: &world_mesh::WorldMesh,
        bwm_visibility_render_instances: &[Vec<usize>],
    ) -> Self {
        let mut map = Self::from_authored_bwm_mapping(
            &world.tome_object_instances,
            bwm_visibility_render_instances,
        );
        for &index in &world.spatial_query_instances {
            let render_indices = bwm_visibility_render_instances[index].clone();
            map.zero_metric_instances
                .extend(render_indices.iter().copied());
            map.spatial_query_instances
                .push(SpatialQueryRenderInstance {
                    bounds: world.visibility[index].bounds,
                    render_indices,
                });
        }
        map
    }

    pub(super) fn from_authored_bwm_mapping(
        tome_object_bwm_instances: &[Vec<usize>],
        bwm_visibility_render_instances: &[Vec<usize>],
    ) -> Self {
        let index_count = tome_object_bwm_instances
            .iter()
            .flatten()
            .filter_map(|&bwm_index| bwm_visibility_render_instances.get(bwm_index))
            .map(Vec::len)
            .sum();
        let mut map = Self {
            ranges: Vec::with_capacity(tome_object_bwm_instances.len()),
            indices: Vec::with_capacity(index_count),
            empty_bwm_groups: Vec::with_capacity(tome_object_bwm_instances.len()),
            spatial_query_instances: Vec::new(),
            zero_metric_instances: HashSet::new(),
        };
        for bwm_indices in tome_object_bwm_instances {
            map.empty_bwm_groups.push(bwm_indices.is_empty());
            let start = map.indices.len();
            for &bwm_index in bwm_indices {
                if let Some(render_indices) = bwm_visibility_render_instances.get(bwm_index) {
                    map.indices.extend_from_slice(render_indices);
                }
            }
            map.ranges.push(start..map.indices.len());
        }
        map
    }

    pub(super) fn get(&self, tome_object: usize) -> &[usize] {
        self.ranges
            .get(tome_object)
            .map(|range| &self.indices[range.start..range.end])
            .unwrap_or_default()
    }

    /// Apply successful FUN_14037e1d0 query results to their BWM render
    /// instances. The original caller marks every instance in each accepted
    /// Tome object's serialized group; an empty group follows a separate
    /// authored-ID path and therefore marks no BWM instance here.
    pub(super) fn mark_visible_instances(
        &self,
        visible_tome_indices: &[u32],
        visible_render_instances: &mut [bool],
    ) -> Result<(), String> {
        for &tome_object in visible_tome_indices {
            if tome_object as usize >= self.ranges.len() {
                return Err("visible Tome index exceeds render-instance mapping".into());
            }
            for &render_index in self.get(tome_object as usize) {
                let Some(visible) = visible_render_instances.get_mut(render_index) else {
                    return Err("Tome render-instance mapping exceeds visibility output".into());
                };
                *visible = true;
            }
        }
        Ok(())
    }

    /// FUN_14037ef90 overwrites the separate +0x418 BWM list's visibility
    /// after worker/state merge, and clears both of those instances' metrics.
    pub(super) fn apply_spatial_queries(
        &self,
        state: Option<&tome::RenderQueryState>,
        visible_render_instances: &mut [bool],
    ) -> Result<usize, String> {
        let mut accepted = 0;
        // Original query descriptor setup precedes all spatial-instance tests;
        // do not rebuild its eight finite frustum corners for every object.
        let query = if self.spatial_query_instances.is_empty() {
            None
        } else {
            state.map(tome::RenderAabbQuery::from_state).transpose()?
        };
        for instance in &self.spatial_query_instances {
            let visible = match &query {
                Some(query) => query.visible(instance.bounds)?,
                None => true,
            };
            accepted += usize::from(visible);
            for &index in &instance.render_indices {
                let target = visible_render_instances
                    .get_mut(index)
                    .ok_or("Tome spatial-instance mapping exceeds visibility output")?;
                *target = visible;
            }
        }
        Ok(accepted)
    }

    /// FUN_14037e1d0 emits IDs only for an empty authored BWM group, not
    /// for a nonempty group whose LOD surfaces are absent from the client.
    /// FUN_14037ef90 appends the job lists and sorts unsigned IDs; duplicates
    /// remain. Resolving these IDs to render entities is still unsupported.
    pub(super) fn visible_authored_ids(
        &self,
        bytes: &[u8],
        visible_tome_indices: &[u32],
    ) -> Result<Vec<u32>, String> {
        let mut ids = Vec::new();
        for &index in visible_tome_indices {
            let empty = self
                .empty_bwm_groups
                .get(index as usize)
                .ok_or("visible Tome index exceeds authored BWM group mapping")?;
            if *empty {
                ids.extend(tome::object_ids_for_index(bytes, index)?);
            }
        }
        ids.sort_unstable();
        Ok(ids)
    }

    /// Forward the original accepted Tome indices and their projected-area
    /// outputs to the diffuse images on the associated static BWM instances.
    pub(super) fn submit_texture_demands(
        &self,
        visible_tome_indices: &[u32],
        area_metrics: &[Option<f32>],
        render_instances: &[RenderInstance],
        image_demands: &[texture::ProjectedAreaDemand],
    ) -> Result<usize, String> {
        self.submit_material_texture_demands(
            visible_tome_indices,
            area_metrics,
            render_instances,
            image_demands,
            &HashMap::new(),
        )
    }

    pub(super) fn submit_material_texture_demands(
        &self,
        visible_tome_indices: &[u32],
        area_metrics: &[Option<f32>],
        render_instances: &[RenderInstance],
        image_demands: &[texture::ProjectedAreaDemand],
        bindings: &HashMap<MaterialHandle, Vec<usize>>,
    ) -> Result<usize, String> {
        if visible_tome_indices.len() != area_metrics.len() {
            return Err("Tome visibility indices and area outputs have different lengths".into());
        }
        let mut submitted = 0;
        for (&tome_object, area_metric) in visible_tome_indices.iter().zip(area_metrics) {
            if tome_object as usize >= self.ranges.len() {
                return Err("visible Tome index exceeds render-instance mapping".into());
            }
            let Some(area_metric) = area_metric else {
                continue;
            };
            for &render_index in self.get(tome_object as usize) {
                let Some(instance) = render_instances.get(render_index) else {
                    return Err("Tome render-instance mapping exceeds scene render queue".into());
                };
                let primary = instance.diffuse_image_slot.into_iter().collect::<Vec<_>>();
                let slots = bindings.get(&instance.material).unwrap_or(&primary);
                for &image_slot in slots {
                    let Some(demand) = image_demands.get(image_slot) else {
                        return Err("render instance references an unknown material image".into());
                    };
                    // Spatial suppression applies to every shader image role.
                    demand.submit(if self.zero_metric_instances.contains(&render_index) {
                        0.0
                    } else {
                        *area_metric
                    });
                    submitted += 1;
                }
            }
        }
        Ok(submitted)
    }
}
