//! Publish an independently evaluated MD6 pose to the scene and shadow paths.
use super::{
    apply_rest_pose, render_bounds, shadow_updates::{ModelShadowSource, ModelShadowSources, RestSkinning},
    RenderModelSurface, RenderSourcePlacements, SceneRenderQueue, ShaderBuffer,
};
use bevy::prelude::*;
use std::sync::Arc;

pub(super) type Affine = [[f32; 4]; 3];

/// Bindings and last submitted pose belong to one independently animated placement.
#[derive(Default)]
pub(super) struct SkinPublication {
    bound: Option<(std::ops::Range<usize>, Handle<ShaderBuffer>)>,
    posed: Option<Vec<Affine>>,
}

/// The caller computes the model-local palette; this publishes the same pose
/// to colour, shadow, mesh bounds and the placement's GPU palette.
#[allow(clippy::too_many_arguments)]
pub(super) fn publish_skin(
    placement_index: usize,
    model: &crate::bmd6::ModelInfo,
    palette: Vec<Affine>,
    publication: &mut SkinPublication,
    meshes: &mut Assets<Mesh>,
    buffers: &mut Assets<ShaderBuffer>,
    queue: &mut SceneRenderQueue,
    sources: &mut ModelShadowSources,
    placements: &RenderSourcePlacements,
) {
    let Some(placement) = placements.0.get(placement_index) else { return; };
    let Some(source) = sources.0.iter_mut().find(|s| s.placement == placement_index) else { return; };
    publish_target(placement, source, model, palette, publication, meshes, buffers, queue);
}

/// Existing physics callers already have these references before evaluating a
/// pose, so they do not perform a second lookup or pose an absent source.
#[allow(clippy::too_many_arguments)]
pub(super) fn publish_target(
    placement: &crate::entities::PlacedModel,
    source: &mut ModelShadowSource,
    model: &crate::bmd6::ModelInfo,
    palette: Vec<Affine>,
    publication: &mut SkinPublication,
    meshes: &mut Assets<Mesh>,
    buffers: &mut Assets<ShaderBuffer>,
    queue: &mut SceneRenderQueue,
) {
    if publication.bound.is_some() && source.skinning.is_some()
        && publication.posed.as_deref() == Some(&palette[..]) { return; }
    publication.posed = Some(palette.clone());
    let (instances, buffer) = publication.bound.get_or_insert_with(|| {
        let gpu: Vec<[Vec4; 3]> = palette.iter().map(|m| m.map(Vec4::from_array)).collect();
        (source.instances.clone(), buffers.add(ShaderBuffer::from(gpu)))
    });
    let mut points = Vec::new();
    for index in instances.clone() {
        let instance = &mut queue.instances[index];
        let handle = instance.mesh.clone();
        let Some(RenderModelSurface::Bmd6 { surface, .. }) = instance.source.map(|s| s.surface) else { continue; };
        let Some(info) = model.surfaces.get(surface) else { continue; };
        if info.skin_influences == 0 { continue; }
        let baked = instance.source.is_some_and(|s| s.world_baked);
        if let Some(mut mesh) = meshes.get_mut(&handle) {
            if apply_rest_pose(&mut mesh, info, &palette, info.skin_influences, placement, baked).is_ok() {
                if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
                    let local: Vec<Vec3> = p.iter().map(|v| Vec3::from_array(*v)).collect();
                    let world: Vec<Vec3> = if baked { local } else {
                        local.iter().map(|v| instance.transform.transform_point(*v)).collect()
                    };
                    instance.bounds = render_bounds(&world);
                    points.extend(world);
                }
            }
        }
    }
    if let Some(mut gpu) = buffers.get_mut(&*buffer) {
        *gpu = ShaderBuffer::from(palette.iter().map(|m| m.map(Vec4::from_array)).collect::<Vec<_>>());
    }
    source.skinning = Some(Ok(Arc::new(RestSkinning { palette: buffer.clone(), matrices: palette })));
    if !points.is_empty() {
        let bounds = render_bounds(&points);
        let (min, max) = (Vec3::from(bounds.min()), Vec3::from(bounds.max()));
        let a = super::view_to_game(min);
        let b = super::view_to_game(max);
        source.bounds = [std::array::from_fn(|i| a[i].min(b[i])), std::array::from_fn(|i| a[i].max(b[i]))];
    }
}
