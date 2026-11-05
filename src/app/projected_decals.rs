//! Prepare FX decal bindings with the ordinary material and geometry owners.
use super::{
    MaterialHandle, MaterialResolver, RenderInstance, RenderModelSurface, SceneRenderQueue,
    ShadowMaterials,
};
use crate::rendering::bevy_backend::projected_decals::{
    DecalReceivers, DecalScene, ProjectedDecalProxy, Receiver,
};
use bevy::{
    prelude::*,
    render::{sync_world::MainEntity, Extract, ExtractSchedule, RenderApp},
};
use std::{
    collections::HashMap,
    sync::Arc,
};

type Images = Vec<(MaterialHandle, Vec<(usize, crate::surface_gpu::ImageRole)>)>;
#[derive(Resource, Default)]
struct GeometryLayouts {
    ids: HashMap<AssetId<Mesh>, u32>,
    samples: Vec<(u32, bool, Handle<Mesh>, bool)>,
}

struct RuntimeDecalProxySet {
    _prepared: super::runtime_materials::PreparedDecal,
    entities: Vec<Entity>,
}
#[derive(Resource, Default)]
pub(super) struct RuntimeDecalProxies {
    generation: u64,
    owned: HashMap<(String, bool), RuntimeDecalProxySet>,
    cube: Option<Handle<Mesh>>,
    owns_scene_geometry: bool,
}

#[derive(Default)]
struct BoundsOwners {
    domain: (usize, usize),
    placements: HashMap<usize, usize>,
}

pub(super) struct ProjectedDecalPlugin;
impl Plugin for ProjectedDecalPlugin {
    fn build(&self, app: &mut App) {
        register(app);
    }
}
pub(super) fn register(app: &mut App) {
    crate::rendering::bevy_backend::projected_decals::register(app);
    app.init_resource::<GeometryLayouts>();
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render
            .init_resource::<DecalReceivers>()
            .add_systems(ExtractSchedule, extract_receivers);
    }
}

/// Preserve compact receiver layouts while the setup meshes are available.
fn receiver_layouts(
    instances: &[RenderInstance],
    meshes: &dyn crate::rendering::bevy_backend::asset_preparation::AssetWrite<Mesh>,
) -> GeometryLayouts {
    let mut keys = HashMap::new();
    let mut layouts = GeometryLayouts::default();
    for instance in instances {
        let Some(source) = instance.source else {
            continue;
        };
        if layouts.ids.contains_key(&instance.mesh.id()) {
            continue;
        }
        let Some(mesh) = meshes.get(&instance.mesh) else {
            continue;
        };
        let key = mesh
            .attributes()
            .map(|(attribute, _)| (attribute.id, attribute.format))
            .collect::<Vec<_>>();
        let next = keys.len() as u32;
        let id = *keys.entry(key).or_insert(next);
        let is_world = matches!(source.surface, RenderModelSurface::World { .. });
        layouts.ids.insert(instance.mesh.id(), id);
        if !layouts.samples
            .iter()
            .any(|(layout, world, _, _)| {
                *layout == id && *world == is_world
            })
        {
            layouts.samples.push((id, is_world, instance.mesh.clone(),
                mesh.contains_attribute(Mesh::ATTRIBUTE_JOINT_INDEX)));
        }
    }
    layouts
}

pub(super) fn prepare(
    sources: &crate::rendering::particles::world_sources::WorldFxSources,
    world: &crate::world_mesh::WorldMesh,
    instances: &[RenderInstance],
    resolver: &mut MaterialResolver,
    meshes: &mut dyn crate::rendering::bevy_backend::asset_preparation::AssetWrite<Mesh>,
    commands: &mut super::setup_commands::SetupCommands,
) -> Images {
    let names = sources.decal_materials();
    let layouts = receiver_layouts(instances, meshes);
    if names.is_empty() {
        commands.insert_resource(DecalScene::default());
        commands.insert_resource(layouts);
        return Vec::new();
    }
    // Native projection local bounds [-1,-1,-1]..[1,1,0], converted to view axes.
    let mut cube_mesh = Cuboid::new(2., 1., 2.)
        .mesh()
        .build()
        .translated_by(Vec3::new(0., -0.5, 0.));
    cube_mesh
        .generate_tangents()
        .expect("complete cube tangent inputs");
    let cube = meshes.add(cube_mesh);
    let mut programs = HashMap::new();
    let mut failures = Vec::new();
    let mut bindings = Vec::new();
    for (name, world) in names {
        let result = (|| -> Result<_, String> {
            let (binding, images) = resolver.projected_decal_material(&name, world)?;
            let MaterialHandle::Decal(handle) = &binding else {
                unreachable!()
            };
            let program = resolver
                .decal_materials
                .get(handle)
                .and_then(|material| material.extension.projection.clone())
                .ok_or("projected decal controls absent")?;
            let depth_box =
                program.kind == crate::rendering::decals::material::ProjectionKind::DepthBox;
            let proxy = |commands: &mut super::setup_commands::SetupCommands, mesh: Handle<Mesh>, layout: u32| {
                let skinned = meshes
                    .get(&mesh)
                    .is_some_and(|mesh| mesh.contains_attribute(Mesh::ATTRIBUTE_JOINT_INDEX));
                let mut entity = commands.spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(handle.clone()),
                    Transform::IDENTITY,
                    GlobalTransform::IDENTITY,
                    Visibility::Visible,
                    bevy::camera::visibility::NoFrustumCulling,
                    bevy::render::batching::NoAutomaticBatching,
                    bevy::light::NotShadowCaster,
                    ProjectedDecalProxy {
                        material: name.clone(),
                        world,
                        program: program.clone(),
                        layout,
                        mesh: mesh.id(),
                    },
                ));
                if skinned {
                    entity.insert(bevy::pbr::DirectMeshSkin);
                }
            };
            if depth_box {
                proxy(commands, cube.clone(), u32::MAX);
            } else {
                for (layout, is_world, mesh, _) in &layouts.samples {
                    if *is_world == world {
                        proxy(commands, mesh.clone(), *layout);
                    }
                }
            }
            programs.entry(name.clone()).or_insert_with(|| [None, None])[usize::from(world)] =
                Some(program);
            Ok((binding, images))
        })();
        match result {
            Ok(binding) => bindings.push(binding),
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    let mut receiver_range_ends = [0; 2];
    for instance in instances {
        if let Some(super::RenderInstanceSource {
            surface: RenderModelSurface::World { object, .. },
            ..
        }) = instance.source
        {
            if let Some(Ok(material)) = resolver.shadow_materials.0.get(&instance.material) {
                let class = usize::from(material.source.main_blend_state & 1 != 0);
                receiver_range_ends[class] =
                    receiver_range_ends[class].max(world.objects[object].matrix_end as u32);
            }
        }
    }
    commands.insert_resource(DecalScene {
        tree: Some(world.spatial_query()),
        receiver_range_ends,
        programs: Arc::new(programs),
        failures: Arc::new(failures),
    });
    commands.insert_resource(layouts);
    bindings
}

/// Publish prepared dynamic decal assets through the same receiver proxies as
/// setup. Prepared records remain leased here for as long as the coordinator
/// retains the corresponding effect or tail.
pub(super) fn publish_runtime(
    request: Option<Res<super::usable_effects::DynamicDecalMaterials>>,
    mut state: Option<ResMut<RuntimeDecalProxies>>,
    mut readiness: ResMut<crate::rendering::bevy_backend::projected_decals::DynamicProxyReadiness>,
    scene_data: Option<Res<super::SceneData>>,
    queue: Option<Res<SceneRenderQueue>>,
    materials: Option<Res<ShadowMaterials>>,
    mut scene: ResMut<DecalScene>,
    layouts: Res<GeometryLayouts>,
    mut meshes: ResMut<Assets<Mesh>>,
    existing: Query<(Entity, &ProjectedDecalProxy)>,
    mut commands: Commands,
) {
    let (Some(request), Some(mut state), Some(scene_data), Some(queue), Some(materials)) =
        (request, state, scene_data, queue, materials) else { return };
    if request.prepared_generation != request.generation || state.generation == request.generation { return; }
    if !request.prepared.is_empty() && scene.tree.is_none() {
            scene.tree = Some(scene_data.world.spatial_query());
            state.owns_scene_geometry = true;
            let mut ends = [0; 2];
            for instance in &queue.instances {
                if let Some(super::RenderInstanceSource { surface: RenderModelSurface::World { object, .. }, .. }) = instance.source {
                    if let Some(Ok(material)) = materials.0.get(&instance.material) {
                        let class = usize::from(material.source.main_blend_state & 1 != 0);
                        ends[class] = ends[class].max(scene_data.world.objects[object].matrix_end as u32);
                    }
                }
            }
            scene.receiver_range_ends = ends;
    }
    let wanted: std::collections::HashSet<_> = request.prepared.iter().map(|item| (item.name.clone(), item.world)).collect();
    let obsolete: Vec<_> = state.owned.iter().filter(|(key, current)| {
        !wanted.contains(*key) || request.prepared.iter().find(|item| item.name == key.0 && item.world == key.1)
            .is_some_and(|item| item.handle.id() != current._prepared.handle.id()
                || !Arc::ptr_eq(&item.program, &current._prepared.program))
    }).map(|(key, _)| key.clone()).collect();
    for key in obsolete {
        if let Some(old) = state.owned.remove(&key) {
            for entity in old.entities { commands.entity(entity).try_despawn(); }
            if let Some(variants) = Arc::make_mut(&mut scene.programs).get_mut(&key.0) {
                variants[usize::from(key.1)] = None;
            }
        }
    }
    Arc::make_mut(&mut scene.programs).retain(|_, variants| variants.iter().any(Option::is_some));
    if request.prepared.is_empty() && state.owned.is_empty() && state.owns_scene_geometry {
        scene.tree = None;
        scene.receiver_range_ends = [0; 2];
        if let Some(cube) = state.cube.take() { meshes.remove(cube.id()); }
        state.owns_scene_geometry = false;
    }
    let mut expected = Vec::new();
    for item in request.prepared.iter() {
        let key = (item.name.clone(), item.world);
        if let Some(current) = state.owned.get(&key) {
            expected.extend(current.entities.iter().copied());
            continue;
        }
        if scene.programs.get(&item.name).and_then(|variants| variants[usize::from(item.world)].as_ref()).is_some() {
            expected.extend(existing.iter().filter(|(_, proxy)| proxy.material == item.name && proxy.world == item.world)
                .map(|(entity, _)| entity));
            continue;
        }
        let depth_box = item.program.kind == crate::rendering::decals::material::ProjectionKind::DepthBox;
        if depth_box && state.cube.is_none() {
            let mut cube = Cuboid::new(2., 1., 2.).mesh().build().translated_by(Vec3::new(0., -0.5, 0.));
            cube.generate_tangents().expect("complete cube tangent inputs");
            state.cube = Some(meshes.add(cube));
        }
        let candidates: Vec<_> = if depth_box {
            vec![(u32::MAX, state.cube.as_ref().unwrap().clone(), false)]
        } else {
            layouts.samples.iter().filter(|(_, world, _, _)| *world == item.world)
                .map(|(layout, _, mesh, skinned)| (*layout, mesh.clone(), *skinned)).collect()
        };
        let mut entities = Vec::with_capacity(candidates.len());
        for (layout, mesh, skinned) in candidates {
            let mut entity = commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(item.handle.clone()),
                Transform::IDENTITY, GlobalTransform::IDENTITY, Visibility::Visible,
                bevy::camera::visibility::NoFrustumCulling, bevy::render::batching::NoAutomaticBatching,
                bevy::light::NotShadowCaster, ProjectedDecalProxy { material: item.name.clone(), world: item.world,
                    program: item.program.clone(), layout, mesh: mesh.id() }));
            if skinned { entity.insert(bevy::pbr::DirectMeshSkin); }
            entities.push(entity.id());
        }
        Arc::make_mut(&mut scene.programs).entry(item.name.clone()).or_insert_with(|| [None, None])
            [usize::from(item.world)] = Some(item.program.clone());
        expected.extend(entities.iter().copied());
        state.owned.insert(key, RuntimeDecalProxySet { _prepared: item.clone(), entities });
    }
    readiness.decals = crate::rendering::bevy_backend::projected_decals::ProxyReadinessBatch {
        generation: request.generation, proxies: Arc::new(expected), completed: request.gpu_completed.clone(),
    };
    state.generation = request.generation;
}

fn extract_receivers(
    mut receivers: ResMut<DecalReceivers>,
    queue: Extract<Option<Res<SceneRenderQueue>>>,
    selection: Extract<Option<Res<super::scene_queue::VisibleSceneInstances>>>,
    materials: Extract<Option<Res<ShadowMaterials>>>,
    layouts: Extract<Res<GeometryLayouts>>,
    decal_scene: Extract<Res<DecalScene>>,
    world_sources: Extract<Option<Res<super::WorldShadowMeshSources>>>,
    transforms: Extract<Query<&GlobalTransform>>,
    model_bounds: Extract<Option<Res<super::ShadowModelBoundsSources>>>,
    mut bounds_owners: Local<BoundsOwners>,
) {
    receivers.0.clear();
    if layouts.ids.is_empty() || decal_scene.tree.is_none() {
        return;
    }
    let (Some(queue), Some(materials)) = (queue.as_ref(), materials.as_ref()) else {
        return;
    };
    if queue.is_added() { *bounds_owners = Default::default(); }
    if let Some(sources) = model_bounds.as_ref() {
        let domain = (sources.0.as_ptr() as usize, sources.0.len());
        if bounds_owners.domain != domain {
            bounds_owners.domain = domain;
            bounds_owners.placements.clear();
            bounds_owners.placements.extend(
                sources
                    .0
                    .iter()
                    .enumerate()
                    .map(|(index, source)| (source.placement, index)),
            );
        }
    }
    super::scene_queue::for_each_visible_instance(queue, selection.as_deref(), |instance| {
        let (Some(entity), Some(source), Some(&layout)) = (
            instance.entity,
            instance.source,
            layouts.ids.get(&instance.mesh.id()),
        ) else {
            return;
        };
        let Some(Ok(material)) = materials.0.get(&instance.material) else {
            return;
        };
        let Ok(transform) = transforms.get(entity) else {
            return;
        };
        let model = transform.to_matrix();
        let world_instance = match source.surface {
            RenderModelSurface::World { matrix, .. } => Some(matrix as u32),
            _ => None,
        };
        // Placed models use their owner's prepared world bounds (25da70 /
        // 330c10), including animation/physics updates. RenderInstance.bounds
        // is already in world/view coordinates: applying model again moves its
        // admission box away from the real receiver.
        let placement = match source.surface {
            RenderModelSurface::Bmodel { placement, .. }
            | RenderModelSurface::Bmd6 { placement, .. } => Some(placement),
            _ => None,
        };
        let bounds = placement
            .and_then(|placement| bounds_owners.placements.get(&placement))
            .and_then(|&index| model_bounds.as_ref()?.0.get(index))
            .map(|source| source.bounds)
            .unwrap_or_else(|| {
                let centre = Vec3::from(instance.bounds.center);
                let extent = Vec3::from(instance.bounds.half_extents);
                let to_game = |p: Vec3| [p.x, -p.z, p.y];
                let low = to_game(centre - extent);
                let high = to_game(centre + extent);
                [
                    std::array::from_fn(|axis| low[axis].min(high[axis])),
                    std::array::from_fn(|axis| low[axis].max(high[axis])),
                ]
            });
        receivers.0.push(Receiver {
            entity: MainEntity::from(entity),
            mesh: instance.mesh.id(),
            model,
            probe: instance.probe_index,
            world_instance,
            opaque: material.source.main_blend_state & 1 == 0,
            flags48: match source.surface {
                RenderModelSurface::World { object, .. } => world_sources
                    .as_ref()
                    .and_then(|sources| sources.objects.get(object))
                    .and_then(|source| source.gather_flags48.as_ref())
                    .and_then(|flags| flags.as_ref().ok())
                    .copied()
                    .unwrap_or(0),
                _ => 0,
            },
            bounds,
            layout,
        });
    });
}
