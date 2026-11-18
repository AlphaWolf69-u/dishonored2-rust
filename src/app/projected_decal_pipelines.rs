//! Host admission of0661e0's selected decal programs.2be900 selects installed
//! program inputs and2c9f00 reads serialized shaders; Bevy additionally needs
//! the actual view and receiver vertex layout. PipelineCache remains the owner.
use super::*;
use crate::rendering::{
    bevy_backend::{asset_preparation::AssetWrite, water_scene::WaterSceneFrame},
    decals::material::ProjectionKind,
};
use bevy::{
    material::MaterialProperties,
    pbr::{MeshPipelineKey, PreparedMaterial, ViewKeyCache},
    render::{
        erased_render_asset::ErasedRenderAssets,
        render_resource::*,
        sync_world::MainEntity,
        view::{ExtractedView, RetainedViewEntity, ViewTarget},
        Render, RenderSystems,
    },
};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

fn trace() -> bool {
    crate::rendering::bevy_backend::projected_decals::trace_pipelines()
}
fn deferred() -> bool {
    static VALUE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| {
        std::env::var_os("DH2_DEBUG_DEFER_DECAL_PIPELINES").is_some_and(|value| value == "1")
    })
}
#[derive(Default)]
struct Failure {
    failed: AtomicBool,
    message: Mutex<Option<String>>,
}
impl Failure {
    fn set(&self, error: String) {
        *self.message.lock().unwrap() = Some(error);
        self.failed.store(true, Ordering::Release);
    }
}
struct Program {
    name: String,
    world: bool,
    material: AssetId<super::super::DecalMaterial>,
    kind: ProjectionKind,
    // Only opt-in diagnostics format these actual canonical typed keys.
    key: String,
}
#[derive(Resource, Default)]
pub(super) struct Inputs {
    programs: Arc<Vec<Program>>,
    // Temporary ordinary material assets, never meshes/images/proxy actors.
    // Released as soon as Bevy has published their immutable properties.
    bindings: Vec<Handle<super::super::DecalMaterial>>,
    captured: Arc<AtomicBool>,
    complete: Arc<AtomicBool>,
    error: Arc<Failure>,
    closed: bool,
}
pub(super) fn prepare(
    resolver: &mut MaterialResolver<'_>,
    meshes: &mut dyn AssetWrite<Mesh>,
    layouts: &mut GeometryLayouts,
) -> Inputs {
    let mut result = Inputs::default();
    if deferred() {
        result.complete.store(true, Ordering::Release);
        return result;
    }
    let Some(catalog) = resolver.catalog else {
        result.complete.store(true, Ordering::Release);
        return result;
    };
    let work = (|| -> Result<(), String> {
        let names = crate::assets::decal::preloaded_materials(catalog)?;
        let mut programs = Vec::with_capacity(names.len());
        for (name, world) in names {
            let image_slots = resolver.diffuse_image_paths.len();
            let handle = resolver.decal_pipeline_material(&name, world)?;
            if resolver.diffuse_image_paths.len() != image_slots {
                return Err("decal pipeline admission registered image payload slots".into());
            }
            let material = resolver
                .decal_materials
                .get(&handle)
                .ok_or("decal pipeline input material absent")?;
            let program = material
                .extension
                .projection
                .as_ref()
                .ok_or("selected decal projection program absent")?;
            let key = if trace() {
                let base = bevy::pbr::StandardMaterialKey::from(&material.base);
                format!(
                    "base={:#x},decal={:?}",
                    base.bits(),
                    super::super::materials::DecalKey::from(&material.extension)
                )
            } else {
                String::new()
            };
            if program.kind == ProjectionKind::DepthBox && layouts.depth_box.is_none() {
                layouts.depth_box = Some(super::depth_box_mesh(meshes));
            }
            programs.push(Program {
                name,
                world,
                material: handle.id(),
                kind: program.kind,
                key,
            });
            result.bindings.push(handle);
        }
        if trace() {
            eprintln!("DECAL_PIPELINES_ADMISSION materials={} image_payload_bytes=0 proxy_actors=0 extra_draws=0 extra_passes=0 temporary_material_bindings={}", programs.len(), result.bindings.len());
        }
        result
            .complete
            .store(programs.is_empty(), Ordering::Release);
        result.programs = Arc::new(programs);
        Ok(())
    })();
    if let Err(error) = work {
        result.error.set(error);
    }
    result
}
pub(in crate::app) fn pipelines_pending(world: &World) -> bool {
    world
        .get_resource::<Inputs>()
        .is_some_and(|inputs| !inputs.closed && !inputs.complete.load(Ordering::Acquire))
}
pub(in crate::app) fn pipelines_error(world: &World) -> Option<String> {
    let inputs = world.get_resource::<Inputs>()?;
    inputs
        .error
        .failed
        .load(Ordering::Acquire)
        .then(|| inputs.error.message.lock().unwrap().clone())
        .flatten()
}
pub(in crate::app) fn retire_pipeline_inputs(world: &mut World) {
    if let Some(mut inputs) = world.get_resource_mut::<Inputs>() {
        inputs.closed = true;
        inputs.bindings.clear();
        inputs.programs = Arc::default();
        inputs.complete.store(true, Ordering::Release);
    }
    if let Some(mut layouts) = world.get_resource_mut::<GeometryLayouts>() {
        layouts.depth_box = None;
    }
}
fn release_bindings(mut inputs: ResMut<Inputs>) {
    if !inputs.bindings.is_empty() && inputs.captured.load(Ordering::Acquire) {
        if trace() {
            eprintln!(
                "DECAL_PIPELINES_BINDINGS_RELEASE count={} immutable_properties_captured=true",
                inputs.bindings.len()
            );
        }
        inputs.bindings.clear();
    }
}

#[derive(Clone, Copy)]
struct Sample {
    mesh: AssetId<Mesh>,
    layout: u32,
    world: bool,
}
struct Work {
    program: usize,
    mesh: AssetId<Mesh>,
    layout: u32,
    ids: Vec<CachedRenderPipelineId>,
    ready: bool,
}
type View = (RetainedViewEntity, MeshPipelineKey, TextureFormat);
#[derive(Resource, Default)]
struct Pending {
    programs: Arc<Vec<Program>>,
    properties: Vec<Option<Arc<MaterialProperties>>>,
    samples: Vec<Sample>,
    depth_box: Option<AssetId<Mesh>>,
    layout_revision: u64,
    work: Vec<Work>,
    view: Option<View>,
    dirty: bool,
    captured: Arc<AtomicBool>,
    complete: Arc<AtomicBool>,
    error: Arc<Failure>,
}
pub(super) fn register(app: &mut App) {
    app.init_resource::<Inputs>()
        .add_systems(Last, release_bindings);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<Pending>()
            .add_systems(ExtractSchedule, extract)
            .add_systems(
                Render,
                acknowledge.run_if(needed).in_set(RenderSystems::Specialize),
            );
    }
}
fn extract(
    inputs: Extract<Res<Inputs>>,
    layouts: Extract<Res<GeometryLayouts>>,
    mut pending: ResMut<Pending>,
) {
    let sources_changed = !Arc::ptr_eq(&inputs.programs, &pending.programs);
    if sources_changed {
        pending.programs = inputs.programs.clone();
        pending.properties = vec![None; inputs.programs.len()];
        pending.captured = inputs.captured.clone();
        pending.complete = inputs.complete.clone();
        pending.error = inputs.error.clone();
    }
    if sources_changed || pending.layout_revision != layouts.revision {
        pending.samples = layouts
            .samples
            .iter()
            .map(|(layout, world, mesh, _)| Sample {
                mesh: mesh.id(),
                layout: *layout,
                world: *world,
            })
            .collect();
        pending.depth_box = layouts.depth_box.as_ref().map(Handle::id);
        pending.layout_revision = layouts.revision;
        pending.dirty = true;
        if !inputs.programs.is_empty() {
            inputs.complete.store(false, Ordering::Release);
        }
    }
}
fn current_view(
    water: &WaterSceneFrame,
    keys: &ViewKeyCache,
    views: &Query<(&MainEntity, &ExtractedView, &ViewTarget)>,
) -> Option<View> {
    let camera = water.0.as_ref()?.camera;
    views
        .iter()
        .find(|(main, _, _)| main.id() == camera)
        .and_then(|(_, view, target)| {
            keys.get(&view.retained_view_entity).map(|key| {
                (
                    view.retained_view_entity,
                    *key,
                    target.main_texture_format(),
                )
            })
        })
}
fn needed(
    pending: Res<Pending>,
    water: Res<WaterSceneFrame>,
    keys: Res<ViewKeyCache>,
    views: Query<(&MainEntity, &ExtractedView, &ViewTarget)>,
) -> bool {
    if pending.error.failed.load(Ordering::Acquire) {
        return false;
    }
    let view = current_view(&water, &keys, &views);
    pending.dirty
        || pending.view != view
        || (!pending.programs.is_empty()
            && !pending.complete.load(Ordering::Acquire)
            && view.is_some())
}
fn acknowledge(world: &mut World) {
    let view = {
        let water = world.resource::<WaterSceneFrame>();
        let camera = water.0.as_ref().map(|frame| frame.camera);
        let mut query = world.query::<(&MainEntity, &ExtractedView, &ViewTarget)>();
        camera.and_then(|camera| {
            query
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
        })
    };
    world.resource_scope(|world, mut pending: Mut<Pending>| {
        let result = (|| -> Result<(), String> {
            if pending.dirty || pending.view != view {
                pending.work.clear(); pending.view = view; pending.dirty = false;
                let Pending { programs, samples, depth_box, work, .. } = &mut *pending;
                for (index, program) in programs.iter().enumerate() {
                    if program.kind == ProjectionKind::DepthBox {
                        if let Some(mesh) = *depth_box { work.push(Work { program: index, mesh, layout: u32::MAX, ids: Vec::new(), ready: false }); }
                    } else {
                        for sample in samples.iter().filter(|sample| sample.world == program.world) {
                            work.push(Work { program: index, mesh: sample.mesh, layout: sample.layout, ids: Vec::new(), ready: false });
                        }
                    }
                }
                pending.complete.store(pending.programs.is_empty(), Ordering::Release);
            }
            for index in 0..pending.properties.len() {
                if pending.properties[index].is_none() {
                    if let Some(material) = world.resource::<ErasedRenderAssets<PreparedMaterial>>().get(pending.programs[index].material) {
                        pending.properties[index] = Some(material.properties.clone());
                    }
                }
            }
            if pending.properties.iter().all(Option::is_some) { pending.captured.store(true, Ordering::Release); }
            let Some((_, view_key, _)) = view else { return Ok(()) };
            for index in 0..pending.work.len() {
                if pending.work[index].ready { continue; }
                let program_index = pending.work[index].program;
                let Some(properties) = pending.properties[program_index].clone() else { continue; };
                if pending.work[index].ids.is_empty() {
                    let work = &pending.work[index];
                    // DirectMeshSkin consumes only the current shared palette;
                    // these proxies have no PreviousSkin component/history.
                    let surface = super::super::world_models::pipelines::Surface { mesh: work.mesh,
                        material: pending.programs[program_index].material.untyped(), skinned: false, alpha:None };
                    let Some(ids) = super::super::world_models::pipelines::queue_properties(world, &surface, view_key, &properties, false)? else { continue; };
                    if trace() {
                        let program = &pending.programs[program_index];
                        let layout_key = world.resource::<bevy::render::render_asset::RenderAssets<bevy::render::mesh::RenderMesh>>()
                            .get(work.mesh).map(|mesh| crate::rendering::bevy_backend::projected_decals::pipeline_layout_signature(&mesh.layout));
                        eprintln!("DECAL_PIPELINES_QUEUE material={} world={} key={} mesh={:?} layout={} layout_key={layout_key:?} view={view:?} pipelines={ids:?}", program.name, program.world, program.key, work.mesh, work.layout);
                    }
                    pending.work[index].ids = ids;
                }
                let mut ready = true;
                for &id in &pending.work[index].ids {
                    match world.resource::<PipelineCache>().get_render_pipeline_state(id) {
                        CachedPipelineState::Ok(_) => (),
                        CachedPipelineState::Err(bevy::shader::ShaderCacheError::ShaderNotLoaded(_)
                            | bevy::shader::ShaderCacheError::ShaderImportNotYetAvailable) => ready = false,
                        CachedPipelineState::Err(error) => return Err(format!("selected decal pipeline {id:?}: {error}")),
                        _ => ready = false,
                    }
                }
                if ready {
                    pending.work[index].ready = true;
                    if trace() { let work = &pending.work[index]; let program = &pending.programs[work.program];
                        eprintln!("DECAL_PIPELINES_READY material={} world={} key={} mesh={:?} layout={} view={view:?} pipelines={:?}", program.name, program.world, program.key, work.mesh, work.layout, work.ids); }
                }
            }
            if pending.captured.load(Ordering::Acquire) && pending.work.iter().all(|work| work.ready) { pending.complete.store(true, Ordering::Release); }
            Ok(())
        })();
        if let Err(error) = result { pending.error.set(error); }
    });
}
