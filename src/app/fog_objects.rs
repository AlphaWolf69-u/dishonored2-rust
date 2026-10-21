//! Whole-model alpha-fog selection, independent of material sharing and SH data.
use super::{
    scene_queue, shadow_updates::ModelShadowSources, DecalMaterial, MaterialHandle, ProbeMaterial,
    RenderModelSurface, SceneData, SceneRenderQueue, ShaderBuffer,
};
use crate::rendering::bevy_backend::{
    fog::FogFrame,
    fog_objects::{FogObjectFrame, FogObjectParameters},
};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

#[derive(SystemParam)]
pub(super) struct FogObjectSources<'w> {
    pub queue: Res<'w, SceneRenderQueue>,
    pub models: Res<'w, ModelShadowSources>,
    pub visible: Option<Res<'w, scene_queue::VisibleSceneInstances>>,
    pub buffers: ResMut<'w, Assets<ShaderBuffer>>,
}

pub(super) fn prepare(
    frame: &FogFrame,
    scene: &SceneData,
    sources: &FogObjectSources,
    probes: &Assets<ProbeMaterial>,
    decals: &Assets<DecalMaterial>,
    view_z: [f32; 4],
) -> Result<FogObjectFrame, String> {
    let normal_count = sources
        .queue
        .instances
        .iter()
        .filter(|i| i.probe_index & 0x80000000 == 0)
        .map(|i| i.probe_index as usize + 1)
        .max()
        .unwrap_or(1);
    let capacity = normal_count
        .checked_add(scene.world.visibility.len())
        .ok_or("alpha fog tag domain overflow")?;
    // control.w is a float in the installed material parameter block.
    if capacity > 0x1000000 {
        return Err("alpha fog tag domain exceeds exact float indices".into());
    }
    let mut result = FogObjectFrame {
        camera: frame.camera,
        revision: frame.revision,
        world_offset: normal_count as u32,
        capacity,
        updates: Vec::new(),
    };
    if frame.areas.is_empty() {
        return Ok(result);
    }
    let models: std::collections::HashMap<_, _> = sources
        .models
        .0
        .iter()
        .map(|m| (m.placement, m.bounds))
        .collect();
    let volumes: Vec<_> = frame.areas.iter().map(|v| v.bounds).collect();
    let mut error = None;
    scene_queue::for_each_visible_instance(
        &sources.queue,
        sources.visible.as_deref(),
        |instance| {
            if error.is_some() {
                return;
            }
            let enabled = match &instance.material {
                MaterialHandle::Probe(h) => probes
                    .get(h)
                    .is_some_and(|m| m.extension.surface.misc.w & 8 != 0),
                MaterialHandle::Decal(h) => decals
                    .get(h)
                    .is_some_and(|m| m.extension.surface.misc.w & 8 != 0),
                _ => false,
            };
            if !enabled {
                return;
            }
            let bounds = match instance.source.map(|s| s.surface) {
                Some(RenderModelSurface::World { matrix, .. }) => {
                    scene.world.visibility.get(matrix).map(|v| v.bounds)
                }
                Some(
                    RenderModelSurface::Bmodel { placement, .. }
                    | RenderModelSurface::Bmd6 { placement, .. },
                ) => models.get(&placement).copied(),
                None => None,
            };
            let Some(bounds) = bounds else {
                error = Some("alpha fog receiver has no whole-model bounds".to_owned());
                return;
            };
            let center = std::array::from_fn(|i| (bounds[0][i] + bounds[1][i]) * 0.5);
            let selected = crate::rendering::fog::selection::select(
                &volumes,
                frame.eye.to_array(),
                center,
                bounds,
                view_z,
            );
            let parameters = selected
                .map(|s| FogObjectParameters::selected(&frame.areas[s.index], s.depths))
                .unwrap_or_default();
            let tag = instance.probe_index;
            let index = if tag & 0x80000000 == 0 {
                tag
            } else {
                result.world_offset + (tag & 0x7fffffff)
            };
            result.updates.push((index, parameters));
        },
    );
    if let Some(error) = error {
        return Err(error);
    }
    result.updates.sort_unstable_by_key(|v| v.0);
    result.updates.dedup_by_key(|v| v.0);
    Ok(result)
}
