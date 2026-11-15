//! Publish an independently evaluated MD6 pose to the scene and shadow paths.
use super::{
    apply_rest_pose, render_bounds, shadow_updates::{ModelShadowSource, ModelShadowSources, RestSkinning},
    view_geometry::Bounds, Aabb,
    RenderModelSurface, RenderSourcePlacements, SceneRenderQueue, ShaderBuffer,
};
use bevy::prelude::*;
use std::sync::{Arc, OnceLock};

pub(super) type Affine = [[f32; 4]; 3];

/// Bindings and last submitted pose belong to one independently animated placement.
#[derive(Default)]
pub(super) struct SkinPublication {
    bound: Option<(std::ops::Range<usize>, Handle<ShaderBuffer>)>,
    posed: Option<Arc<RestSkinning>>,
}

/// The caller computes the model-local palette; this publishes the same pose
/// to colour, shadow, mesh bounds and the placement's GPU palette.
#[allow(clippy::too_many_arguments)]
pub(super) fn publish_skin(
    placement_index: usize,
    model: &crate::bmd6::SkinModel,
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
    publish_target(placement, source, model.surfaces(), palette, publication, meshes, buffers, queue);
}

/// Existing physics callers already have these references before evaluating a
/// pose, so they do not perform a second lookup or pose an absent source.
#[allow(clippy::too_many_arguments)]
pub(super) fn publish_target(
    placement: &crate::entities::PlacedModel,
    source: &mut ModelShadowSource,
    surfaces: &[crate::bmd6::SurfaceInfo],
    palette: Vec<Affine>,
    publication: &mut SkinPublication,
    meshes: &mut Assets<Mesh>,
    buffers: &mut Assets<ShaderBuffer>,
    queue: &mut SceneRenderQueue,
) {
    if publication.bound.is_some() && source.skinning.is_some()
        && publication.posed.as_ref().is_some_and(|pose| pose.matrices == palette) { return; }
    static OBSERVE: OnceLock<bool> = OnceLock::new();
    let observe = *OBSERVE.get_or_init(|| std::env::var_os("DH2_DEBUG_SKIN_PUBLICATION").is_some_and(|value| value == "1"));
    let mut observation = observe.then(BoundsObservation::default);
    let gpu_palette = || ShaderBuffer::from(
        palette.iter().map(|matrix| matrix.map(Vec4::from_array)).collect::<Vec<_>>());
    let first_pose = publication.bound.is_none();
    let (instances, buffer) = publication.bound.get_or_insert_with(|| {
        (source.instances.clone(), buffers.add(gpu_palette()))
    });
    let mut bounds = Bounds::empty();
    for index in instances.clone() {
        let instance = &mut queue.instances[index];
        let handle = instance.mesh.clone();
        let Some(RenderModelSurface::Bmd6 { surface, .. }) = instance.source.map(|s| s.surface) else { continue; };
        let Some(info) = surfaces.get(surface) else { continue; };
        if info.skin_influences == 0 { continue; }
        let baked = instance.source.is_some_and(|s| s.world_baked);
        if let Some(mut mesh) = meshes.get_mut(&handle) {
            if apply_rest_pose(&mut mesh, info, &palette, info.skin_influences, placement, baked).is_ok() {
                if let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
                    let start = observe.then(crate::loading::thread_cycles).flatten();
                    let mut instance_bounds = Bounds::empty();
                    for &point in p {
                        let local = Vec3::from_array(point);
                        let world = if baked { local } else { instance.transform.transform_point(local) };
                        instance_bounds.add(world);
                        bounds.add(world);
                    }
                    instance.bounds = Aabb::from_min_max(instance_bounds.low, instance_bounds.high);
                    if let Some(observation) = &mut observation {
                        observation.capture(p, baked, &instance.transform, instance.bounds, start);
                    }
                }
            }
        }
    }
    if !first_pose {
        if let Some(mut gpu) = buffers.get_mut(&*buffer) {
            *gpu = gpu_palette();
        }
    }
    let posed = Arc::new(RestSkinning { palette: buffer.clone(), matrices: palette });
    let start = observe.then(crate::loading::thread_cycles).flatten();
    if bounds.points != 0 {
        source.bounds = game_bounds(Aabb::from_min_max(bounds.low, bounds.high));
    }
    publication.posed = Some(posed.clone());
    if let Some(observation) = observation {
        let gpu_bytes = buffers.get(&posed.palette).and_then(|buffer| buffer.data.as_deref());
        observation.report(source, &posed, first_pose, gpu_bytes, start);
    }
    source.skinning = Some(Ok(posed));
}

fn game_bounds(bounds: Aabb) -> [[f32; 3]; 2] {
    let a = super::view_to_game(Vec3::from(bounds.min()));
    let b = super::view_to_game(Vec3::from(bounds.max()));
    [std::array::from_fn(|i| a[i].min(b[i])), std::array::from_fn(|i| a[i].max(b[i]))]
}

#[derive(Default)]
struct BoundsObservation {
    world: Vec<Vec3>,
    transient_bytes: usize,
    direct_cycles: u64,
    copy_cycles: u64,
    differences: usize,
}

impl BoundsObservation {
    fn capture(&mut self, vertices: &[[f32; 3]], baked: bool, transform: &Transform,
        actual: Aabb, start: Option<u64>,
    ) {
        self.direct_cycles += elapsed_cycles(start);
        let copy_start = crate::loading::thread_cycles();
        let local: Vec<_> = vertices.iter().copied().map(Vec3::from_array).collect();
        let local_capacity = local.capacity();
        let world: Vec<_> = if baked { local } else {
            local.iter().map(|point| transform.transform_point(*point)).collect()
        };
        let expected = render_bounds(&world);
        self.transient_bytes += (local_capacity + if baked { 0 } else { world.capacity() })
            * std::mem::size_of::<Vec3>();
        self.world.extend(world);
        self.copy_cycles += elapsed_cycles(copy_start);
        self.differences += usize::from(expected.center != actual.center || expected.half_extents != actual.half_extents);
    }

    fn report(mut self, source: &ModelShadowSource, pose: &Arc<RestSkinning>, first_pose: bool,
        gpu_bytes: Option<&[u8]>, direct_start: Option<u64>,
    ) {
        self.direct_cycles += elapsed_cycles(direct_start);
        let start = crate::loading::thread_cycles();
        if !self.world.is_empty() && game_bounds(render_bounds(&self.world)) != source.bounds {
            self.differences += 1;
        }
        self.copy_cycles += elapsed_cycles(start);
        self.transient_bytes += self.world.capacity() * std::mem::size_of::<Vec3>();
        let palette_bytes = pose.matrices.len() * std::mem::size_of::<Affine>();
        let gpu_matches = gpu_bytes.is_some_and(|bytes| bytes.len() == palette_bytes
            && bytes.chunks_exact(4).zip(pose.matrices.iter().flatten().flatten()).all(
                |(encoded, value)| encoded == value.to_le_bytes()));
        eprintln!("SKIN_PUBLICATION placement={} vertices={} palette_bytes={palette_bytes} first_pose={first_pose} gpu_matches={gpu_matches} bounds_scratch_bytes={} bounds_differences={} shared_pose_leases={} direct_cycles={} copy_cycles={}",
            source.placement, self.world.len(), self.transient_bytes, self.differences,
            Arc::strong_count(pose), self.direct_cycles, self.copy_cycles);
    }
}

fn elapsed_cycles(start: Option<u64>) -> u64 {
    start.zip(crate::loading::thread_cycles()).map_or(0, |(start, end)| end.saturating_sub(start))
}
