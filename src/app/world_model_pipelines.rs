//! bevy_backend: an uploaded mesh/material is not a compiled draw pipeline.
//! 1402be900/1402c9f00 select/cache the installed material programs; Bevy adds
//! view/layout specialization. Admit the real template only after that extra
//! host dependency is ready, using the same specialization owner as real draws.
use super::*;
use crate::rendering::bevy_backend::water_scene::WaterSceneFrame;
use bevy::{
    material::key::{ErasedMaterialPipelineKey, ErasedMeshPipelineKey},
    pbr::{MeshPipelineKey, PreparedMaterial, ViewKeyCache},
    render::{
        erased_render_asset::ErasedRenderAssets,
        mesh::RenderMesh,
        render_asset::RenderAssets,
        render_resource::*,
        sync_world::MainEntity,
        view::{ExtractedView, RetainedViewEntity, ViewTarget},
    },
};

#[derive(Clone)]
pub(in crate::app) struct Surface {
    pub mesh: bevy::asset::AssetId<Mesh>,
    pub material: bevy::asset::UntypedAssetId,
    pub skinned: bool,
}
#[derive(Clone, Default)]
pub(super) struct Admission {
    pub complete: Arc<AtomicBool>,
    pub error: Arc<Failure>,
}
struct Work {
    surface: Surface,
    base: Vec<CachedRenderPipelineId>,
    complete: Arc<AtomicBool>,
    error: Arc<Failure>,
}
#[derive(Default)]
pub(super) struct Failure {
    failed: AtomicBool,
    message: std::sync::Mutex<Option<String>>,
}
impl Failure {
    pub(super) fn message(&self) -> Option<String> {
        if !self.failed.load(Ordering::Acquire) {
            return None;
        }
        self.message.lock().unwrap().clone()
    }
    fn set(&self, message: String) {
        *self.message.lock().unwrap() = Some(message);
        self.failed.store(true, Ordering::Release);
    }
}
#[derive(Resource, Default)]
pub(super) struct Pending {
    sources: Weak<Vec<Upload>>,
    view: Option<(RetainedViewEntity, MeshPipelineKey, TextureFormat)>,
    work: Vec<Work>,
    remaining: HashMap<usize, usize>,
}

// Read-only condition avoids scheduling an exclusive render-world barrier once
// this roster/view has completed. It inspects views, never model surfaces.
pub(super) fn needed(
    uploads: Res<Uploads>,
    pending: Res<Pending>,
    water: Res<WaterSceneFrame>,
    keys: Res<ViewKeyCache>,
    views: Query<(&MainEntity, &ExtractedView, &ViewTarget)>,
) -> bool {
    let view = water.0.as_ref().and_then(|water| {
        views
            .iter()
            .find(|(main, _, _)| main.id() == water.camera)
            .and_then(|(_, view, target)| {
                keys.get(&view.retained_view_entity).map(|key| {
                    (
                        view.retained_view_entity,
                        *key,
                        target.main_texture_format(),
                    )
                })
            })
    });
    !pending.sources.ptr_eq(&Arc::downgrade(&uploads.0))
        || pending.view != view
        || (view.is_some() && !pending.work.is_empty())
}

pub(super) fn acknowledge(world: &mut World) {
    let camera = world
        .resource::<WaterSceneFrame>()
        .0
        .as_ref()
        .map(|frame| frame.camera);
    let view = camera.and_then(|camera| {
        let mut views = world.query::<(&MainEntity, &ExtractedView, &ViewTarget)>();
        views
            .iter(world)
            .find(|(main, _, _)| main.id() == camera)
            .and_then(|(_, view, target)| {
                world
                    .resource::<ViewKeyCache>()
                    .get(&view.retained_view_entity)
                    .map(|key| {
                        (
                            view.retained_view_entity,
                            *key,
                            target.main_texture_format(),
                        )
                    })
            })
    });
    let uploads = world.resource::<Uploads>().0.clone();
    world.resource_scope(|world, mut pending: Mut<Pending>| {
        let view_changed = pending.view != view;
        let changed = !pending.sources.ptr_eq(&Arc::downgrade(&uploads)) || view_changed;
        if changed {
            pending.work.clear();
            pending.remaining.clear();
            pending.sources = Arc::downgrade(&uploads);
            pending.view = view;
            for upload in uploads.iter() {
                // An unrelated model entering/leaving the roster cannot revoke
                // a completed certificate for this same view/layout/material.
                // Only an actual view/key/format change invalidates it.
                if !view_changed && upload.complete.load(Ordering::Acquire) {
                    continue;
                }
                upload
                    .complete
                    .store(upload.surfaces.is_empty(), Ordering::Release);
                pending.remaining.insert(
                    Arc::as_ptr(&upload.complete) as usize,
                    upload.surfaces.len(),
                );
                for surface in &upload.surfaces {
                    pending.work.push(Work {
                        surface: surface.clone(),
                        base: Vec::new(),
                        complete: upload.complete.clone(),
                        error: upload.error.clone(),
                    });
                }
            }
        }
        let Some((_, view_key, _)) = view else {
            return;
        };
        // No per-surface work, cloning or specialization once this exact
        // template roster/view has completed. On retirement the weak roster
        // changes and removes its pending work before asset handles can recycle.
        let mut index = 0;
        while index < pending.work.len() {
            let result = prepare(world, &mut pending.work[index], view_key);
            match result {
                Ok(false) => index += 1,
                result => {
                    let item = pending.work.swap_remove(index);
                    if let Err(error) = result {
                        item.error.set(error);
                    }
                    let remaining = pending
                        .remaining
                        .get_mut(&(Arc::as_ptr(&item.complete) as usize))
                        .unwrap();
                    *remaining -= 1;
                    if *remaining == 0 && !item.error.failed.load(Ordering::Acquire) {
                        item.complete.store(true, Ordering::Release);
                    }
                }
            }
        }
    });
}

fn ready(world: &World, id: CachedRenderPipelineId) -> Result<bool, String> {
    match world
        .resource::<PipelineCache>()
        .get_render_pipeline_state(id)
    {
        CachedPipelineState::Ok(_) => Ok(true),
        CachedPipelineState::Err(
            bevy::shader::ShaderCacheError::ShaderNotLoaded(_)
            | bevy::shader::ShaderCacheError::ShaderImportNotYetAvailable,
        ) => Ok(false),
        CachedPipelineState::Err(error) => Err(format!("world model pipeline {id:?}: {error}")),
        _ => Ok(false),
    }
}
fn prepare(world: &mut World, item: &mut Work, view: MeshPipelineKey) -> Result<bool, String> {
    if item.base.is_empty() {
        let Some(ids) = queue_surface(world, &item.surface, view)? else { return Ok(false) };
        item.base = ids;
    }
    for &id in &item.base {
        if !ready(world, id)? { return Ok(false); }
    }
    Ok(true)
}

/// The canonical material specializer is also used by the decal owner's
/// selected, image-free preload inputs. This owns no pipeline cache or assets.
pub(in crate::app) fn queue_surface(world: &mut World, surface: &Surface, view: MeshPipelineKey)
    -> Result<Option<Vec<CachedRenderPipelineId>>, String> {
        let Some(material) = world
            .resource::<ErasedRenderAssets<PreparedMaterial>>()
            .get(surface.material)
        else {
            return Ok(None);
        };
        let properties = material.properties.clone();
        queue_properties(world, surface, view, &properties, true)
}

/// Immutable metadata survives its temporary material binding: layouts,
/// callback and key are retained, never that material's GPU binding/uniform.
pub(in crate::app) fn queue_properties(world: &mut World, surface: &Surface, view: MeshPipelineKey,
    properties: &Arc<bevy::material::MaterialProperties>, model_diagnostics: bool) -> Result<Option<Vec<CachedRenderPipelineId>>, String> {
        let Some(mesh) = world.resource::<RenderAssets<RenderMesh>>().get(surface.mesh) else { return Ok(None) };
        let layout = mesh.layout.clone();
        let mesh_bits = MeshPipelineKey::from_bits_retain(mesh.key_bits.bits());
        let specialize = properties
            .base_specialize
            .ok_or("world model material has no MAIN specializer")?;
        let mut bits: MeshPipelineKey = properties.mesh_pipeline_key_bits.downcast();
        bits.insert(bevy::pbr::alpha_mode_pipeline_key(
            properties.alpha_mode,
            &Msaa::from_samples(view.msaa_samples()),
        ));
        let key = view | mesh_bits | bits;
        // Real Animated templates first publish current skin, then acquire the
        // previous-skin flag in set_mesh_motion_vector_flags. These are their
        // two actual lifecycle keys, not arbitrary shader permutations.
        let previous_skin =
            surface.skinned && view.contains(MeshPipelineKey::MOTION_VECTOR_PREPASS);
        let mut ids = Vec::with_capacity(1 + usize::from(previous_skin));
        for previous in 0..=usize::from(previous_skin) {
            let mesh_key = if previous == 0 {
                key
            } else {
                key | MeshPipelineKey::HAS_PREVIOUS_SKIN
            };
            let id = specialize(
                world,
                ErasedMaterialPipelineKey {
                    type_id: surface.material.type_id(),
                    mesh_key: ErasedMeshPipelineKey::new(mesh_key),
                    material_key: properties.material_key.clone(),
                },
                &layout,
                &properties,
            )
            .map_err(|error| format!("world model specialization: {error}"))?;
            if model_diagnostics && crate::rendering::bevy_backend::world_effects::trace_lifecycle() {
                let state = match world
                    .resource::<PipelineCache>()
                    .get_render_pipeline_state(id)
                {
                    CachedPipelineState::Ok(_) => "ready",
                    CachedPipelineState::Err(_) => "error",
                    _ => "pending",
                };
                eprintln!(
                    "FX_MODEL_PIPELINE mesh={:?} material={:?} mesh_key={mesh_key:?} pipeline={id:?} state={state}",
                    surface.mesh, surface.material,
                );
            }
            ids.push(id);
        }
        Ok(Some(ids))
}
