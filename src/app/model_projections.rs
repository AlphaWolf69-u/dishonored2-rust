//! Actual model-local projected-decal contexts. The shared FX projection
//! manager retains descriptions/materials; these rows retain only identity,
//! matrices and source surface masks (233a30/234ab0/235680/1c0a10).
use super::*;
pub(in crate::app) use crate::app::model_motion::ModelKey;
use crate::{
    gameplay::entity_lifetime::{EntityId, LiveEntities},
    rendering::decals::{MODEL_PROJECTION_LIMIT, ModelProjection},
};
use bevy::prelude::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
#[derive(Resource, Default)]
pub(in crate::app) struct ModelProjectionContexts {
    rows: HashMap<ModelKey, Arc<[ModelProjection]>>,
    dirty: HashSet<ModelKey>,
    retired: HashSet<ModelKey>,
    sources: Vec<(u32, u64, [[f32; 4]; 4], [[f32; 3]; 2])>,
    ranges: HashMap<usize, (std::ops::Range<usize>, usize)>,
    range_domain: (usize, usize),
}
impl ModelProjectionContexts {
    pub(in crate::app) fn dirty(&mut self, key: ModelKey) {
        self.dirty.insert(key);
    }
    pub(in crate::app) fn rows(&self, key: ModelKey) -> Option<&Arc<[ModelProjection]>> {
        self.rows.get(&key)
    }
    pub(in crate::app) fn remove(&mut self, key: ModelKey) {
        self.rows.remove(&key);
        self.dirty.remove(&key);
    }
    pub(in crate::app) fn retire(&mut self, key: ModelKey) {
        self.remove(key);
        self.retired.insert(key);
    }
}
struct ModelInput {
    model: [[f32; 4]; 4],
    surfaces: Vec<(usize, [[f32; 3]; 2])>,
}
type Projection = (
    u32,
    u64,
    crate::rendering::decals::CommittedDecal<Arc<crate::assets::decal::DecalMaterials<String>>>,
);
fn index_ranges(world: &World, state: &mut ModelProjectionContexts) {
    let Some(bounds) = world.get_resource::<super::super::ShadowModelBoundsSources>() else {
        return;
    };
    let domain = (bounds.0.as_ptr() as usize, bounds.0.len());
    if domain == state.range_domain {
        return;
    }
    state.range_domain = domain;
    state.ranges.clear();
    state.ranges.extend(
        bounds
            .0
            .iter()
            .enumerate()
            .map(|(index, source)| (source.placement, (source.instances.clone(), index))),
    );
}
fn input(
    world: &World,
    key: ModelKey,
    ranges: &HashMap<usize, (std::ops::Range<usize>, usize)>,
) -> Option<ModelInput> {
    match key {
        ModelKey::Placed(placement) => {
            let source = world
                .get_resource::<super::super::RenderSourcePlacements>()?
                .0
                .get(placement)?;
            let affine =
                crate::render_model::affine_rows(source.position, source.rotation, source.scale);
            let model = [affine[0], affine[1], affine[2], [0., 0., 0., 1.]];
            let queue = world.get_resource::<super::super::SceneRenderQueue>()?;
            let surfaces = queue
                .instances
                .get(ranges.get(&placement)?.0.clone())?
                .iter()
                .filter_map(|instance| {
                    let actual = match instance.source?.surface {
                        RenderModelSurface::Bmodel {
                            placement: actual,
                            surface,
                        }
                        | RenderModelSurface::Bmd6 {
                            placement: actual,
                            surface,
                            ..
                        } if actual == placement => surface,
                        _ => return None,
                    };
                    let low = Vec3::from(instance.bounds.center)
                        - Vec3::from(instance.bounds.half_extents);
                    let high = Vec3::from(instance.bounds.center)
                        + Vec3::from(instance.bounds.half_extents);
                    Some((actual, [[low.x, -high.z, low.y], [high.x, -low.z, high.y]]))
                })
                .collect();
            Some(ModelInput { model, surfaces })
        }
        ModelKey::Actor(id) => {
            let owner = world.get_resource::<super::super::world_models::WorldModels>()?;
            let (model, surfaces) = owner.projection_input(id)?;
            Some(ModelInput { model, surfaces })
        }
    }
}
fn mask(decal: &Projection, input: &ModelInput) -> u32 {
    let d = &decal.2.description;
    let volume = crate::rendering::bounds::OrientedBox {
        centre: std::array::from_fn(|axis| d.position[axis] - d.axes[2][axis] * d.scale[2] * 0.5),
        half_extent: [d.scale[0], d.scale[1], d.scale[2] * 0.5],
        axes: d.axes,
    };
    input.surfaces.iter().fold(0, |mask, (surface, bounds)| {
        if volume.overlaps(&crate::rendering::bounds::OrientedBox::from_aabb(*bounds)) {
            mask | 1u32.rotate_left(*surface as u32)
        } else {
            mask
        }
    })
}
fn refresh(
    world: &World,
    state: &mut ModelProjectionContexts,
    key: ModelKey,
    decals: &[Projection],
) -> Result<(), String> {
    if state.retired.contains(&key) {
        return Ok(());
    }
    let has_context = state.rows.get(&key).is_some_and(|rows| !rows.is_empty());
    // Host broadphase reuses the canonical current model bounds. Rejecting
    // disjoint AABBs avoids material/surface allocations for untouched models.
    if !has_context {
        let bounds = match key {
            ModelKey::Placed(placement) => state
                .ranges
                .get(&placement)
                .and_then(|(_, index)| {
                    world
                        .get_resource::<super::super::ShadowModelBoundsSources>()?
                        .0
                        .get(*index)
                })
                .map(|source| source.bounds),
            ModelKey::Actor(id) => super::super::world_models::bounds(world, id),
        };
        let Some(bounds) = bounds else { return Ok(()) };
        if !decals.iter().any(|decal| {
            (0..3).all(|axis| {
                bounds[0][axis] <= decal.2.projection.bounds[1][axis]
                    && decal.2.projection.bounds[0][axis] <= bounds[1][axis]
            })
        }) {
            return Ok(());
        }
    }
    let Some(input) = input(world, key, &state.ranges) else {
        state.remove(key);
        return Ok(());
    };
    let previous = state.rows.get(&key).cloned().unwrap_or_default();
    let mut rows = previous
        .iter()
        .filter(|row| {
            decals
                .iter()
                .any(|decal| decal.0 == row.slot && decal.1 == row.owner)
        })
        .cloned()
        .collect::<Vec<_>>();
    for row in &mut rows {
        let Some(decal) = decals.iter().find(|d| d.0 == row.slot && d.1 == row.owner) else {
            continue;
        };
        //235680 rebuilds evaluated rows from staging on model updates. Normal
        //registration is state0 (global projector); copied state1 preserves the
        //parent's local matrix. Both recheck source surface bounds (1c0a10).
        row.surfaces = mask(decal, &input);
        row.pending = false;
        if row.surfaces == 0 {
            continue;
        }
        if !row.inherited {
            row.local_to_decal =
                crate::render_shadow::multiply_rows(decal.2.world_to_decal, input.model);
        }
        row.place(input.model)?;
    }
    for decal in decals {
        if rows
            .iter()
            .any(|row| row.slot == decal.0 && row.owner == decal.1)
        {
            continue;
        }
        if rows.len() == MODEL_PROJECTION_LIMIT {
            break;
        }
        let surfaces = mask(decal, &input);
        if surfaces != 0 {
            rows.push(ModelProjection::new(
                decal.0,
                decal.1,
                decal.2.world_to_decal,
                input.model,
                surfaces,
            ));
        }
    }
    if rows.as_slice() != previous.as_ref() {
        if rows.is_empty() {
            state.rows.remove(&key);
        } else {
            state.rows.insert(key, Arc::from(rows));
        }
    }
    Ok(())
}
pub(in crate::app) fn sync(world: &mut World) {
    if let Err(error) = sync_inner(world) {
        error!("Model projection publication failed: {error}");
        crate::platform::client_diagnostics::record(&format!(
            "Model projection publication failed: {error}"
        ));
        world.write_message(AppExit::error());
    }
}
fn sync_inner(world: &mut World) -> Result<(), String> {
    if !world.contains_resource::<crate::rendering::bevy_backend::camera_scene::FxPublication>() {
        return Ok(());
    }
    world.resource_scope(
        |world, publication: Mut<crate::rendering::bevy_backend::camera_scene::FxPublication>| {
            let Some(decals) = publication.model_decals() else {
                return Ok(());
            };
            let mut state = world
                .remove_resource::<ModelProjectionContexts>()
                .unwrap_or_default();
            index_ranges(world, &mut state);
            let changed = state.sources.len() != decals.len()
                || state.sources.iter().zip(decals).any(|(old, new)| {
                    *old != (new.0, new.1, new.2.world_to_decal, new.2.projection.bounds)
                });
            if !changed && state.dirty.is_empty() {
                world.insert_resource(state);
                return Ok(());
            }
            let mut keys = std::mem::take(&mut state.dirty);
            if changed {
                keys.extend(state.rows.keys().copied());
            }
            if changed && !decals.is_empty() {
                if let Some(bounds) = world.get_resource::<super::super::ShadowModelBoundsSources>()
                {
                    keys.extend(
                        bounds
                            .0
                            .iter()
                            .map(|source| ModelKey::Placed(source.placement)),
                    );
                }
                if let Some(models) =
                    world.get_resource::<super::super::world_models::WorldModels>()
                {
                    keys.extend(models.projection_keys().map(ModelKey::Actor));
                }
            }
            let result = (|| {
                for key in keys.drain() {
                    refresh(world, &mut state, key, decals)?;
                }
                Ok(())
            })();
            state.dirty = keys;
            if changed {
                state.sources.clear();
                state.sources.extend(
                    decals
                        .iter()
                        .map(|d| (d.0, d.1, d.2.world_to_decal, d.2.projection.bounds)),
                );
            }
            world.insert_resource(state);
            result
        },
    )
}

pub(in crate::app) fn copy_parent_projection_contexts(
    world: &mut World,
    parent: EntityId,
    child: EntityId,
) -> Result<(), String> {
    let (name, _) = world
        .resource::<LiveEntities>()
        .get(parent)
        .ok_or("projection copy parent generation stale")?;
    let mut parents = world
        .get_resource::<super::super::RenderSourcePlacements>()
        .map(|p| {
            p.0.iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    p.id.eq_ignore_ascii_case(name)
                        .then_some(ModelKey::Placed(i))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if world
        .get_resource::<super::super::world_models::WorldModels>()
        .is_some_and(|models| models.projection_input(parent).is_some())
    {
        parents.push(ModelKey::Actor(parent));
    }
    let child_key = ModelKey::Actor(child);
    if super::super::world_models::frame(world, child).is_none() {
        return Ok(());
    }
    let decals = world
        .get_resource::<crate::rendering::bevy_backend::camera_scene::FxPublication>()
        .and_then(|p| p.model_decals())
        .unwrap_or(&[])
        .to_vec();
    let mut state = world
        .remove_resource::<ModelProjectionContexts>()
        .unwrap_or_default();
    index_ranges(world, &mut state);
    let result = (|| {
        for key in parents {
            refresh(world, &mut state, key, &decals)?;
            let rows = state.rows(key).cloned().unwrap_or_default();
            let mut child_rows = state
                .rows(child_key)
                .map_or_else(Vec::new, |rows| rows.to_vec());
            for row in rows.iter().filter(|row| row.surfaces != 0) {
                //234ab0 checks capacity before duplicate identity.
                if child_rows.len() == MODEL_PROJECTION_LIMIT {
                    break;
                }
                if child_rows
                    .iter()
                    .any(|old| old.slot == row.slot && old.owner == row.owner)
                {
                    continue;
                }
                let mut copied = row.clone();
                //234ab0 only stages state2->1. 235680/1c0a10 evaluate it
                //after the actual child frame has reached its render owner.
                copied.pending = true;
                copied.inherited = true;
                child_rows.push(copied);
            }
            if !child_rows.is_empty() {
                state.rows.insert(child_key, Arc::from(child_rows));
            }
        }
        state.dirty.insert(child_key);
        Ok(())
    })();
    world.insert_resource(state);
    result
}
