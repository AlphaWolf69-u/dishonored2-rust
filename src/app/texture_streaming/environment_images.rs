//! Environment consumers of shared scene image identities.
use super::*;

/// Streamable ocean environment consumers share the scene image slots.
#[derive(Resource, Default)]
pub(crate) struct OceanImageSlots(pub Vec<(String, usize)>);

pub(crate) fn publish_ocean(
    scene: Option<Res<SceneRenderQueue>>,
    slots: Option<Res<OceanImageSlots>>,
    registry: Option<Res<super::super::resident_images::ResidentImages>>,
    consumers: Option<ResMut<crate::rendering::bevy_backend::water_scene::WaterImageSources>>,
    mut changed: Local<Vec<(String, Handle<Image>)>>,
) {
    let (Some(scene), Some(slots), Some(mut consumers)) = (scene, slots, consumers) else { return; };
    changed.clear();
    for (path, slot) in &slots.0 {
        if scene.diffuse_mip_states.get(*slot).and_then(|state| state.resident).is_none() { continue; }
        let Some(handle) = scene.diffuse_image_handles.get(*slot).and_then(Option::as_ref) else { continue; };
        if consumers.ocean.get(path).is_none_or(|old| old.id() != handle.id()) {
            changed.push((path.clone(), handle.clone()));
        }
    }
    if changed.is_empty() { return; }
    let images = Arc::make_mut(&mut consumers.ocean);
    for (path, handle) in changed.drain(..) {
        if let Some(registry) = registry.as_deref() { registry.retire_replaced_inline(&path, &handle); }
        images.insert(path, handle);
    }
}
