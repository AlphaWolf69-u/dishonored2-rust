use super::*;
use std::collections::HashMap;
use crate::{material, render_program, shader_cache};
use std::sync::Arc;
#[path = "../../../app/material_loading/water.rs"]
mod selector;

fn installed_material() -> Arc<super::super::water_scene::WaterMaterialSource> {
    let base = crate::assets::installation::game_base().unwrap();
    let catalog = crate::archive::Catalog::open(&base).unwrap();
    let mut library =
        shader_cache::ProgramLibrary::open(&base, shader_cache::ShaderCacheBackend::Pc, &catalog)
            .unwrap();
    for row in include_str!("../../../../tests/fixtures/water_world_colour_stages.txt").lines() {
        let path = row.split_whitespace().next().unwrap();
        let bytes = catalog.read(path).unwrap().unwrap();
        let main = crate::material_surface::Surface::parse(&bytes)
            .main_program
            .unwrap();
        let values = material::material_option_parameters(&bytes, |name| {
            library.parameter_kinds.get(name.as_bytes())
                == Some(&render_program::RenderParameterKind::Option)
        })
        .unwrap();
        let options = render_program::SerializedParameterBlock {
            flags: 0,
            parameters: values
                .into_iter()
                .map(|(name, value)| render_program::SerializedParameter {
                    name: name.into_bytes(),
                    kind: render_program::RenderParameterKind::Option,
                    value: render_program::SerializedParameterValue::Words([value as u32, 0, 0, 0]),
                })
                .collect(),
        };
        let base = library
            .definition(main.as_bytes())
            .unwrap()
            .unwrap()
            .render_state();
        let state = crate::render_state::material_render_state(
            &crate::entities::declaration_text(&bytes),
            base,
        )
        .unwrap();
        let depth = selector::depth_sources(&mut library, &bytes, &main, &options, state);
        if !depth.iter().all(|s| matches!(s, Ok(Some(_)))) {
            continue;
        }
        return Arc::new(super::super::water_scene::WaterMaterialSource {
            main_program: main.clone(),
            state,
            options,
            parameters: selector::colour_parameters(&mut library, &bytes, &main).unwrap(),
            depth,
            declaration: bytes.into(),
            world_colour: Ok(None),
        });
    }
    panic!("no installed material supplies both placed-water depth banks");
}

#[test]
#[ignore = "requires installed water programs and a graphics adapter; no window"]
fn placed_depth_submission_uses_source_geometry_and_retires_draws() {
    use bevy::{
        asset::RenderAssetUsages,
        camera::RenderTarget,
        render::{storage::ShaderBuffer, RenderApp},
    };
    let material = installed_material();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(bevy::window::WindowPlugin {
                primary_window: None,
                exit_condition: bevy::window::ExitCondition::DontExit,
                ..default()
            })
            .set(bevy::render::RenderPlugin {
                render_creation: bevy::render::settings::RenderCreation::Automatic(Box::new(
                    bevy::render::settings::WgpuSettings {
                        features: bevy::render::settings::WgpuSettings::default().features
                            | bevy::render::settings::WgpuFeatures::DEPTH32FLOAT_STENCIL8,
                        ..default()
                    },
                )),
                ..default()
            })
            .disable::<bevy::winit::WinitPlugin>()
            .disable::<bevy::render::pipelined_rendering::PipelinedRenderingPlugin>(),
    );
    app.add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
        super::super::water_scene::WaterWorldMeshes,
    >::default());
    app.finish();
    app.cleanup();
    let target = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            32,
            32,
            TextureFormat::Rgba8UnormSrgb,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        Camera::default(),
        RenderTarget::Image(target.into()),
        Transform::IDENTITY,
    ));
    let mut positions = vec![
        [-1f32, -1., 0.75],
        [-1., 1., 0.75],
        [1., -1., 0.75],
        [-1., 1., 0.75],
        [1., 1., 0.75],
        [1., -1., 0.75],
    ];
    let mut back = positions.clone();
    back.reverse();
    positions.extend(back);
    let raw_mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[99f32; 3]; positions.len()])
        .with_inserted_attribute(super::super::shadow_pass::VEGETATION_POSITION, positions),
    );
    let matrices = app
        .world_mut()
        .resource_mut::<Assets<ShaderBuffer>>()
        .add(ShaderBuffer::from(vec![[
            1f32, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., -0.25,
        ]]));
    let world = WaterWorldMeshes {
        objects: Arc::new(HashMap::from([(
            7,
            Ok(super::super::water_scene::WaterWorldMesh {
                mesh: raw_mesh.clone(),
                surface_flags: 0,
                indices_reversed: false,
            }),
        )])),
        matrices: Some(matrices),
    };
    app.insert_resource(world.clone());
    for _ in 0..4 {
        app.update();
    }
    let render = app.sub_app_mut(RenderApp).world_mut();
    render.init_resource::<TlfPipelines>();
    let device = render.resource::<RenderDevice>();
    let queue = render.resource::<RenderQueue>();
    let pipelines = render.resource::<TlfPipelines>();
    let errors = Arc::new(std::sync::Mutex::new(Vec::new()));
    let caught = errors.clone();
    device.wgpu_device().on_uncaptured_error(Arc::new(move |e| {
        caught.lock().unwrap().push(e.to_string())
    }));
    let extents = FogExtents::new(UVec2::splat(32), 2).unwrap();
    let mut owner = WaterDepthScene::new(
        device,
        pipelines,
        extents,
        TextureFormat::Depth32FloatStencil8,
    )
    .unwrap();
    let opaque = device.create_texture(&TextureDescriptor {
        label: None,
        size: Extent3d {
            width: 32,
            height: 32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Depth32Float,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let opaque_view = opaque.create_view(&default());
    let name: Arc<str> = "test/installed-water".into();
    let materials = WaterMaterials(Arc::new(HashMap::from([(name.clone(), Ok(material))])));
    let frame = crate::rendering::view_history::ViewMatrixFrame {
        frame: 1,
        current: crate::rendering::view_history::ViewMatrices::default(),
        previous: crate::rendering::view_history::ViewMatrices::default(),
    };
    let output = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 16,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&BufferDescriptor {
        label: None,
        size: 16,
        usage: BufferUsages::COPY_DST | BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let module = device.create_and_validate_shader_module(ShaderModuleDescriptor { label:None,source:ShaderSource::Wgsl(r#"
@group(0) @binding(0) var water:texture_depth_2d;
@group(0) @binding(1) var alpha:texture_depth_2d;
@group(0) @binding(2) var reduced:texture_depth_2d;
@group(0) @binding(3) var opaque:texture_depth_2d;
@group(0) @binding(4) var<storage,read_write> result:array<vec4<f32>>;
@compute @workgroup_size(1) fn read() {
 result[0]=vec4(textureLoad(water,vec2(16,16),0),textureLoad(alpha,vec2(16,16),0),textureLoad(reduced,vec2(8,8),0),textureLoad(opaque,vec2(16,16),0));
}
"#.into()) });
    let read = device.create_compute_pipeline(&RawComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &module,
        entry_point: Some("read"),
        compilation_options: default(),
        cache: None,
    });
    let mut entries: Vec<_> = [
        &owner.water.sample,
        &owner.alpha.full_sample,
        &owner.alpha.reduced_sample,
        &opaque_view,
    ]
    .into_iter()
    .enumerate()
    .map(|(i, v)| BindGroupEntry {
        binding: i as u32,
        resource: BindingResource::TextureView(v),
    })
    .collect();
    entries.push(BindGroupEntry {
        binding: 4,
        resource: output.as_entire_binding(),
    });
    let binding = device.create_bind_group(None, &read.get_bind_group_layout(0).into(), &entries);
    for (geometry, expected) in [
        (
            Some(WaterGeometry::World {
                object: 7,
                matrix: 0,
            }),
            0.5f32,
        ),
        (Some(WaterGeometry::Model), 0.75),
        (None, 0.1),
    ] {
        let instance = WaterInstance {
            material: name.clone(),
            mesh: raw_mesh.id(),
            order: 0,
            geometry,
            world_from_local: Mat4::IDENTITY,
        };
        let instances = if geometry.is_some() {
            vec![&instance]
        } else {
            Vec::new()
        };
        owner
            .prepare(
                device,
                queue,
                instances,
                &materials,
                &world,
                render.resource::<RenderAssets<RenderMesh>>(),
                render.resource::<MeshAllocator>(),
                render.resource::<RenderAssets<GpuShaderBuffer>>(),
                frame,
            )
            .unwrap();
        let mut encoder = device.create_command_encoder(&default());
        {
            let _pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: None,
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &opaque_view,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(0.1),
                        store: StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..default()
            });
        }
        owner.record(device, &mut encoder, &opaque).unwrap();
        {
            let mut pass = encoder.begin_compute_pass(&default());
            pass.set_pipeline(&read);
            pass.set_bind_group(0, &binding, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }
        encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, 16);
        queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        staging
            .slice(..)
            .map_async(MapMode::Read, move |r| tx.send(r).unwrap());
        device
            .poll(PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let bytes = staging.slice(..).get_mapped_range();
        let values: Vec<_> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        for actual in &values[..3] {
            assert!(
                (actual - expected).abs() < 1e-6,
                "{geometry:?}: {values:?} != {expected}"
            );
        }
        assert!(
            (values[3] - 0.1).abs() < 1e-6,
            "source opaque depth changed"
        );
        drop(bytes);
        staging.unmap();
    }
    super::super::alpha_depth::tests::check_preparation(device, queue, pipelines);
    assert!(
        errors.lock().unwrap().is_empty(),
        "{:?}",
        errors.lock().unwrap()
    );
}
