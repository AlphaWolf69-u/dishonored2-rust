//! bevy_backend: an uploaded mesh/material is not a compiled draw pipeline.
//! 1402be900/1402c9f00 select/cache the installed material programs; Bevy adds
//! view/layout specialization. Admit the real template only after that extra
//! host dependency is ready, using the same specialization owner as real draws.
use super::*;
use crate::rendering::bevy_backend::water_scene::WaterSceneFrame;
use bevy::{
    material::key::{ErasedMaterialPipelineKey, ErasedMeshPipelineKey},
    core_pipeline::{prepass::{DepthPrepass, NormalPrepass, MotionVectorPrepass}, deferred::DeferredPrepass},
    ecs::query::Has,
    pbr::{MeshPipelineKey, PreparedMaterial, ViewKeyCache, ViewKeyPrepassCache},
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
type ModelView = (&'static MainEntity, &'static ExtractedView, &'static ViewTarget,
    Has<DepthPrepass>, Has<NormalPrepass>, Has<MotionVectorPrepass>, Has<DeferredPrepass>);
type PrepassView = Option<(MeshPipelineKey, bool)>;
type ViewSignature = (RetainedViewEntity, MeshPipelineKey, TextureFormat, PrepassView);

#[derive(Resource, Default)]
pub(super) struct Pending {
    sources: Weak<Vec<Upload>>,
    view: Option<ViewSignature>,
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
    prepass_keys: Res<ViewKeyPrepassCache>,
    views: Query<ModelView>,
) -> bool {
    let view = water.0.as_ref().and_then(|water| {
        views
            .iter()
            .find(|(main, ..)| main.id() == water.camera)
            .and_then(|(_, view, target, depth, normal, motion, deferred)| {
                let key=*keys.get(&view.retained_view_entity)?;
                let prepass=if depth||normal||motion||deferred {
                    Some((*prepass_keys.get(&view.retained_view_entity)?,deferred))
                }else{None};
                Some((view.retained_view_entity,key,target.main_texture_format(),prepass))
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
        let mut views = world.query::<ModelView>();
        views
            .iter(world)
            .find(|(main, ..)| main.id() == camera)
            .and_then(|(_, view, target, depth, normal, motion, deferred)| {
                let key=*world.resource::<ViewKeyCache>().get(&view.retained_view_entity)?;
                let prepass=if depth||normal||motion||deferred {
                    Some((*world.resource::<ViewKeyPrepassCache>().get(&view.retained_view_entity)?,deferred))
                }else{None};
                Some((view.retained_view_entity,key,target.main_texture_format(),prepass))
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
        let Some((_, view_key, _, prepass)) = view else {
            return;
        };
        // No per-surface work, cloning or specialization once this exact
        // template roster/view has completed. On retirement the weak roster
        // changes and removes its pending work before asset handles can recycle.
        let mut index = 0;
        while index < pending.work.len() {
            let result = prepare(world, &mut pending.work[index], view_key, prepass);
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
fn prepare(world: &mut World, item: &mut Work, view: MeshPipelineKey, prepass: PrepassView) -> Result<bool, String> {
    if item.base.is_empty() {
        let Some(mut ids) = queue_surface(world, &item.surface, view)? else { return Ok(false) };
        //Actual model draw admission also owns its enabled depth/normal/motion
        //pass. The separate decal program-only queue_properties API remains
        //main-only: no fictitious prepass entity or material is introduced.
        if let Some((key,deferred))=prepass {
            let Some(prepass_ids)=queue_model_prepass(world,&item.surface,key,deferred)? else{return Ok(false);};
            ids.extend(prepass_ids);
        }
        item.base = ids;
    }
    for &id in &item.base {
        if !ready(world, id)? { return Ok(false); }
    }
    Ok(true)
}

///Same prepared material and mesh as this actual model's normal admission.
///Mesh layout encodes skinning; only its real no-previous/previous-skin
///lifecycle flags are requested, on the actual prepass view key.
fn queue_model_prepass(world:&mut World,surface:&Surface,view:MeshPipelineKey,deferred:bool)
    ->Result<Option<Vec<CachedRenderPipelineId>>,String>{
    let Some(mesh)=world.resource::<RenderAssets<RenderMesh>>().get(surface.mesh)else{return Ok(None);};
    let layout=mesh.layout.clone();
    let mesh_bits=MeshPipelineKey::from_bits_retain(mesh.key_bits.bits());
    let Some(material)=world.resource::<ErasedRenderAssets<PreparedMaterial>>().get(surface.material)else{return Ok(None);};
    let properties=material.properties.clone();
    let previous_skin=surface.skinned&&view.contains(MeshPipelineKey::MOTION_VECTOR_PREPASS);
    let mut ids=Vec::with_capacity(1+usize::from(previous_skin));
    for previous in 0..=usize::from(previous_skin){
        let mut key=view|mesh_bits;
        if previous!=0{key|=MeshPipelineKey::HAS_PREVIOUS_SKIN;}
        if let Some((id,actual_key))=bevy::pbr::specialize_prepared_material_prepass(world,
            surface.material.type_id(),&properties,&layout,key,deferred,(surface.mesh,surface.material))
            .map_err(|error|format!("world model prepass mesh={:?} material={:?}: {error}",surface.mesh,surface.material))? {
            let _=actual_key; //Both phases are observed by the canonical owner.
            ids.push(id);
        }
    }
    Ok(Some(ids))
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
