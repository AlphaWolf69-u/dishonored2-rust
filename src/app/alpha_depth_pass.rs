//! Associate admitted alpha-depth materials with the actual rendered instances.
use super::{material_loading::alpha::AlphaDepthMaterials, RenderModelSurface, SceneRenderQueue};
use crate::rendering::bevy_backend::alpha_depth_scene::{AlphaDepthMaterial, AlphaDepthSources};
use std::sync::Arc;
use bevy::{
    prelude::*,
    render::{sync_world::MainEntity, Extract, ExtractSchedule, RenderApp},
};
pub(super) struct AlphaDepthPassPlugin;
impl Plugin for AlphaDepthPassPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AlphaDepthMaterials>();
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(ExtractSchedule, extract);
        }
    }
}
fn extract(
    mut sources: ResMut<AlphaDepthSources>,
    scene: Extract<Option<Res<SceneRenderQueue>>>,
    materials: Extract<Res<AlphaDepthMaterials>>,
    mut candidates: Local<(usize, usize, Vec<usize>)>,
) {
    sources.0.clear();
    let Some(scene) = scene.as_ref() else {
        return;
    };
    if scene.is_added() { *candidates = Default::default(); }
    // An instance's material never changes; instances are only appended and
    // the material table (blended materials only) is replaced whole. Index the
    // instances with a table entry once per (table, instance count) instead of
    // walking and hashing every scene instance each frame.
    let key = (Arc::as_ptr(&materials.0) as usize, scene.instances.len());
    if (candidates.0, candidates.1) != key {
        let indices: Vec<usize> = scene
            .instances
            .iter()
            .enumerate()
            .filter(|(_, instance)| materials.0.contains_key(&instance.material))
            .map(|(index, _)| index)
            .collect();
        info!(
            candidates = indices.len(),
            instances = scene.instances.len(),
            "Alpha-depth candidate instances indexed"
        );
        *candidates = (key.0, key.1, indices);
    }
    for &index in &candidates.2 {
        let instance = &scene.instances[index];
        let Some(entity) = instance.entity.filter(|_| instance.visible) else {
            continue;
        };
        let Some(routes) = materials.0.get(&instance.material) else {
            continue;
        };
        let source: Result<AlphaDepthMaterial, String> = match instance.source.map(|s| s.surface) {
            Some(RenderModelSurface::World { .. }) => routes[2].clone(),
            Some(RenderModelSurface::Bmodel { .. }) => routes[0].clone(),
            Some(RenderModelSurface::Bmd6 {
                skin_influences: 0, ..
            }) => routes[0].clone(),
            Some(RenderModelSurface::Bmd6 { .. }) if routes[0] == routes[1] => routes[0].clone(),
            Some(RenderModelSurface::Bmd6 { .. }) => {
                Err("alpha-depth optional mesh bank association unresolved".into())
            }
            None => Err("alpha-depth instance source unavailable".into()),
        };
        sources.0.insert(MainEntity::from(entity), source);
    }
}
