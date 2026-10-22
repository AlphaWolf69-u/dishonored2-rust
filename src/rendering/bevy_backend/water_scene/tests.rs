use super::*;
use crate::render_program;
use bevy::{
    asset::RenderAssetUsages,
    camera::RenderTarget,
    render::{
        mesh::{allocator::MeshAllocator, RenderMesh},
        render_asset::RenderAssets,
        RenderApp,
    },
};

#[test]
fn water_scene_extracts_visible_shared_meshes_and_material_identity() {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );
    app.add_plugins(WaterPlugin);
    app.finish();
    app.cleanup();
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            32,
            32,
            bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        Camera::default(),
        RenderTarget::Image(target.into()),
        Transform::IDENTITY,
    ));
    let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
        Mesh::new(
            bevy::render::render_resource::PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[-1f32, -1., 0.], [1., -1., 0.], [0., 1., 0.]],
        ),
    );
    let name: std::sync::Arc<str> = "test/water.material".into();
    let material = std::sync::Arc::new(WaterMaterialSource {
        main_program: "arkWater".into(),
        state: [0; 9],
        options: render_program::SerializedParameterBlock {
            flags: 0,
            parameters: Vec::new(),
        },
        declaration: b"{}".as_slice().into(),
        parameters: std::array::from_fn(|_| render_program::SerializedParameterBlock {
            flags: 0,
            parameters: Vec::new(),
        }),
        depth: [Ok(None), Ok(None)],
        world_colour: Ok(None),
    });
    std::sync::Arc::make_mut(&mut app.world_mut().resource_mut::<WaterMaterials>().0)
        .insert(name.clone(), Ok(material.clone()));
    let object = app
        .world_mut()
        .spawn((
            Mesh3d(mesh.clone()),
            WaterMesh {
                material: name.clone(),
                order: 17,
                geometry: Some(WaterGeometry::World {
                    object: 7,
                    matrix: 19,
                }),
            },
            Transform::from_xyz(0., 0., -2.),
            Visibility::Visible,
        ))
        .id();
    use bevy::render::{
        render_resource::*,
        storage::{GpuShaderBuffer, ShaderBuffer},
        texture::GpuImage,
    };
    let (references, info, indices, planes) = {
        let mut buffers = app.world_mut().resource_mut::<Assets<ShaderBuffer>>();
        (
            buffers.add(ShaderBuffer::from(vec![0u32; 8])),
            buffers.add(ShaderBuffer::from(vec![[0u32, 1f32.to_bits()]])),
            buffers.add(ShaderBuffer::from(vec![0u32])),
            buffers.add(ShaderBuffer::from(vec![0u32; 20])),
        )
    };
    let mut cube = Image::new_target_texture(4, 4, TextureFormat::Rgba8Unorm, None);
    cube.resize(Extent3d {
        width: 4,
        height: 4,
        depth_or_array_layers: 6,
    });
    cube.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::CubeArray),
        ..default()
    });
    let cubes = app.world_mut().resource_mut::<Assets<Image>>().add(cube);
    // Host-baked forward geometry and the raw local source must retain distinct
    // identities. A WORLD draw must use the latter with its original matrix ID.
    let raw_mesh = {
        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        let mut raw = meshes.get(&mesh).unwrap().clone();
        raw.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[10.0, 20.0, 30.0], [11.0, 20.0, 30.0], [10.0, 21.0, 30.0]],
        );
        meshes.add(raw)
    };
    let matrices = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(vec![[1f32; 12]; 20]));
    let world_objects = std::sync::Arc::new(std::collections::HashMap::from([
        (
            7,
            Ok(WaterWorldMesh {
                mesh: raw_mesh.clone(),
                surface_flags: 0,
                indices_reversed: true,
            }),
        ),
        (
            8,
            Ok(WaterWorldMesh {
                mesh: raw_mesh.clone(),
                surface_flags: 1,
                indices_reversed: false,
            }),
        ),
        (9, Err("test invalid source channel".into())),
    ]));
    app.insert_resource(WaterWorldMeshes {
        objects: world_objects.clone(),
        matrices: Some(matrices.clone()),
    });
    app.insert_resource(WaterLighting(Ok(Some(WaterProbeResources {
        vertex_references: references.clone(),
        probe_info: info.clone(),
        cube_indices: indices.clone(),
        cubes: cubes.clone(),
        planes: planes.clone(),
        primary_count: 1,
        cube_count: 1,
    }))));
    for step in 0..4 {
        let position = Vec3::new(step as f32 * 0.1, 0., -2.);
        app.world_mut().entity_mut(object).insert((
            Transform::from_translation(position),
            if step == 2 {
                Visibility::Hidden
            } else {
                Visibility::Visible
            },
        ));
        for _ in 0..4 {
            app.update();
        }
        let render = app.sub_app_mut(RenderApp).world_mut();
        let instances = render
            .query::<&WaterInstance>()
            .iter(render)
            .collect::<Vec<_>>();
        assert_eq!(instances.len(), usize::from(step != 2), "step{step}");
        if let Some(instance) = instances.first() {
            assert_eq!(instance.mesh, mesh.id());
            assert_eq!(instance.material, name);
            assert_eq!(instance.order, 17);
            assert_eq!(
                instance.geometry,
                Some(WaterGeometry::World {
                    object: 7,
                    matrix: 19
                })
            );
            assert_eq!(instance.world_from_local, Mat4::from_translation(position));
        }
        assert!(render
            .resource::<RenderAssets<RenderMesh>>()
            .get(&mesh)
            .is_some());
        assert!(render
            .resource::<MeshAllocator>()
            .mesh_vertex_slice(&mesh.id())
            .is_some());
        let retained = render
            .resource::<WaterMaterials>()
            .0
            .get(&name)
            .unwrap()
            .as_ref()
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(retained, &material));
        let world = render.resource::<WaterWorldMeshes>();
        assert!(std::sync::Arc::ptr_eq(&world.objects, &world_objects));
        let source = world.objects[&7].as_ref().unwrap();
        assert_eq!(source.mesh.id(), raw_mesh.id());
        assert_ne!(source.mesh.id(), mesh.id());
        assert_eq!(source.surface_flags, 0);
        assert!(source.indices_reversed);
        let other = world.objects[&8].as_ref().unwrap();
        assert_eq!(other.mesh.id(), source.mesh.id());
        assert_eq!(other.surface_flags, 1);
        assert!(!other.indices_reversed);
        assert!(world.objects[&9].is_err());
        assert_eq!(world.matrices.as_ref().unwrap().id(), matrices.id());
        assert!(render
            .resource::<RenderAssets<RenderMesh>>()
            .get(&raw_mesh)
            .is_some());
        assert!(render
            .resource::<MeshAllocator>()
            .mesh_vertex_slice(&raw_mesh.id())
            .is_some());
        let lighting = render
            .resource::<WaterLighting>()
            .0
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap();
        assert_eq!(lighting.vertex_references.id(), references.id());
        assert_eq!(lighting.probe_info.id(), info.id());
        assert_eq!(lighting.cube_indices.id(), indices.id());
        assert_eq!(lighting.planes.id(), planes.id());
        assert_eq!(lighting.cubes.id(), cubes.id());
        assert_eq!((lighting.primary_count, lighting.cube_count), (1, 1));
        let buffers = render.resource::<RenderAssets<GpuShaderBuffer>>();
        for handle in [&references, &info, &indices, &planes, &matrices] {
            assert!(buffers.get(handle).is_some());
        }
        let cube = render
            .resource::<RenderAssets<GpuImage>>()
            .get(&cubes)
            .unwrap();
        assert_eq!(cube.texture.depth_or_array_layers(), 6);
    }
    app.insert_resource(WaterLighting(Err("test unresolved probe input".into())));
    for _ in 0..4 {
        app.update();
    }
    assert!(
        matches!(&app.sub_app(RenderApp).world().resource::<WaterLighting>().0,Err(error) if error=="test unresolved probe input")
    );
    app.insert_resource(WaterLighting(Ok(None)));
    app.insert_resource(WaterWorldMeshes::default());
    for _ in 0..4 {
        app.update();
    }
    assert!(matches!(
        &app.sub_app(RenderApp).world().resource::<WaterLighting>().0,
        Ok(None)
    ));
    let cleared = app
        .sub_app(RenderApp)
        .world()
        .resource::<WaterWorldMeshes>();
    assert!(cleared.objects.is_empty());
    assert!(cleared.matrices.is_none());
}
