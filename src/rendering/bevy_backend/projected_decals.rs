//! Host submission inputs for ordinary animated decal projections.
//! Mesh/image assets remain with the existing geometry/material owners; the
//! FX manager supplies only the bounded projection records.
use crate::rendering::decals::material::DecalProgram;
use bevy::{
    prelude::*,
    render::{
        extract_component::ExtractComponent,
        render_resource::{
            binding_types::{sampler, storage_buffer_read_only_sized},
            *,
        },
    },
};
use std::sync::Arc;

/// Discrete material/pipeline receipts, independent of the verbose FX trace.
pub(crate) fn trace_pipelines() -> bool {
    static VALUE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *VALUE.get_or_init(|| std::env::var_os("DH2_DEBUG_DECAL_PIPELINES").is_some_and(|value| value == "1"))
}

/// Structural vertex-layout identity for same-binary A/B receipts. Hash the
/// actual attributes/stride, never the interned Arc address or mesh asset ID.
pub(crate) fn pipeline_layout_signature(layout: &bevy::mesh::MeshVertexBufferLayoutRef) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut signature = std::collections::hash_map::DefaultHasher::new();
    layout.0.as_ref().hash(&mut signature);
    signature.finish()
}

/// One material/layout proxy can submit many shared receiver meshes.
#[derive(Component, Clone)]
pub struct ProjectedDecalProxy {
    pub material: String,
    pub world: bool,
    pub program: Arc<DecalProgram>,
    pub layout: u32,
    pub mesh: AssetId<Mesh>,
}
impl bevy::render::sync_component::SyncComponent for ProjectedDecalProxy {
    type Target = Self;
}
impl ExtractComponent for ProjectedDecalProxy {
    type QueryData = &'static Self;
    type QueryFilter = ();
    type Out = Self;
    fn extract_component(item: &Self) -> Option<Self> {
        Some(item.clone())
    }
}

/// Records, projection lighting, the existing shared skin palette, and the
/// installed bilinear sampler. The first two draw immediates remain the
/// direct-surface service's {mesh_index, first}.
pub fn instance_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "projected decal instances",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::VERTEX_FRAGMENT,
            (
                storage_buffer_read_only_sized(false, None),
                storage_buffer_read_only_sized(false, None),
                storage_buffer_read_only_sized(false, None),
                storage_buffer_read_only_sized(false, None),
                sampler(SamplerBindingType::Filtering),
            ),
        ),
    )
}

/// Host GPU records keep one projection per live slot, one record per visible
/// receiver and a four-byte remap per admitted pair. Projection matrices and
/// colours are never duplicated into every receiver's instance record.
#[derive(Clone, ShaderType)]
pub(super) struct GpuProjection {
    pub world_to_decal: [Vec4; 3],
    pub colour: Vec4,
    pub lighting: [Vec4; 9],
}
impl GpuProjection {
    pub(super) fn new(
        world_to_decal: &[[f32; 4]; 4],
        colour: [f32; 4],
        lighting: [[f32; 4]; 9],
    ) -> Self {
        Self {
            world_to_decal: std::array::from_fn(|index| {
                let r = world_to_decal[index];
                Vec4::new(r[0], r[2], -r[1], r[3])
            }),
            colour: Vec4::from_array(colour),
            lighting: lighting.map(Vec4::from_array),
        }
    }
}
#[derive(Clone, ShaderType)]
pub(super) struct GpuReceiver {
    pub model: [Vec4; 3],
    pub indices: UVec4,
}
impl GpuReceiver {
    fn new(model: Mat4, indices: UVec4) -> Self {
        let rows = model.transpose();
        Self {
            model: [rows.x_axis, rows.y_axis, rows.z_axis],
            indices,
        }
    }
}

/// Geometry identities/poses come from the current scene publication. This is
/// a view of its assets, not a second geometry or animation owner.
pub struct Receiver {
    pub entity: bevy::render::sync_world::MainEntity,
    pub mesh: AssetId<Mesh>,
    pub model: Mat4,
    pub probe: u32,
    pub world_instance: Option<u32>,
    pub opaque: bool,
    pub flags48: u8,
    pub bounds: [[f32; 3]; 2],
    pub layout: u32,
    pub surface: usize,
    pub model_projections: Option<Arc<[crate::rendering::decals::ModelProjection]>>,
}
#[derive(Resource, Default)]
pub struct DecalReceivers(pub Vec<Receiver>);
pub(super) type ProgramTable = std::collections::HashMap<String, [Option<Arc<DecalProgram>>; 2]>;
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub struct DecalScene {
    pub tree: Option<Arc<crate::local_caster_query::LocalWorldQuery>>,
    pub receiver_range_ends: [u32; 2],
    pub programs: Arc<std::collections::HashMap<String, [Option<Arc<DecalProgram>>; 2]>>,
    pub failures: Arc<Vec<String>>,
}

/// Generation-scoped readiness for the existing Bevy material pipeline.
/// Both dynamic decal and particle-mesh publishers use the same render check.
#[derive(Clone, Default)]
pub(crate) struct ProxyReadinessBatch {
    pub generation: u64,
    pub proxies: Arc<Vec<Entity>>,
    pub completed: Arc<std::sync::atomic::AtomicU64>,
    pub failed: Arc<std::sync::atomic::AtomicU64>,
}
#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
pub(crate) struct DynamicProxyReadiness {
    /// Opt-in observer metadata: IDs only, never asset handles or readiness writes.
    pub diagnostics: Arc<Vec<ProxyMainObservation>>,
    pub decals: ProxyReadinessBatch,
    pub world_meshes: ProxyReadinessBatch,
}


/// A snapshot of the real main-world proxy, transported by its existing
/// readiness resource. No strong resource lease is retained by diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProxyMainObservation {
    pub entity: Entity,
    pub render_entity: Option<Entity>,
    pub main: String,
    pub images: Vec<(String, AssetId<Image>, bool)>,
    pub buffers: Vec<(&'static str, AssetId<bevy::render::storage::ShaderBuffer>, bool)>,
}

#[derive(bevy::ecs::system::SystemParam)]
struct ProxyAdmissionInputs<'w, 's> {
    meshes: Res<'w, bevy::pbr::RenderMeshInstances>,
    gpu_meshes: Res<'w, bevy::render::render_asset::RenderAssets<bevy::render::mesh::RenderMesh>>,
    materials: Res<'w, bevy::pbr::RenderMaterialInstances>,
    prepared: Res<'w, bevy::render::erased_render_asset::ErasedRenderAssets<bevy::pbr::PreparedMaterial>>,
    images: Res<'w, bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    buffers: Res<'w, bevy::render::render_asset::RenderAssets<bevy::render::storage::GpuShaderBuffer>>,
    keys: Res<'w, bevy::pbr::ViewKeyCache>,
    dirty: Res<'w, bevy::render::camera::DirtySpecializations>,
    pending: Res<'w, bevy::pbr::PendingMeshMaterialQueues>,
    opaque: Res<'w, bevy::render::render_phase::ViewBinnedRenderPhases<bevy::core_pipeline::core_3d::Opaque3d>>,
    masked: Res<'w, bevy::render::render_phase::ViewBinnedRenderPhases<bevy::core_pipeline::core_3d::AlphaMask3d>>,
    transmissive: Res<'w, bevy::render::render_phase::ViewSortedRenderPhases<bevy::pbr::Transmissive3d>>,
    transparent: Res<'w, bevy::render::render_phase::ViewSortedRenderPhases<bevy::core_pipeline::core_3d::Transparent3d>>,
    draws: Res<'w, bevy::pbr::DirectInstanceDraws>,
    mapping: Query<'w, 's, (Entity, &'static bevy::render::sync_world::MainEntity,
        Has<super::particle_meshes::ParticleMeshProxy>, Has<ProjectedDecalProxy>)>,
}

fn acknowledge_dynamic_proxies(
    request: Res<DynamicProxyReadiness>,
    specialized: Res<bevy::pbr::SpecializedMaterialPipelineCache>,
    pipelines: Res<PipelineCache>,
    water: Res<super::water_scene::WaterSceneFrame>,
    view: bevy::render::renderer::ViewQuery<(&bevy::render::sync_world::MainEntity, &bevy::render::view::ExtractedView, Option<&bevy::render::view::RenderVisibleEntities>, Option<&bevy::camera::visibility::RenderLayers>)>,
    observation: ProxyAdmissionInputs,
    mut last_observation: Local<Option<String>>,
) {
    use std::sync::atomic::Ordering;
    let (camera, view, visible, camera_layers) = view.into_inner();
    if super::world_effects::trace_lifecycle() {
        // Observe the actual render-world cache, without changing its gates,
        // specializing another pipeline, or manufacturing a proxy/binding.
        let cached = specialized.get(&view.retained_view_entity);
        let mut snapshot = format!("water_camera={:?} view_camera={:?} camera_match={} view_cache={}",
            water.0.as_ref().map(|water| water.camera), camera.id(),
            water.0.as_ref().is_some_and(|water| water.camera == camera.id()), cached.is_some());
        let retained = view.retained_view_entity;
        let visible_meshes = visible.and_then(|visible| visible.get::<Mesh3d>());
        let pending = observation.pending.get(&retained);
        snapshot.push_str(&format!(" retained={retained:?} camera_layers={camera_layers:?} view_key={:?} phases=({},{},{},{}) visibility_component={} mesh_class={} dirty_view={}",
            observation.keys.get(&retained), observation.opaque.contains_key(&retained),
            observation.masked.contains_key(&retained), observation.transmissive.contains_key(&retained),
            observation.transparent.contains_key(&retained), visible.is_some(), visible_meshes.is_some(),
            observation.dirty.views.contains(&retained)));
        for (kind, batch) in [("decal", &request.decals), ("world", &request.world_meshes)] {
            snapshot.push_str(&format!(" {kind}=(generation={},proxies={},completed={},failed={})",
                batch.generation, batch.proxies.len(), batch.completed.load(Ordering::Acquire), batch.failed.load(Ordering::Acquire)));
            for entity in batch.proxies.iter() {
                let pipeline = cached.and_then(|cache| cache.get(&bevy::render::sync_world::MainEntity::from(*entity)));
                let state = pipeline.map(|id| match pipelines.get_render_pipeline_state(*id) {
                    CachedPipelineState::Ok(_) => "ready".to_owned(),
                    CachedPipelineState::Err(error) => format!("error:{error:?}"),
                    state => format!("pending:{:?}", std::mem::discriminant(state)),
                });
                snapshot.push_str(&format!(" proxy={entity:?}:pipeline={pipeline:?}:state={state:?}"));
                let main = bevy::render::sync_world::MainEntity::from(*entity);
                let cpu = request.diagnostics.iter().find(|cpu| cpu.entity == *entity);
                let mapped = observation.mapping.iter().filter(|(_, mapped, _, _)| **mapped == main)
                    .map(|(entity, _, particle, decal)| (entity, particle, decal)).collect::<Vec<_>>();
                let mesh = observation.meshes.render_mesh_queue_data(main);
                let material = observation.materials.instances.get(&main);
                let prepared = material.and_then(|material| observation.prepared.get(material.asset_id));
                let cpu_visible = visible_meshes.map(|visible| visible.entities_cpu_culling.iter()
                    .filter(|(_, candidate)| *candidate == main).copied().collect::<Vec<_>>());
                let gpu_visible = visible_meshes.and_then(|visible| visible.entities_gpu_culling.get(&main));
                let added = visible_meshes.is_some_and(|visible| visible.added_entities.iter().any(|(_, candidate)| *candidate == main));
                let removed = visible_meshes.is_some_and(|visible| visible.removed_entities.iter().any(|(_, candidate)| *candidate == main));
                let prev = pending.map(|pending| pending.prev_frame.iter().filter(|(_, candidate)| *candidate == main).copied().collect::<Vec<_>>());
                let current = pending.map(|pending| pending.current_frame.iter().filter(|(_, candidate)| *candidate == main).copied().collect::<Vec<_>>());
                let draw = observation.draws.entities.get(&main).map(|draw| match draw {
                    bevy::pbr::DirectInstanceDraw::Instances { first, count } => format!("instances:first={first},count={count}"),
                    bevy::pbr::DirectInstanceDraw::Meshes(meshes) => format!("meshes:{}", meshes.len()),
                });
                snapshot.push_str(&format!(" main_observation={:?} render_mapping={mapped:?} mesh_instance={} mesh_id={:?} gpu_mesh={} mesh_layers={:?} material_instance={:?} prepared_material={} base_specialize={} cpu_visible={cpu_visible:?} gpu_visible={gpu_visible:?} added={added} removed={removed} dirty_changed={} dirty_removed={} pending_prev={prev:?} pending_current={current:?} direct_draw={draw:?}",
                    cpu.map(|cpu| &cpu.main), mesh.is_some(), mesh.as_ref().map(|mesh| mesh.mesh_asset_id()),
                    mesh.as_ref().is_some_and(|mesh| observation.gpu_meshes.get(mesh.mesh_asset_id()).is_some()),
                    mesh.as_ref().map(|mesh| &mesh.render_layers), material.map(|material| material.asset_id),
                    prepared.is_some(), prepared.is_some_and(|material| material.properties.base_specialize.is_some()),
                    observation.dirty.changed_renderables.contains(&main), observation.dirty.removed_renderables.contains(&main)));
                if let Some(prepared) = prepared {
                    snapshot.push_str(&format!(" prepared=(alpha={:?},phase={:?},render_method={:?},transmission={},bindless={},prepass={},shadows={})",
                        prepared.properties.alpha_mode, std::mem::discriminant(&prepared.properties.render_phase_type),
                        prepared.properties.render_method, prepared.properties.reads_view_transmission_texture,
                        prepared.properties.bindless, prepared.properties.prepass_specialize.is_some(), prepared.properties.shadows_enabled));
                }
                if let Some(cpu) = cpu {
                    snapshot.push_str(&format!(" main_render_entity={:?}", cpu.render_entity));
                    for (name, id, present) in &cpu.images {
                        snapshot.push_str(&format!(" image=({name},{id:?},cpu={present},gpu={})", observation.images.get(*id).is_some()));
                    }
                    for (name, id, present) in &cpu.buffers {
                        snapshot.push_str(&format!(" buffer=({name},{id:?},cpu={present},gpu={})", observation.buffers.get(*id).is_some()));
                    }
                }
            }
        }
        if last_observation.as_ref() != Some(&snapshot) {
            eprintln!("FX_RENDER_ADMISSION {snapshot}");
            *last_observation = Some(snapshot);
        }
    }
    let Some(water) = water.0.as_ref() else { return };
    if camera.id() != water.camera { return; }
    let Some(view_pipelines) = specialized.get(&view.retained_view_entity) else { return };
    for batch in [&request.decals, &request.world_meshes] {
        if batch.generation == 0 || batch.completed.load(Ordering::Acquire) == batch.generation
            || batch.failed.load(Ordering::Acquire) == batch.generation { continue; }
        let mut failed = false;
        let ready = batch.proxies.iter().all(|entity| {
            let main = bevy::render::sync_world::MainEntity::from(*entity);
            let Some(id) = view_pipelines.get(&main) else { return false };
            match pipelines.get_render_pipeline_state(*id) {
                CachedPipelineState::Ok(_) => return true,
                CachedPipelineState::Err(bevy::shader::ShaderCacheError::ProcessShaderError(_))
                | CachedPipelineState::Err(bevy::shader::ShaderCacheError::CreateShaderModule(_)) => {
                    bevy::log::warn!("Runtime FX proxy pipeline {id:?} failed to compile");
                    failed = true;
                }
                _ => {}
            }
            false
        });
        if failed { batch.failed.store(batch.generation, Ordering::Release); }
        if ready {
            if trace_pipelines() && std::ptr::eq(batch, &request.decals) {
                for entity in batch.proxies.iter() {
                    let main = bevy::render::sync_world::MainEntity::from(*entity);
                    let mesh = observation.meshes.render_mesh_queue_data(main);
                    let material = observation.materials.instances.get(&main);
                    let layout = mesh.as_ref().and_then(|mesh| observation.gpu_meshes.get(mesh.mesh_asset_id()))
                        .map(|mesh| pipeline_layout_signature(&mesh.layout));
                    eprintln!("DECAL_PIPELINES_ACTUAL_READY generation={} proxy={entity:?} pipeline={:?} mesh={:?} material={:?} layout_key={layout:?} view={:?} view_key={:?}",
                        batch.generation, view_pipelines.get(&main), mesh.as_ref().map(|mesh| mesh.mesh_asset_id()),
                        material.map(|material| material.asset_id), view.retained_view_entity, observation.keys.get(&view.retained_view_entity));
                }
            }
            batch.completed.store(batch.generation, Ordering::Release);
        }
    }
}

/// Immutable material values belong to the existing material binding, once per
/// material, rather than being copied into every projection/receiver pair.
#[derive(Clone, Debug, Default, Reflect, ShaderType)]
pub struct ProjectionParameters {
    pub colour_adjust: Vec4,
    pub box_projection: Vec4,
    pub parameters: Vec4,
}
impl From<&DecalProgram> for ProjectionParameters {
    fn from(program: &DecalProgram) -> Self {
        Self {
            colour_adjust: Vec4::from_array(program.colour_adjust),
            box_projection: Vec4::from_array(program.box_projection),
            parameters: Vec4::new(program.liquid_alpha_blur, 0., 0., 0.),
        }
    }
}

const PROJECTION_SHADER: Handle<Shader> =
    bevy::asset::uuid_handle!("b66e80fb-9140-4cde-b207-05794c668fe7");
pub fn register(app: &mut App) {
    bevy::asset::load_internal_asset!(
        app,
        PROJECTION_SHADER,
        "../../../assets/shaders/dh2_decal_projection.wgsl",
        Shader::from_wgsl
    );
    app.init_resource::<DynamicProxyReadiness>()
        .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<DynamicProxyReadiness>::default())
        .init_resource::<DecalScene>()
        .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
            DecalScene,
        >::default())
        .add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            ProjectedDecalProxy,
        >::default())
        .add_plugins(bevy::render::extract_component::ExtractComponentPlugin::<
            bevy::pbr::DirectMeshSkin,
        >::default());
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.init_resource::<DecalGpu>().add_systems(
            bevy::core_pipeline::Core3d,
            (prepare, acknowledge_dynamic_proxies)
                .chain()
                .after(super::camera_scene::FxUpdate)
                .before(bevy::core_pipeline::Core3dSystems::Prepass),
        );
    }
}

#[derive(Resource, Default)]
struct DecalGpu {
    projections: StorageBuffer<Vec<GpuProjection>>,
    receivers: StorageBuffer<Vec<GpuReceiver>>,
    remap: StorageBuffer<Vec<UVec2>>,
    receiver_indices: Vec<Option<u32>>,
    world_target_floors: Vec<u32>,
    previous: Vec<bevy::render::sync_world::MainEntity>,
    bindings: std::collections::HashMap<BufferId, BindGroup>,
    identities: Option<(BufferId, BufferId, BufferId)>,
    skin_buffers: Option<(BufferId, BufferId)>,
    bilinear: Option<Sampler>,
    // Physical slot identity and immutable probe-bank identity bound the cache.
    lighting: std::collections::HashMap<u32, (u64, usize, [f32; 3], [[f32; 4]; 9])>,
}

pub(super) fn release_level(world: &mut World) {
    let mut gpu = world.resource_mut::<DecalGpu>();
    let bindings = gpu.bindings.len();
    gpu.bindings.clear();
    gpu.identities = None;
    gpu.skin_buffers = None;
    gpu.previous.clear();
    gpu.receiver_indices.clear();
    gpu.world_target_floors.clear();
    gpu.lighting.clear();
    gpu.projections.get_mut().clear();
    gpu.receivers.get_mut().clear();
    gpu.remap.get_mut().clear();
    // Preserve upload capacity; release world/skinning references separately.
    if std::env::var("DH2_DEBUG_LOAD_TIMING").as_deref() == Ok("1") {
        eprintln!("MEMORY decal release: resource_bindings={bindings}");
    }
}
fn prepare(
    scene: Res<DecalScene>,
    receivers: Res<DecalReceivers>,
    proxies: Query<(&bevy::render::sync_world::MainEntity, &ProjectedDecalProxy)>,
    publication: Res<super::camera_scene::FxPublication>,
    probes: Res<super::particle_systems::ParticleQueryFrame>,
    water: Res<super::water_scene::WaterSceneFrame>,
    allocator: Res<bevy::render::mesh::allocator::MeshAllocator>,
    skins: Res<bevy::pbr::SkinUniforms>,
    device: Res<bevy::render::renderer::RenderDevice>,
    queue: Res<bevy::render::renderer::RenderQueue>,
    cache: Res<PipelineCache>,
    mut gpu: ResMut<DecalGpu>,
    mut draws: ResMut<bevy::pbr::DirectInstanceDraws>,
    view: bevy::render::renderer::ViewQuery<&bevy::render::sync_world::MainEntity>,
    mut reported: Local<Option<String>>,
    mut traced: Local<Option<(usize, usize, usize)>>,
) {
    let Some(water) = water.0.as_ref() else {
        return;
    };
    if view.into_inner().id() != water.camera {
        return;
    }
    let gpu = &mut *gpu;
    for entity in gpu.previous.drain(..) {
        draws.entities.remove(&entity);
        draws.bind_groups.remove(&entity);
    }
    // SkinUniforms can replace both buffers even in a frame without any
    // projected draws. Drop groups for old buffers once per identity change.
    let skin_buffers = (skins.current_buffer.id(), skins.prev_buffer.id());
    // The renderer normally swaps current/previous every frame; that is the
    // same pair and must not trigger a scan.
    if !gpu.skin_buffers.is_some_and(|old| old == skin_buffers || old == (skin_buffers.1, skin_buffers.0)) {
        let before = gpu.bindings.len();
        gpu.bindings.retain(|id, _| *id == skin_buffers.0 || *id == skin_buffers.1);
        gpu.skin_buffers = Some(skin_buffers);
        let retired = before - gpu.bindings.len();
        if retired != 0 && std::env::var_os("DH2_TRACE_DECALS").is_some() {
            eprintln!("DECAL_LIFETIME skin_buffer_groups_retired={retired}");
        }
    }
    for (entity, _) in &proxies {
        gpu.previous.push(*entity);
        draws
            .entities
            .insert(*entity, bevy::pbr::DirectInstanceDraw::Meshes(Vec::new()));
    }
    gpu.projections.get_mut().clear();
    gpu.receivers.get_mut().clear();
    gpu.remap.get_mut().clear();
    let result = (|| -> Result<(), String> {
        if scene.tree.is_none() {
            gpu.lighting.clear();
            return Ok(());
        }
        let decals = &publication.0.as_ref()
            .filter(|frame| frame.camera == water.camera && frame.frame == water.matrices.frame)
            .ok_or("decal publication awaits the FX update")?.decals;
        gpu.lighting
            .retain(|slot, _| decals.iter().any(|(live, _, _)| live == slot));
        if decals.is_empty() {
            return Ok(());
        }
        let bank = probes.0.as_ref().and_then(|frame| frame.probes.as_ref());
        let bank_id = bank.map_or(0, |bank| Arc::as_ptr(bank) as usize);
        for (slot, owner, decal) in decals {
            let position = decal.description.position;
            let entry = gpu.lighting.entry(*slot).or_insert((
                u64::MAX,
                usize::MAX,
                [f32::NAN; 3],
                [[0.; 4]; 9],
            ));
            if entry.0 != *owner || entry.1 != bank_id || entry.2 != position {
                let lighting = bank
                    .and_then(|bank| bank.interpolate_static_sh(position))
                    .map_or([[0.; 4]; 9], |lighting| lighting.object_shader_record());
                *entry = (*owner, bank_id, position, lighting);
            }
            gpu.projections.get_mut().push(GpuProjection::new(
                &decal.world_to_decal,
                decal.description.colour,
                entry.3,
            ));
        }
        gpu.world_target_floors.clear();
        for (_, _, decal) in decals {
            let admitted = [true, false].map(|opaque| {
                decal
                    .description
                    .material
                    .select(opaque, true)
                    .and_then(|name| scene.programs.get(name))
                    .and_then(|variants| variants[1].as_ref())
                    .is_some_and(|program| {
                        program.kind != crate::rendering::decals::material::ProjectionKind::DepthBox
                    })
            });
            gpu.world_target_floors
                .push(crate::rendering::decals::receiver_target_floor(
                    scene.receiver_range_ends,
                    admitted,
                ));
        }
        gpu.receiver_indices.clear();
        gpu.receiver_indices.resize(receivers.0.len(), None);
        // Diagnostic A/B of the previous eager receiver publication. The
        // production path records only meshes actually referenced by a pair.
        if std::env::var("DH2_DEBUG_ALL_DECAL_RECEIVERS").as_deref() == Ok("1") {
            for (number, receiver) in receivers.0.iter().enumerate() {
                let Some(vertices) = allocator.mesh_vertex_slice(&receiver.mesh) else {
                    continue;
                };
                let index = gpu.receivers.get().len() as u32;
                gpu.receiver_indices[number] = Some(index);
                gpu.receivers.get_mut().push(GpuReceiver::new(
                    receiver.model,
                    UVec4::new(
                        receiver.probe,
                        vertices.range.start,
                        skins.skin_index(receiver.entity).unwrap_or(0),
                        0,
                    ),
                ));
            }
        }
        let axes = Mat4::from_cols(
            Vec4::X,
            Vec4::new(0., 0., -1., 0.),
            Vec4::new(0., 1., 0., 0.),
            Vec4::W,
        );
        let mut model_projection_indices=std::collections::HashMap::<(u32,u64,[[u32;4];4]),u32>::new();
        for (entity, proxy) in &proxies {
            let mut packets = Vec::new();
            if proxy.program.kind == crate::rendering::decals::material::ProjectionKind::DepthBox {
                for (projection, (_, _, decal)) in decals.iter().enumerate() {
                    // 298ee0 sets the opaque depth-box bit only for overlapping
                    // WORLD receivers; the receiver depth is read in the PS.
                    let visible = receivers.0.iter().any(|receiver| {
                        receiver.world_instance.is_some()
                            && receiver.opaque
                            && decal.description.material.select(true, true)
                                == Some(&proxy.material)
                            && decal
                                .projection
                                .targets
                                .binary_search(&receiver.world_instance.unwrap())
                                .is_ok()
                    });
                    if !visible {
                        continue;
                    }
                    let d = &decal.description;
                    let model = Mat4::from_cols(
                        Vec3::from_array(d.axes[0]).extend(0.) * d.scale[0],
                        Vec3::from_array(d.axes[1]).extend(0.) * d.scale[1],
                        Vec3::from_array(d.axes[2]).extend(0.) * d.scale[2],
                        Vec3::from_array(d.position).extend(1.),
                    );
                    let receiver = gpu.receivers.get().len() as u32;
                    gpu.receivers
                        .get_mut()
                        .push(GpuReceiver::new(axes * model * axes.inverse(), UVec4::ZERO));
                    let first = gpu.remap.get().len() as u32;
                    gpu.remap
                        .get_mut()
                        .push(UVec2::new(projection as u32, receiver));
                    packets.push(bevy::pbr::DirectMeshDraw {
                        mesh: proxy.mesh,
                        first,
                        count: 1,
                    });
                }
            } else {
                // 33bf30: receiver order, then physical projection order.
                for (number, receiver) in receivers.0.iter().enumerate() {
                    if receiver.world_instance.is_some() != proxy.world
                        || receiver.layout != proxy.layout
                    {
                        continue;
                    }
                    let first = gpu.remap.get().len() as u32;
                    for (projection, (_, _, decal)) in decals.iter().enumerate() {
                        if decal
                            .description
                            .material
                            .select(receiver.opaque, proxy.world)
                            != Some(&proxy.material)
                        {
                            continue;
                        }
                        let admitted = if let Some(world) = receiver.world_instance {
                            let depth_box = decal.description.material.opaque_instanced.as_deref()
                                .and_then(|name| scene.programs.get(name)).and_then(|variants| variants[1].as_ref())
                                .is_some_and(|program| program.kind == crate::rendering::decals::material::ProjectionKind::DepthBox);
                            world >= gpu.world_target_floors[projection]
                                && crate::rendering::decals::receiver_accepts_geometry(
                                    depth_box,
                                    u32::from(!receiver.opaque),
                                    receiver.flags48,
                                )
                                && decal.projection.targets.binary_search(&world).is_ok()
                        } else {
                            receiver.model_projections.as_deref().is_some_and(|rows|rows.iter().any(|row|
                                row.slot==decals[projection].0 && row.owner==decals[projection].1
                                    && row.surfaces & 1u32.rotate_left(receiver.surface as u32)!=0))
                        };
                        if admitted {
                            let index = if let Some(index) = gpu.receiver_indices[number] {
                                index
                            } else {
                                let Some(vertices) = allocator.mesh_vertex_slice(&receiver.mesh)
                                else {
                                    continue;
                                };
                                let index = gpu.receivers.get().len() as u32;
                                gpu.receivers.get_mut().push(GpuReceiver::new(
                                    receiver.model,
                                    UVec4::new(
                                        receiver.probe,
                                        vertices.range.start,
                                        skins.skin_index(receiver.entity).unwrap_or(0),
                                        0,
                                    ),
                                ));
                                gpu.receiver_indices[number] = Some(index);
                                index
                            };
                            let projection_index = if let Some(rows)=receiver.model_projections.as_deref() {
                                let row=rows.iter().find(|row|row.slot==decals[projection].0 && row.owner==decals[projection].1
                                    && row.surfaces & 1u32.rotate_left(receiver.surface as u32)!=0).ok_or("admitted model projection context absent")?;
                                let key=(row.slot,row.owner,row.world_to_decal.map(|r|r.map(f32::to_bits)));
                                *model_projection_indices.entry(key).or_insert_with(||{
                                    let index=gpu.projections.get().len() as u32;
                                    let lighting=gpu.lighting.get(&row.slot).map_or([[0.;4];9],|entry|entry.3);
                                    gpu.projections.get_mut().push(GpuProjection::new(&row.world_to_decal,decal.description.colour,lighting));
                                    index
                                })
                            }else{projection as u32};
                            gpu.remap.get_mut().push(UVec2::new(projection_index,index));
                        }
                    }
                    let count = gpu.remap.get().len() as u32 - first;
                    if count != 0 {
                        packets.push(bevy::pbr::DirectMeshDraw {
                            mesh: receiver.mesh,
                            first,
                            count,
                        });
                    }
                }
            }
            draws
                .entities
                .insert(*entity, bevy::pbr::DirectInstanceDraw::Meshes(packets));
        }
        // bevy_backend: model-local contexts exceed the WORLD-only 8-bit
        // projection payload. Explicit u32 pairs preserve every row identity.
        if gpu.receivers.get().len()>u32::MAX as usize || gpu.projections.get().len()>u32::MAX as usize {
            return Err("decal record indices exceed u32".into());
        }
        if std::env::var_os("DH2_TRACE_DECALS").is_some() {
            let counts = (
                gpu.projections.get().len(),
                gpu.receivers.get().len(),
                gpu.remap.get().len(),
            );
            if *traced != Some(counts) {
                info!(frame=water.matrices.frame, projections=counts.0, receivers=counts.1, pairs=counts.2,
                    first_colour=?gpu.projections.get().first().map(|p| p.colour.to_array()), "Decal publication");
                *traced = Some(counts);
            }
        }
        if gpu.remap.get().is_empty() {
            return Ok(());
        }
        gpu.projections.write_buffer(&device, &queue);
        gpu.receivers.write_buffer(&device, &queue);
        gpu.remap.write_buffer(&device, &queue);
        let identities = (
            gpu.projections.buffer().unwrap().id(),
            gpu.receivers.buffer().unwrap().id(),
            gpu.remap.buffer().unwrap().id(),
        );
        if gpu.identities != Some(identities) {
            gpu.bindings.clear();
            gpu.identities = Some(identities);
        }
        let sampler = gpu
            .bilinear
            .get_or_insert_with(|| super::shadow_pipeline::shadow_alpha_sampler(&device));
        let group = gpu
            .bindings
            .entry(skins.current_buffer.id())
            .or_insert_with(|| {
                device.create_bind_group(
                    Some("projected decal records"),
                    &cache.get_bind_group_layout(&instance_layout()),
                    // Bind capacity, not last_written_size: cached groups survive
                    // shrinking and regrowing lists within the same allocation.
                    &BindGroupEntries::sequential((
                        gpu.projections.buffer().unwrap().as_entire_binding(),
                        gpu.receivers.buffer().unwrap().as_entire_binding(),
                        gpu.remap.buffer().unwrap().as_entire_binding(),
                        skins.current_buffer.as_entire_binding(),
                        &*sampler,
                    )),
                )
            });
        for entity in &gpu.previous {
            draws.bind_groups.insert(*entity, group.clone());
        }
        Ok(())
    })();
    if let Err(error) = result {
        // Never leave partially prepared draws pointing at stale frame buffers.
        for entity in &gpu.previous {
            draws
                .entities
                .insert(*entity, bevy::pbr::DirectInstanceDraw::Meshes(Vec::new()));
        }
        if reported.as_ref() != Some(&error) {
            warn!("Decal submission pending: {error}");
            *reported = Some(error);
        }
    } else {
        *reported = None;
    }
}
