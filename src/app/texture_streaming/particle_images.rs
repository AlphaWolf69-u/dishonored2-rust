//! Particle consumers of the scene image owner; no independent read/upload.
use super::*;
use crate::rendering::bevy_backend::particle_images::SceneParticleImages;

pub(crate) fn publish(
    scene: Option<Res<SceneRenderQueue>>,
    mut consumers: ResMut<SceneParticleImages>,
    mut changed: Local<Vec<(Arc<str>, Handle<Image>)>>,
) {
    let Some(scene) = scene else { return; };
    changed.clear();
    for (slot, source) in consumers.slots.iter() {
        let Some(handle) = scene.diffuse_image_handles.get(*slot).and_then(Option::as_ref) else { continue; };
        if consumers.handles.get(&source.path).is_none_or(|old| old.id() != handle.id()) {
            changed.push((source.path.clone(), handle.clone()));
        }
    }
    if changed.is_empty() { return; }
    let handles = Arc::make_mut(&mut consumers.handles);
    for (path, handle) in changed.drain(..) { handles.insert(path, handle); }
}
