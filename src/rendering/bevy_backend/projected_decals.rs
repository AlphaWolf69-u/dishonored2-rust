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
    pub decals: ProxyReadinessBatch,
    pub world_meshes: ProxyReadinessBatch,
}

fn acknowledge_dynamic_proxies(
    request: Res<DynamicProxyReadiness>,
    specialized: Res<bevy::pbr::SpecializedMaterialPipelineCache>,
    pipelines: Res<PipelineCache>,
    water: Res<super::water_scene::WaterSceneFrame>,
    view: bevy::render::renderer::ViewQuery<(&bevy::render::sync_world::MainEntity, &bevy::render::view::ExtractedView)>,
) {
    use std::sync::atomic::Ordering;
    let Some(water) = water.0.as_ref() else { return };
    let (camera, view) = view.into_inner();
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
        if ready { batch.completed.store(batch.generation, Ordering::Release); }
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
    remap: StorageBuffer<Vec<u32>>,
    receiver_indices: Vec<Option<u32>>,
    world_target_floors: Vec<u32>,
    previous: Vec<bevy::render::sync_world::MainEntity>,
    bindings: std::collections::HashMap<BufferId, BindGroup>,
    identities: Option<(BufferId, BufferId, BufferId)>,
    bilinear: Option<Sampler>,
    // Physical slot identity and immutable probe-bank identity bound the cache.
    lighting: std::collections::HashMap<u32, (u64, usize, [f32; 3], [[f32; 4]; 9])>,
}

pub(super) fn release_level(world: &mut World) {
    let mut gpu = world.resource_mut::<DecalGpu>();
    let bindings = gpu.bindings.len();
    gpu.bindings.clear();
    gpu.identities = None;
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
    mut camera: ResMut<super::camera_scene::CameraGpu>,
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
        let Some(tree) = scene.tree.as_deref() else {
            return Ok(());
        };
        let pool = camera.world_particles.decal_pool();
        let limit = crate::platform::console::shared()
            .read()
            .cvars
            .int("r_maxDynamicDecals")
            .ok_or("decal budget absent")?;
        pool.reconfigure_when_empty(limit as usize, |_| {})?;
        pool.prepare_world(tree)?;
        let decals = (0..pool.limit() as u32)
            .filter_map(|slot| {
                let entry = pool.slot(slot)?;
                Some((slot, entry.owner()?.get(), entry.renderer.as_ref()?))
            })
            .collect::<Vec<_>>();
        gpu.lighting
            .retain(|slot, _| decals.iter().any(|(live, _, _)| live == slot));
        if decals.is_empty() {
            return Ok(());
        }
        let bank = probes.0.as_ref().and_then(|frame| frame.probes.as_ref());
        let bank_id = bank.map_or(0, |bank| Arc::as_ptr(bank) as usize);
        for (slot, owner, decal) in &decals {
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
        for (_, _, decal) in &decals {
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
                        .push((projection as u32) << 24 | receiver);
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
                            let d = &decal.description;
                            let volume = crate::rendering::bounds::OrientedBox {
                                centre: std::array::from_fn(|axis| {
                                    d.position[axis] - d.axes[2][axis] * d.scale[2] * 0.5
                                }),
                                half_extent: [d.scale[0], d.scale[1], d.scale[2] * 0.5],
                                axes: d.axes,
                            };
                            volume.overlaps(&crate::rendering::bounds::OrientedBox::from_aabb(
                                receiver.bounds,
                            ))
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
                            gpu.remap.get_mut().push((projection as u32) << 24 | index);
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
        if gpu.receivers.get().len() >= 0x01000000 {
            return Err("decal receiver remap exceeds 24 bits".into());
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
        gpu.bindings
            .retain(|id, _| *id == skins.current_buffer.id() || *id == skins.prev_buffer.id());
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
