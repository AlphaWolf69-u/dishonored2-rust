//! Associate admitted alpha-depth materials with the actual rendered instances.
use super::{material_loading::alpha::AlphaDepthMaterials, RenderModelSurface, SceneRenderQueue};
use crate::rendering::bevy_backend::alpha_depth_scene::{AlphaDepthMaterial, AlphaDepthSources};
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
) {
    sources.0.clear();
    let Some(scene) = scene.as_ref() else {
        return;
    };
    for instance in &scene.instances {
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
