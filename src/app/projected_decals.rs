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
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

type Images = Vec<(MaterialHandle, Vec<(usize, crate::surface_gpu::ImageRole)>)>;
#[derive(Resource, Default)]
struct GeometryLayouts(HashMap<AssetId<Mesh>, u32>);

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

pub(super) fn prepare(
    sources: &crate::rendering::particles::world_sources::WorldFxSources,
    world: &crate::world_mesh::WorldMesh,
    instances: &[RenderInstance],
    resolver: &mut MaterialResolver,
    meshes: &mut Assets<Mesh>,
    commands: &mut Commands,
) -> Images {
    let mut names = BTreeSet::new();
    for decal in sources
        .effects
        .values()
        .filter_map(|effect| effect.as_ref().ok())
        .flat_map(|effect| &effect.groups)
        .flat_map(|group| &group.decals)
        .filter_map(|decal| decal.as_ref().ok())
    {
        for references in decal.declarations.iter().flatten() {
            for (name, instanced) in [
                (&references.opaque, false),
                (&references.opaque_instanced, true),
                (&references.alpha, false),
                (&references.alpha_instanced, true),
            ] {
                if let Some(name) = name {
                    names.insert((name.clone(), instanced));
                }
            }
        }
    }
    if names.is_empty() {
        commands.insert_resource(DecalScene::default());
        commands.insert_resource(GeometryLayouts::default());
        return Vec::new();
    }
    let mut keys = HashMap::new();
    let mut layouts = GeometryLayouts::default();
    let mut samples = Vec::new();
    for instance in instances {
        let Some(source) = instance.source else {
            continue;
        };
        if layouts.0.contains_key(&instance.mesh.id()) {
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
        layouts.0.insert(instance.mesh.id(), id);
        if !samples
            .iter()
            .any(|&(layout, world, _): &(u32, bool, Handle<Mesh>)| {
                layout == id && world == is_world
            })
        {
            samples.push((id, is_world, instance.mesh.clone()));
        }
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
            let proxy = |commands: &mut Commands, mesh: Handle<Mesh>, layout: u32| {
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
                for (layout, is_world, mesh) in &samples {
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

fn extract_receivers(
    mut receivers: ResMut<DecalReceivers>,
    queue: Extract<Option<Res<SceneRenderQueue>>>,
    selection: Extract<Option<Res<super::scene_queue::VisibleSceneInstances>>>,
    materials: Extract<Option<Res<ShadowMaterials>>>,
    layouts: Extract<Res<GeometryLayouts>>,
    world_sources: Extract<Option<Res<super::WorldShadowMeshSources>>>,
    transforms: Extract<Query<&GlobalTransform>>,
    model_bounds: Extract<Option<Res<super::ShadowModelBoundsSources>>>,
    mut bounds_owners: Local<BoundsOwners>,
) {
    receivers.0.clear();
    if layouts.0.is_empty() {
        return;
    }
    let (Some(queue), Some(materials)) = (queue.as_ref(), materials.as_ref()) else {
        return;
    };
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
            layouts.0.get(&instance.mesh.id()),
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
